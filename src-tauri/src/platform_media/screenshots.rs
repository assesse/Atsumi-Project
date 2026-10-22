//! Local live-frame captures. No provider account, remote URL, or CHZZK state.
use super::{chat_status, clean, storage, MediaRef, PlatformMediaService};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const ERROR: &str = "스크린샷 파일을 안전하게 처리하지 못했습니다.";
const MAX_BYTES: usize = 16 * 1024 * 1024;
const CHUNK: usize = 128 * 1024;
const TTL: Duration = Duration::from_secs(20);
const UNDO_TTL: Duration = Duration::from_secs(60);
const MAX_RECORD: u64 = 16 * 1024;
const MAX_CATALOG: usize = 10_000;

#[derive(Clone)]
pub(super) struct CaptureContext {
    pub(super) download_root: PathBuf,
    pub(super) reference: MediaRef,
    pub(super) title: String,
    pub(super) channel_name: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Item {
    pub id: String,
    pub provider: String,
    pub reference: MediaRef,
    pub title: String,
    pub channel_name: String,
    pub file_name: String,
    pub created_at: u64,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Begin {
    pub capture_id: String,
}
#[derive(Serialize)]
pub struct Page {
    pub items: Vec<Item>,
    pub cursor: Option<String>,
}
#[derive(Serialize)]
pub struct DeleteFailure {
    pub id: String,
    pub error: String,
}
#[derive(Serialize)]
pub struct DeleteResult {
    pub deleted: Vec<String>,
    pub failed: Vec<DeleteFailure>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    volume: u64,
    index: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    version: u8,
    item: Item,
    download_root: PathBuf,
    sha256: String,
    identity: FileIdentity,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Deleted {
    version: u8,
    id: String,
    sha256: String,
}
struct Pending {
    token: String,
    request_id: String,
    context: CaptureContext,
    size: usize,
    width: u32,
    height: u32,
    next: usize,
    bytes: Vec<u8>,
    expires: Instant,
}
struct Receipt {
    token: String,
    request_id: String,
    item: Item,
    sha256: String,
    expires: Instant,
}
#[derive(Default)]
struct TransferState {
    closing: bool,
    pending: HashMap<String, Pending>,
    receipts: HashMap<String, Receipt>,
    operations: HashMap<String, (String, Arc<AtomicBool>)>,
    admitted: HashSet<String>,
}
pub(super) struct Store {
    journal: PathBuf,
    transfers: Mutex<TransferState>,
    // File mutation and raster decoding are serialized; active_ids never takes this lock.
    catalog: Mutex<Option<HashMap<String, Record>>>,
}
struct Operation<'a> {
    store: &'a Store,
    id: String,
    cancel: Arc<AtomicBool>,
}
pub struct Work {
    service: PlatformMediaService,
    id: String,
}
impl Drop for Work {
    fn drop(&mut self) {
        self.service
            .inner
            .screenshots
            .transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .admitted
            .remove(&self.id);
    }
}
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        let mut state = self
            .store
            .transfers
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if state
            .operations
            .get(&self.id)
            .is_some_and(|(_, c)| Arc::ptr_eq(c, &self.cancel))
        {
            state.operations.remove(&self.id);
        }
    }
}
impl Store {
    pub(super) fn new(journal: PathBuf) -> Self {
        Self {
            journal,
            transfers: Mutex::new(TransferState::default()),
            catalog: Mutex::new(None),
        }
    }
    fn expire(state: &mut TransferState) {
        let now = Instant::now();
        state.pending.retain(|_, v| v.expires > now);
        state.receipts.retain(|_, v| v.expires > now);
    }
    pub(super) fn active_ids(&self) -> Vec<String> {
        let mut state = self.transfers.lock().unwrap_or_else(|e| e.into_inner());
        Self::expire(&mut state);
        state
            .pending
            .keys()
            .chain(state.operations.keys())
            .chain(state.admitted.iter())
            .map(|id| format!("platform-screenshot:{id}"))
            .collect()
    }
    pub(super) fn shutdown(&self) {
        let mut state = self.transfers.lock().unwrap_or_else(|e| e.into_inner());
        state.closing = true;
        state.pending.clear();
        for (_, cancel) in state.operations.values() {
            cancel.store(true, Ordering::Release);
        }
    }
    /// Native hosts already hold the complete bounded frame. Keep the same
    /// transfer/publication path without trusting dimensions from page script.
    /// The lease serializes calls for its private, non-page-visible owner token.
    pub(super) fn save_native(
        &self,
        token: &str,
        request_id: &str,
        context: CaptureContext,
        png: &[u8],
        check_live: impl Fn() -> Result<(), String>,
    ) -> Result<Item, String> {
        check_live()?;
        if png.is_empty() || png.len() > MAX_BYTES || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err("PNG 스크린샷 크기 또는 형식이 올바르지 않습니다.".into());
        }
        let (width, height) =
            image::ImageReader::with_format(Cursor::new(png), image::ImageFormat::Png)
                .into_dimensions()
                .map_err(|_| "스크린샷 이미지가 손상됐습니다.")?;
        dimensions(width, height)?;
        let id = self
            .begin(token, request_id, context, png.len(), width, height)?
            .capture_id;
        {
            let state = self.transfers.lock().map_err(|_| ERROR)?;
            if let Some(receipt) = state.receipts.get(&id).filter(|r| r.token == token) {
                if receipt.sha256 != digest(png) {
                    return Err("이미 사용한 캡처 요청입니다.".into());
                }
                return Ok(receipt.item.clone());
            }
        }
        let result = (|| {
            for (index, chunk) in png.chunks(CHUNK).enumerate() {
                self.chunk(token, &id, index, &STANDARD.encode(chunk))?;
            }
            self.finish(token, &id, check_live)
        })();
        if result.is_err() {
            let _ = self.abort(token, &id);
        }
        result
    }
    fn begin(
        &self,
        token: &str,
        request_id: &str,
        context: CaptureContext,
        size: usize,
        width: u32,
        height: u32,
    ) -> Result<Begin, String> {
        uuid(token)?;
        uuid(request_id)?;
        dimensions(width, height)?;
        if size == 0 || size > MAX_BYTES {
            return Err("스크린샷 크기 한도를 넘었습니다.".into());
        }
        context.reference.canonical_url()?;
        if context.reference.kind != "live" {
            return Err("라이브 화면만 캡처할 수 있습니다.".into());
        }
        storage::safe_path(&context.download_root)?;
        let mut state = self.transfers.lock().map_err(|_| ERROR)?;
        Self::expire(&mut state);
        if state.closing {
            return Err("앱을 종료하는 중입니다.".into());
        }
        if let Some((id, old)) = state
            .pending
            .iter()
            .find(|(_, p)| p.token == token && p.request_id == request_id)
        {
            if old.size != size
                || old.width != width
                || old.height != height
                || old.context.reference != context.reference
            {
                return Err("이미 사용한 캡처 요청입니다.".into());
            }
            return Ok(Begin {
                capture_id: id.clone(),
            });
        }
        if let Some((id, old)) = state
            .receipts
            .iter()
            .find(|(_, p)| p.token == token && p.request_id == request_id)
        {
            if old.item.bytes != size as u64
                || old.item.width != width
                || old.item.height != height
                || old.item.reference != context.reference
            {
                return Err("이미 사용한 캡처 요청입니다.".into());
            }
            return Ok(Begin {
                capture_id: id.clone(),
            });
        }
        if state.pending.len() + state.operations.len() >= 4 || state.receipts.len() >= 256 {
            return Err("스크린샷을 저장 중입니다. 잠시 후 다시 시도해 주세요.".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        state.pending.insert(
            id.clone(),
            Pending {
                token: token.into(),
                request_id: request_id.into(),
                context,
                size,
                width,
                height,
                next: 0,
                bytes: Vec::with_capacity(size),
                expires: Instant::now() + TTL,
            },
        );
        Ok(Begin { capture_id: id })
    }
    fn chunk(&self, token: &str, id: &str, index: usize, data: &str) -> Result<(), String> {
        uuid(token)?;
        uuid(id)?;
        if data.len() > CHUNK.div_ceil(3) * 4 {
            return Err("캡처 조각이 너무 큽니다.".into());
        }
        let bytes = STANDARD
            .decode(data)
            .map_err(|_| "캡처 조각이 올바르지 않습니다.")?;
        if bytes.is_empty() || bytes.len() > CHUNK {
            return Err("캡처 조각 크기가 올바르지 않습니다.".into());
        }
        let mut state = self.transfers.lock().map_err(|_| ERROR)?;
        Self::expire(&mut state);
        let pending = state
            .pending
            .get_mut(id)
            .filter(|p| p.token == token)
            .ok_or("캡처 요청이 만료됐습니다.")?;
        if pending.next != index
            || pending
                .bytes
                .len()
                .checked_add(bytes.len())
                .is_none_or(|n| n > pending.size)
        {
            return Err("캡처 조각 순서 또는 크기가 올바르지 않습니다.".into());
        }
        pending.bytes.extend(bytes);
        pending.next += 1;
        Ok(())
    }
    fn operation(&self, id: String, token: &str) -> Result<Operation<'_>, String> {
        let mut state = self.transfers.lock().map_err(|_| ERROR)?;
        Self::expire(&mut state);
        if state.closing
            || state.operations.len() + state.pending.len() >= 4
            || state.operations.contains_key(&id)
        {
            return Err("스크린샷 작업을 지금 시작할 수 없습니다.".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        state
            .operations
            .insert(id.clone(), (token.into(), cancel.clone()));
        Ok(Operation {
            store: self,
            id,
            cancel,
        })
    }
    fn finish(
        &self,
        token: &str,
        id: &str,
        check_live: impl FnOnce() -> Result<(), String>,
    ) -> Result<Item, String> {
        uuid(token)?;
        uuid(id)?;
        let (pending, operation) = {
            let mut state = self.transfers.lock().map_err(|_| ERROR)?;
            Self::expire(&mut state);
            if state.closing {
                return Err("앱을 종료하는 중입니다.".into());
            }
            if let Some(old) = state.receipts.get(id).filter(|p| p.token == token) {
                return Ok(old.item.clone());
            }
            let pending = state
                .pending
                .get(id)
                .filter(|p| p.token == token)
                .ok_or("캡처 요청이 만료됐습니다.")?;
            if pending.bytes.len() != pending.size {
                return Err("스크린샷 전송이 아직 끝나지 않았습니다.".into());
            }
            let pending = state.pending.remove(id).unwrap();
            let cancel = Arc::new(AtomicBool::new(false));
            state
                .operations
                .insert(id.into(), (token.into(), cancel.clone()));
            (
                pending,
                Operation {
                    store: self,
                    id: id.into(),
                    cancel,
                },
            )
        };
        let mut slot = self.catalog.lock().map_err(|_| ERROR)?;
        let catalog = self.catalog(&mut slot)?;
        if catalog.len() >= MAX_CATALOG {
            return Err("스크린샷 보관 한도에 도달했습니다. 불필요한 항목을 정리해 주세요.".into());
        }
        validate_png(&pending.bytes, pending.width, pending.height)?;
        check_live()?;
        if operation.cancel.load(Ordering::Acquire) {
            return Err("스크린샷 저장을 취소했습니다.".into());
        }
        let item = Item {
            id: id.into(),
            provider: pending.context.reference.provider.clone(),
            reference: pending.context.reference,
            title: clean(Some(&pending.context.title), 300),
            channel_name: clean(Some(&pending.context.channel_name), 160),
            file_name: format!("{id}.png"),
            created_at: millis(),
            bytes: pending.size as u64,
            width: pending.width,
            height: pending.height,
        };
        let path = image_path(&pending.context.download_root, &item)?;
        make_parent(&path)?;
        let _pins = chat_status::pin_parents(&path)?;
        let mut image = create_file(&path)?;
        image
            .write_all(&pending.bytes)
            .and_then(|_| image.sync_all())
            .map_err(|_| ERROR)?;
        let record = Record {
            version: 1,
            item: item.clone(),
            download_root: pending.context.download_root,
            sha256: digest(&pending.bytes),
            identity: file_identity(&image)?,
        };
        chat_status::checked_handle(&image, MAX_BYTES as u64)?;
        // The exact image remains locked until the immutable journal is durably published.
        publish(&self.journal, &format!("{id}.json"), &record)?;
        drop(image);
        catalog.insert(id.into(), record);
        let cancelled = {
            let mut state = self.transfers.lock().map_err(|_| ERROR)?;
            state.receipts.insert(
                id.into(),
                Receipt {
                    token: token.into(),
                    request_id: pending.request_id,
                    item: item.clone(),
                    sha256: digest(&pending.bytes),
                    expires: Instant::now() + UNDO_TTL,
                },
            );
            operation.cancel.load(Ordering::Acquire)
        };
        if cancelled {
            self.delete_one(catalog, &item.provider, id)?;
            self.transfers
                .lock()
                .map_err(|_| ERROR)?
                .receipts
                .remove(id);
            return Err("스크린샷 저장을 취소했습니다.".into());
        }
        Ok(item)
    }
    fn abort(&self, token: &str, id: &str) -> Result<(), String> {
        uuid(token)?;
        uuid(id)?;
        let saved = {
            let mut state = self.transfers.lock().map_err(|_| ERROR)?;
            Self::expire(&mut state);
            if state.pending.get(id).is_some_and(|p| p.token == token) {
                state.pending.remove(id);
            }
            if let Some((owner, cancel)) =
                state.operations.get(id).filter(|(owner, _)| owner == token)
            {
                let _ = owner;
                cancel.store(true, Ordering::Release);
            }
            state.receipts.get(id).is_some_and(|p| p.token == token)
        };
        if saved {
            self.undo(token, id)
        } else {
            Ok(())
        }
    }
    fn undo(&self, token: &str, id: &str) -> Result<(), String> {
        uuid(token)?;
        uuid(id)?;
        let provider = {
            let mut state = self.transfers.lock().map_err(|_| ERROR)?;
            Self::expire(&mut state);
            state
                .receipts
                .get(id)
                .filter(|r| r.token == token)
                .ok_or("스크린샷 취소 시간이 지났습니다.")?
                .item
                .provider
                .clone()
        };
        let operation = self.operation(uuid::Uuid::new_v4().to_string(), token)?;
        let mut slot = self.catalog.lock().map_err(|_| ERROR)?;
        if operation.cancel.load(Ordering::Acquire) {
            return Err("앱을 종료하는 중입니다.".into());
        }
        self.delete_one(self.catalog(&mut slot)?, &provider, id)?;
        self.transfers
            .lock()
            .map_err(|_| ERROR)?
            .receipts
            .remove(id);
        Ok(())
    }
    fn catalog<'a>(
        &self,
        slot: &'a mut Option<HashMap<String, Record>>,
    ) -> Result<&'a mut HashMap<String, Record>, String> {
        if slot.is_none() {
            let mut result = HashMap::new();
            if self.journal.exists() {
                storage::safe_path(&self.journal)?;
                let _pins = chat_status::pin_parents(&self.journal.join("catalog"))?;
                let mut inspected = 0usize;
                let mut total = 0u64;
                for entry in fs::read_dir(&self.journal).map_err(|_| ERROR)? {
                    let path = entry.map_err(|_| ERROR)?.path();
                    if path.extension().is_none_or(|e| e != "json") {
                        continue;
                    }
                    inspected += 1;
                    if inspected > MAX_CATALOG {
                        return Err("스크린샷 목록이 너무 큽니다.".into());
                    }
                    let name = path.file_stem().and_then(|s| s.to_str()).ok_or(ERROR)?;
                    if uuid(name).is_err() {
                        continue;
                    }
                    let record: Record = read_json(&path)?.ok_or(ERROR)?;
                    validate_record(&record, name)?;
                    total = total.saturating_add(
                        serde_json::to_vec(&record).map_err(|_| ERROR)?.len() as u64,
                    );
                    if total > 64 * 1024 * 1024 {
                        return Err("스크린샷 목록이 너무 큽니다.".into());
                    }
                    if let Some(deleted) =
                        read_json::<Deleted>(&self.journal.join(format!("{name}.deleted")))?
                    {
                        if deleted.version != 1
                            || deleted.id != name
                            || deleted.sha256 != record.sha256
                        {
                            return Err(ERROR.into());
                        }
                        continue;
                    }
                    result.insert(name.into(), record);
                }
            }
            *slot = Some(result);
        }
        Ok(slot.as_mut().unwrap())
    }
    fn list(&self, provider: &str, cursor: Option<&str>, limit: usize) -> Result<Page, String> {
        provider_ok(provider)?;
        if !(1..=100).contains(&limit) {
            return Err("목록 요청 범위가 올바르지 않습니다.".into());
        }
        let boundary = decode_cursor(provider, cursor)?;
        let mut slot = self.catalog.lock().map_err(|_| ERROR)?;
        let mut items: Vec<_> = self
            .catalog(&mut slot)?
            .values()
            .filter(|r| r.item.provider == provider)
            .filter(|r| {
                boundary.as_ref().is_none_or(|(at, id)| {
                    (r.item.created_at, r.item.id.as_str()) < (*at, id.as_str())
                })
            })
            .map(|r| &r.item)
            .collect();
        items.sort_unstable_by(|a, b| (b.created_at, &b.id).cmp(&(a.created_at, &a.id)));
        let more = items.len() > limit;
        let items: Vec<Item> = items.into_iter().take(limit).cloned().collect();
        let cursor = if more {
            items.last().map(|i| encode_cursor(provider, i))
        } else {
            None
        };
        Ok(Page { items, cursor })
    }
    fn image(&self, provider: &str, id: &str, full: bool) -> Result<String, String> {
        provider_ok(provider)?;
        uuid(id)?;
        let mut slot = self.catalog.lock().map_err(|_| ERROR)?;
        let record = self
            .catalog(&mut slot)?
            .get(id)
            .filter(|r| r.item.provider == provider)
            .ok_or("스크린샷을 찾을 수 없습니다.")?;
        let bytes = read_image(record)?;
        if full {
            return Ok(format!("data:image/png;base64,{}", STANDARD.encode(bytes)));
        }
        let decoded = validate_png(&bytes, record.item.width, record.item.height)?;
        let preview = decoded.thumbnail(480, 270).to_rgb8();
        let mut output = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 82)
            .encode_image(&preview)
            .map_err(|_| ERROR)?;
        if output.len() > 512 * 1024 {
            return Err(ERROR.into());
        }
        Ok(format!(
            "data:image/jpeg;base64,{}",
            STANDARD.encode(output)
        ))
    }
    fn delete(&self, provider: &str, ids: &[String]) -> Result<DeleteResult, String> {
        provider_ok(provider)?;
        if ids.is_empty() || ids.len() > 100 {
            return Err("한 번에 1~100개를 선택해 주세요.".into());
        }
        for id in ids {
            uuid(id)?;
        }
        let operation = self.operation(uuid::Uuid::new_v4().to_string(), "library")?;
        let mut slot = self.catalog.lock().map_err(|_| ERROR)?;
        let catalog = self.catalog(&mut slot)?;
        let mut result = DeleteResult {
            deleted: Vec::new(),
            failed: Vec::new(),
        };
        for id in ids {
            let outcome = if operation.cancel.load(Ordering::Acquire) {
                Err("앱을 종료하는 중입니다.".into())
            } else {
                self.delete_one(catalog, provider, id)
            };
            match outcome {
                Ok(()) => result.deleted.push(id.clone()),
                Err(error) => result.failed.push(DeleteFailure {
                    id: id.clone(),
                    error,
                }),
            }
        }
        Ok(result)
    }
    fn delete_one(
        &self,
        catalog: &mut HashMap<String, Record>,
        provider: &str,
        id: &str,
    ) -> Result<(), String> {
        let record = catalog
            .get(id)
            .filter(|r| r.item.provider == provider)
            .ok_or("스크린샷을 찾을 수 없습니다.")?;
        let path = image_path(&record.download_root, &record.item)?;
        delete_image(&path, record)?;
        publish(
            &self.journal,
            &format!("{id}.deleted"),
            &Deleted {
                version: 1,
                id: id.into(),
                sha256: record.sha256.clone(),
            },
        )?;
        catalog.remove(id);
        Ok(())
    }
    fn folder(&self, provider: &str, id: &str) -> Result<PathBuf, String> {
        provider_ok(provider)?;
        uuid(id)?;
        let mut slot = self.catalog.lock().map_err(|_| ERROR)?;
        let record = self
            .catalog(&mut slot)?
            .get(id)
            .filter(|r| r.item.provider == provider)
            .ok_or("스크린샷을 찾을 수 없습니다.")?;
        let path = image_path(&record.download_root, &record.item)?;
        let _pins = chat_status::pin_parents(&path)?;
        Ok(path.parent().ok_or(ERROR)?.to_path_buf())
    }
}

impl PlatformMediaService {
    /// Constant-time registration for the app's quit gate. No decoding, waiting, or I/O.
    pub fn screenshot_work(&self) -> Result<Work, String> {
        let mut state = self.inner.screenshots.transfers.lock().map_err(|_| ERROR)?;
        if state.closing || state.admitted.len() >= 16 {
            return Err("스크린샷 작업을 지금 시작할 수 없습니다.".into());
        }
        let id = uuid::Uuid::new_v4().to_string();
        state.admitted.insert(id.clone());
        Ok(Work {
            service: self.clone(),
            id,
        })
    }
    pub fn screenshot_begin(
        &self,
        token: &str,
        request_id: &str,
        size: usize,
        width: u32,
        height: u32,
    ) -> Result<Begin, String> {
        let context = self.screenshot_context(token)?;
        self.inner
            .screenshots
            .begin(token, request_id, context, size, width, height)
    }
    pub fn screenshot_chunk(
        &self,
        token: &str,
        capture_id: &str,
        index: usize,
        data: &str,
    ) -> Result<(), String> {
        self.inner.screenshots.chunk(token, capture_id, index, data)
    }
    pub fn screenshot_finish(&self, token: &str, capture_id: &str) -> Result<Item, String> {
        self.screenshot_context(token)?;
        self.inner.screenshots.finish(token, capture_id, || {
            self.screenshot_context(token).map(|_| ())
        })
    }
    pub fn screenshot_abort(&self, token: &str, capture_id: &str) -> Result<(), String> {
        self.inner.screenshots.abort(token, capture_id)
    }
    pub fn screenshot_undo(&self, token: &str, id: &str) -> Result<(), String> {
        self.inner.screenshots.undo(token, id)
    }
    pub fn screenshot_list(
        &self,
        provider: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<Page, String> {
        self.inner.screenshots.list(provider, cursor, limit)
    }
    pub fn screenshot_image(&self, provider: &str, id: &str, full: bool) -> Result<String, String> {
        self.inner.screenshots.image(provider, id, full)
    }
    pub fn screenshot_delete(
        &self,
        provider: &str,
        ids: &[String],
    ) -> Result<DeleteResult, String> {
        self.inner.screenshots.delete(provider, ids)
    }
    pub fn screenshot_folder(&self, provider: &str, id: &str) -> Result<PathBuf, String> {
        self.inner.screenshots.folder(provider, id)
    }
}

fn uuid(value: &str) -> Result<(), String> {
    if uuid::Uuid::parse_str(value).is_err() || value.len() != 36 {
        return Err("스크린샷 식별자가 올바르지 않습니다.".into());
    }
    Ok(())
}
fn provider_ok(value: &str) -> Result<(), String> {
    if matches!(value, "youtube" | "soop") {
        Ok(())
    } else {
        Err(ERROR.into())
    }
}
fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn dimensions(width: u32, height: u32) -> Result<(), String> {
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || u64::from(width) * u64::from(height) > 16_000_000
    {
        Err("스크린샷 해상도가 너무 큽니다.".into())
    } else {
        Ok(())
    }
}
fn validate_png(bytes: &[u8], width: u32, height: u32) -> Result<image::DynamicImage, String> {
    dimensions(width, height)?;
    if bytes.len() > MAX_BYTES || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("PNG 스크린샷이 아닙니다.".into());
    }
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "스크린샷 이미지가 손상됐습니다.")?;
    if image.width() != width || image.height() != height {
        return Err("스크린샷 크기가 일치하지 않습니다.".into());
    }
    Ok(image)
}
fn image_path(root: &Path, item: &Item) -> Result<PathBuf, String> {
    provider_ok(&item.provider)?;
    uuid(&item.id)?;
    if item.file_name != format!("{}.png", item.id) {
        return Err(ERROR.into());
    }
    storage::safe_path(root)?;
    let path = root
        .join(if item.provider == "youtube" {
            "YouTube"
        } else {
            "SOOP"
        })
        .join("Screenshots")
        .join(&item.file_name);
    storage::safe_path(&path)?;
    Ok(path)
}
fn validate_record(record: &Record, id: &str) -> Result<(), String> {
    if record.version != 1
        || record.item.id != id
        || record.item.provider != record.item.reference.provider
        || record.item.reference.kind != "live"
        || record.item.bytes == 0
        || record.item.bytes > MAX_BYTES as u64
        || record.sha256.len() != 64
        || !record.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || record.item.title.chars().count() > 300
        || record.item.channel_name.chars().count() > 160
    {
        return Err(ERROR.into());
    }
    record.item.reference.canonical_url()?;
    dimensions(record.item.width, record.item.height)?;
    image_path(&record.download_root, &record.item)?;
    Ok(())
}
fn make_parent(path: &Path) -> Result<(), String> {
    let parent = path.parent().ok_or(ERROR)?;
    storage::safe_path(parent)?;
    // Every existing ancestor is held against replacement during directory creation.
    let existing = parent.ancestors().find(|p| p.exists()).ok_or(ERROR)?;
    let _pins = chat_status::pin_parents(&existing.join("screenshot-anchor"))?;
    fs::create_dir_all(parent).map_err(|_| ERROR)?;
    storage::safe_path(parent)
}
fn create_file(path: &Path) -> Result<File, String> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0xC0010000)
            .share_mode(1)
            .custom_flags(0x00200000);
    }
    let file = options.open(path).map_err(|_| ERROR)?;
    chat_status::checked_handle(&file, MAX_BYTES as u64)?;
    Ok(file)
}
fn file_identity(file: &File) -> Result<FileIdentity, String> {
    chat_status::checked_handle(file, MAX_BYTES as u64)?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe {
            GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)
                .map_err(|_| ERROR)?;
        }
        Ok(FileIdentity {
            volume: u64::from(info.dwVolumeSerialNumber),
            index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let meta = file.metadata().map_err(|_| ERROR)?;
        Ok(FileIdentity {
            volume: meta.dev(),
            index: meta.ino(),
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        Err(ERROR.into())
    }
}
fn verify_reader(file: &mut File, record: &Record) -> Result<Vec<u8>, String> {
    if chat_status::checked_handle(file, MAX_BYTES as u64)? != record.item.bytes
        || file_identity(file)? != record.identity
    {
        return Err("저장된 스크린샷 파일이 변경됐습니다.".into());
    }
    let mut bytes = Vec::with_capacity(record.item.bytes as usize);
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() as u64 != record.item.bytes || digest(&bytes) != record.sha256 {
        return Err("저장된 스크린샷 파일이 변경됐습니다.".into());
    }
    Ok(bytes)
}
fn read_image(record: &Record) -> Result<Vec<u8>, String> {
    let path = image_path(&record.download_root, &record.item)?;
    let mut reader =
        chat_status::open_reader(&path, MAX_BYTES as u64)?.ok_or("스크린샷 파일이 없습니다.")?;
    verify_reader(&mut reader.file, record)
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    let Some(mut reader) = chat_status::open_reader(path, MAX_RECORD)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    (&mut reader.file)
        .take(MAX_RECORD + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ERROR)?;
    if bytes.len() as u64 > MAX_RECORD
        || chat_status::checked_handle(&reader.file, MAX_RECORD)? != bytes.len() as u64
    {
        return Err(ERROR.into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| ERROR.into())
}
fn publish(directory: &Path, name: &str, value: &impl Serialize) -> Result<(), String> {
    let target = directory.join(name);
    make_parent(&target)?;
    let _pins = chat_status::pin_parents(&target)?;
    let bytes = serde_json::to_vec(value).map_err(|_| ERROR)?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err(ERROR.into());
    }
    let temp = directory.join(format!("{}.partial", uuid::Uuid::new_v4()));
    let mut file = create_file(&temp)?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| ERROR)?;
    #[cfg(windows)]
    {
        use std::os::windows::{ffi::OsStrExt, io::AsRawHandle};
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{FileRenameInfo, SetFileInformationByHandle, FILE_RENAME_INFO},
        };
        let path: Vec<u16> = target.as_os_str().encode_wide().collect();
        if path.len() > 32767 {
            return Err(ERROR.into());
        }
        let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
        let size = offset + (path.len() + 1) * 2;
        let mut buffer = vec![0usize; size.div_ceil(std::mem::size_of::<usize>())];
        unsafe {
            let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
            (*info).FileNameLength = (path.len() * 2) as u32;
            std::ptr::copy_nonoverlapping(path.as_ptr(), (*info).FileName.as_mut_ptr(), path.len());
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileRenameInfo,
                buffer.as_ptr().cast(),
                size as u32,
            )
            .map_err(|_| ERROR)?;
        }
    }
    #[cfg(not(windows))]
    {
        fs::hard_link(&temp, &target).map_err(|_| ERROR)?;
        fs::remove_file(&temp).map_err(|_| ERROR)?;
    }
    file.sync_all().map_err(|_| ERROR)?;
    chat_status::checked_handle(&file, MAX_RECORD)?;
    Ok(())
}
fn delete_image(path: &Path, record: &Record) -> Result<(), String> {
    storage::safe_path(path)?;
    if fs::symlink_metadata(path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
        return Ok(());
    }
    let _pins = chat_status::pin_parents(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options
            .access_mode(0x80010000)
            .share_mode(1)
            .custom_flags(0x00200000);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(ERROR.into()),
    };
    verify_reader(&mut file, record)?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{
                FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
            },
        };
        let info = FILE_DISPOSITION_INFO { DeleteFile: true };
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                (&info as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
            .map_err(|_| ERROR)?;
        }
    }
    #[cfg(not(windows))]
    {
        storage::regular(path)?;
        fs::remove_file(path).map_err(|_| ERROR)?;
    }
    Ok(())
}
fn encode_cursor(provider: &str, item: &Item) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&(1u8, provider, item.created_at, &item.id)).unwrap())
}
fn decode_cursor(provider: &str, value: Option<&str>) -> Result<Option<(u64, String)>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.len() > 256 {
        return Err("스크린샷 목록 위치가 올바르지 않습니다.".into());
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| ERROR)?;
    let (version, owner, time, id): (u8, String, u64, String) =
        serde_json::from_slice(&bytes).map_err(|_| ERROR)?;
    if version != 1 || owner != provider {
        return Err(ERROR.into());
    }
    uuid(&id)?;
    Ok(Some((time, id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Store, CaptureContext, String, Vec<u8>) {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::new(directory.path().join("data/screenshots"));
        let context = CaptureContext {
            download_root: directory.path().join("downloads"),
            reference: MediaRef {
                provider: "youtube".into(),
                kind: "live".into(),
                external_id: "abcdefghijk".into(),
            },
            title: "검증된 방송".into(),
            channel_name: "채널".into(),
        };
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(3, 2)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        (
            directory,
            store,
            context,
            uuid::Uuid::new_v4().to_string(),
            bytes.into_inner(),
        )
    }
    fn transfer(store: &Store, context: CaptureContext, token: &str, bytes: &[u8]) -> String {
        let id = store
            .begin(
                token,
                &uuid::Uuid::new_v4().to_string(),
                context,
                bytes.len(),
                3,
                2,
            )
            .unwrap()
            .capture_id;
        for (index, chunk) in bytes.chunks(CHUNK).enumerate() {
            store
                .chunk(token, &id, index, &STANDARD.encode(chunk))
                .unwrap();
        }
        id
    }
    fn save(store: &Store, context: CaptureContext, token: &str, bytes: &[u8]) -> Item {
        let id = transfer(store, context, token, bytes);
        store.finish(token, &id, || Ok(())).unwrap()
    }
    #[test]
    fn native_save_reuses_validation_and_rejects_changed_request_bytes() {
        let (_dir, store, context, token, bytes) = fixture();
        let request = uuid::Uuid::new_v4().to_string();
        let item = store
            .save_native(&token, &request, context.clone(), &bytes, || Ok(()))
            .unwrap();
        assert_eq!(
            store
                .save_native(&token, &request, context.clone(), &bytes, || Ok(()))
                .unwrap()
                .id,
            item.id
        );
        let mut changed = bytes.clone();
        let last = changed.len() - 1;
        changed[last] ^= 1;
        assert!(store
            .save_native(&token, &request, context.clone(), &changed, || Ok(()))
            .is_err());
        assert!(store
            .save_native(
                &token,
                &uuid::Uuid::new_v4().to_string(),
                context.clone(),
                &bytes[..bytes.len() / 2],
                || Ok(())
            )
            .is_err());
        assert!(store
            .save_native(&token, &request, context, &bytes, || Err("closed".into()))
            .is_err());
        assert_eq!(store.list("youtube", None, 24).unwrap().items.len(), 1);
        assert!(store.active_ids().is_empty());
    }
    #[test]
    fn durable_png_reopens_without_current_download_folder_or_network() {
        let (dir, store, context, token, bytes) = fixture();
        let item = save(&store, context.clone(), &token, &bytes);
        assert!(item.created_at > 1_000_000_000_000);
        assert!(store.active_ids().is_empty());
        assert_eq!(
            store.image("youtube", &item.id, true).unwrap(),
            format!("data:image/png;base64,{}", STANDARD.encode(&bytes))
        );
        assert!(store
            .image("youtube", &item.id, false)
            .unwrap()
            .starts_with("data:image/jpeg;base64,"));
        drop(store);
        let reopened = Store::new(dir.path().join("data/screenshots"));
        let page = reopened.list("youtube", None, 24).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].title, "검증된 방송");
        assert_eq!(
            reopened.folder("youtube", &item.id).unwrap(),
            context.download_root.join("YouTube/Screenshots")
        );
        assert_eq!(
            reopened.image("youtube", &item.id, true).unwrap(),
            format!("data:image/png;base64,{}", STANDARD.encode(bytes))
        );
        assert!(reopened.list("soop", None, 24).unwrap().items.is_empty());
    }
    #[test]
    fn wrong_owner_sequence_total_and_expiry_are_rejected() {
        let (_dir, store, context, token, bytes) = fixture();
        let request = uuid::Uuid::new_v4().to_string();
        let id = store
            .begin(&token, &request, context.clone(), bytes.len(), 3, 2)
            .unwrap()
            .capture_id;
        assert_eq!(
            store
                .begin(&token, &request, context.clone(), bytes.len(), 3, 2)
                .unwrap()
                .capture_id,
            id
        );
        assert!(store
            .begin(&token, &request, context, bytes.len() + 1, 3, 2)
            .is_err());
        assert!(store
            .chunk(
                &uuid::Uuid::new_v4().to_string(),
                &id,
                0,
                &STANDARD.encode(&bytes)
            )
            .is_err());
        assert!(store
            .chunk(&token, &id, 1, &STANDARD.encode(&bytes))
            .is_err());
        assert!(store.finish(&token, &id, || Ok(())).is_err());
        store
            .transfers
            .lock()
            .unwrap()
            .pending
            .get_mut(&id)
            .unwrap()
            .expires = Instant::now() - Duration::from_secs(1);
        assert!(store
            .chunk(&token, &id, 0, &STANDARD.encode(bytes))
            .is_err());
        assert!(store.active_ids().is_empty());
    }
    #[test]
    fn four_pending_captures_all_finish_without_admission_double_count() {
        let (_dir, store, context, token, bytes) = fixture();
        let mut ids = Vec::new();
        for _ in 0..4 {
            ids.push(transfer(&store, context.clone(), &token, &bytes));
        }
        assert!(store
            .begin(
                &token,
                &uuid::Uuid::new_v4().to_string(),
                context.clone(),
                bytes.len(),
                3,
                2
            )
            .is_err());
        store
            .transfers
            .lock()
            .unwrap()
            .admitted
            .insert("test-quit-gate".into());
        for id in ids {
            store.finish(&token, &id, || Ok(())).unwrap();
        }
        assert_eq!(store.list("youtube", None, 24).unwrap().items.len(), 4);
        assert_eq!(store.active_ids(), ["platform-screenshot:test-quit-gate"]);
    }
    #[test]
    fn invalid_png_mismatch_truncation_and_limits_do_not_publish() {
        let (_dir, store, context, token, bytes) = fixture();
        assert!(store
            .begin(&token, "bad", context.clone(), bytes.len(), 3, 2)
            .is_err());
        assert!(store
            .begin(
                &token,
                &uuid::Uuid::new_v4().to_string(),
                context.clone(),
                MAX_BYTES + 1,
                3,
                2
            )
            .is_err());
        assert!(store
            .begin(
                &token,
                &uuid::Uuid::new_v4().to_string(),
                context.clone(),
                10,
                4096,
                4096
            )
            .is_err());
        assert!(validate_png(&bytes, 2, 2).is_err());
        assert!(validate_png(&bytes[..bytes.len() / 2], 3, 2).is_err());
        let id = transfer(&store, context.clone(), &token, b"not a PNG");
        assert!(store.finish(&token, &id, || Ok(())).is_err());
        assert!(store.list("youtube", None, 24).unwrap().items.is_empty());
        assert!(!context.download_root.exists());
    }
    #[test]
    fn exact_owner_undo_is_idempotent_finish_but_time_bounded() {
        let (_dir, store, context, token, bytes) = fixture();
        let item = save(&store, context.clone(), &token, &bytes);
        assert_eq!(
            store.finish(&token, &item.id, || Ok(())).unwrap().id,
            item.id
        );
        assert!(store
            .undo(&uuid::Uuid::new_v4().to_string(), &item.id)
            .is_err());
        store.undo(&token, &item.id).unwrap();
        assert!(!image_path(&context.download_root, &item).unwrap().exists());
        assert!(store.list("youtube", None, 24).unwrap().items.is_empty());
        let item = save(&store, context, &token, &bytes);
        store
            .transfers
            .lock()
            .unwrap()
            .receipts
            .get_mut(&item.id)
            .unwrap()
            .expires = Instant::now() - Duration::from_secs(1);
        assert!(store.undo(&token, &item.id).is_err());
        assert_eq!(store.list("youtube", None, 24).unwrap().items.len(), 1);
    }
    #[test]
    fn modified_same_handle_or_replaced_identical_file_is_never_deleted() {
        let (_dir, store, context, token, bytes) = fixture();
        let item = save(&store, context.clone(), &token, &bytes);
        let path = image_path(&context.download_root, &item).unwrap();
        let mut changed = bytes.clone();
        changed[20] ^= 1;
        fs::write(&path, changed).unwrap();
        assert!(store.undo(&token, &item.id).is_err());
        assert!(store.image("youtube", &item.id, true).is_err());
        fs::rename(&path, path.with_extension("original")).unwrap();
        fs::write(&path, &bytes).unwrap();
        let result = store.delete("youtube", &[item.id]).unwrap();
        assert_eq!(result.failed.len(), 1);
        assert!(path.exists());
    }
    #[test]
    fn hardlink_and_linked_parent_fail_closed() {
        let (dir, store, context, token, bytes) = fixture();
        let item = save(&store, context.clone(), &token, &bytes);
        let path = image_path(&context.download_root, &item).unwrap();
        let alias = dir.path().join("unrelated.png");
        fs::hard_link(&path, &alias).unwrap();
        assert!(store.image("youtube", &item.id, true).is_err());
        assert!(store.undo(&token, &item.id).is_err());
        assert!(path.exists() && alias.exists());
    }
    #[test]
    fn missing_owned_file_clears_only_stale_metadata() {
        let (dir, store, context, token, bytes) = fixture();
        let item = save(&store, context.clone(), &token, &bytes);
        let path = image_path(&context.download_root, &item).unwrap();
        let unrelated = path.parent().unwrap().join("do-not-touch.png");
        fs::write(&unrelated, b"unrelated").unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(
            store.delete("youtube", &[item.id.clone()]).unwrap().deleted,
            [item.id]
        );
        assert_eq!(fs::read(unrelated).unwrap(), b"unrelated");
        let reopened = Store::new(dir.path().join("data/screenshots"));
        assert!(reopened.list("youtube", None, 24).unwrap().items.is_empty());
    }
    #[test]
    fn cancellation_before_commit_and_shutdown_release_pending_but_not_inflight() {
        let (_dir, store, context, token, bytes) = fixture();
        let id = transfer(&store, context.clone(), &token, &bytes);
        assert!(store
            .finish(&token, &id, || {
                store.abort(&token, &id)?;
                Ok(())
            })
            .is_err());
        assert!(!context.download_root.exists());
        assert!(store.active_ids().is_empty());
        let id = transfer(&store, context.clone(), &token, &bytes);
        let operation = store
            .operation(uuid::Uuid::new_v4().to_string(), &token)
            .unwrap();
        store.shutdown();
        assert!(operation.cancel.load(Ordering::Acquire));
        assert_eq!(store.active_ids().len(), 1);
        assert!(store.chunk(&token, &id, 0, "AA==").is_err());
        drop(operation);
        assert!(store.active_ids().is_empty());
        assert!(store
            .begin(
                &token,
                &uuid::Uuid::new_v4().to_string(),
                context,
                bytes.len(),
                3,
                2
            )
            .is_err());
    }
    #[test]
    fn closed_live_validation_fails_before_any_publication() {
        let (_dir, store, context, token, bytes) = fixture();
        let id = transfer(&store, context.clone(), &token, &bytes);
        assert!(store.finish(&token, &id, || Err("closed".into())).is_err());
        assert!(!context.download_root.exists());
        assert!(store.active_ids().is_empty());
    }
    #[test]
    fn late_abort_of_saved_capture_uses_exact_owned_undo() {
        let (_dir, store, context, token, bytes) = fixture();
        let item = save(&store, context, &token, &bytes);
        store.abort(&token, &item.id).unwrap();
        assert!(store.list("youtube", None, 24).unwrap().items.is_empty());
    }
    #[test]
    fn provider_scoped_keyset_survives_new_items_and_deletions() {
        let (_dir, store, context, token, bytes) = fixture();
        for _ in 0..3 {
            save(&store, context.clone(), &token, &bytes);
        }
        let first = store.list("youtube", None, 1).unwrap();
        let cursor = first.cursor.unwrap();
        store
            .delete("youtube", &[first.items[0].id.clone()])
            .unwrap();
        let second = store.list("youtube", Some(&cursor), 100).unwrap();
        assert_eq!(second.items.len(), 2);
        assert!(second.items.iter().all(|i| i.id != first.items[0].id));
        assert!(store.list("soop", Some(&cursor), 24).is_err());
        assert!(store.list("youtube", Some(&"a".repeat(257)), 24).is_err());
        assert!(store.list("youtube", None, 101).is_err());
    }
    #[test]
    fn partial_or_unowned_files_are_never_adopted_or_deleted() {
        let (_dir, store, context, token, bytes) = fixture();
        let item = save(&store, context.clone(), &token, &bytes);
        let partial = store.journal.join("incomplete.partial");
        fs::write(&partial, b"unfinished").unwrap();
        let foreign = context
            .download_root
            .join("YouTube/Screenshots/foreign.png");
        fs::write(&foreign, &bytes).unwrap();
        store.delete("youtube", &[item.id]).unwrap();
        assert!(partial.exists() && foreign.exists());
    }
    #[test]
    fn serialized_metadata_does_not_expose_root_hash_or_file_identity() {
        let (_dir, store, context, token, bytes) = fixture();
        let item = save(&store, context, &token, &bytes);
        let value = serde_json::to_value(item).unwrap();
        assert!(value.get("createdAt").is_some());
        assert!(value.get("channelName").is_some());
        assert!(value.get("downloadRoot").is_none());
        assert!(value.get("identity").is_none());
        assert!(value.get("sha256").is_none());
    }
}
