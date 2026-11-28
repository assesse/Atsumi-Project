//! SOOP public display metadata. Protocol evidence: official LivePlayer module
//! 9915 and LiveView module 9216 / ordinary-message renderer, checked 2026-11-17.
//! This module never rewrites an archive or constructs an account/profile URL.
use super::*;
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: u32 = 3;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SoopChatMetadata {
    pub flag1: u32,
    pub flag2: u32,
    pub role: String,
    pub badge: String,
    pub subscription_tier: Option<u8>,
    pub subscription_months: Option<u16>,
    pub accumulated_subscription_months: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname_color_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub personalcon: Option<crate::soop::chat::personalcon::Metadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<crate::soop::chat::portrait::Metadata>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_type: Option<String>,
}
fn word(value: &Value) -> Option<u32> {
    let value = number(value)?;
    (i32::MIN as i64..=u32::MAX as i64)
        .contains(&value)
        .then_some(value as u32)
}
fn months(value: &Value) -> Option<u16> {
    number(value)
        .filter(|value| (1..=1200).contains(value))
        .map(|value| value as u16)
}
fn display(
    flag1: u32,
    flag2: u32,
    subscription_months: Option<u16>,
    accumulated_subscription_months: Option<u16>,
) -> SoopChatMetadata {
    // The official badge priority excludes subscription: a subscriber may still
    // have the F fan badge, while the container uses user-type="subscribe".
    let badge = if flag1 & 4 != 0 {
        "streamer"
    } else if flag1 & 256 != 0 {
        "manager"
    } else if flag1 & 32768 != 0 {
        "vip"
    } else if flag1 & 32 != 0 {
        "fan"
    } else if flag1 & (1 << 20) != 0 {
        "support"
    } else if flag1 & (1 << 26) != 0 {
        "nightbot"
    } else {
        "normal"
    };
    let subscription_tier = if flag2 & (1 << 19) != 0 {
        Some(2)
    } else if flag2 & (1 << 18) != 0 {
        Some(1)
    } else if flag2 & (1 << 20) != 0 {
        Some(3)
    } else {
        None
    };
    let role = if matches!(badge, "streamer" | "manager" | "vip") {
        badge
    } else if subscription_tier.is_some() {
        "subscribe"
    } else {
        badge
    };
    SoopChatMetadata {
        flag1,
        flag2,
        role: role.into(),
        badge: badge.into(),
        subscription_tier,
        subscription_months,
        accumulated_subscription_months,
        message_color: None,
        nickname_color_key: None,
        personalcon: None,
        avatar: None,
        message_type: None,
    }
}
pub(super) fn from_raw(value: &Value) -> Option<SoopChatMetadata> {
    if value["kind"] != "chat" {
        return None;
    }
    if value["extra"]["service"]==5 {
        if let Some(kind)=value["extra"]["messageType"].as_str().filter(|s|matches!(*s,"staff"|"police")){
            let mut metadata=display(0,0,None,None);metadata.message_type=Some(kind.into());return Some(metadata);
        }
    }
    let raw = value["flags"].as_str().filter(|raw| raw.len() <= 24)?;
    let (first, second) = raw.split_once('|')?;
    if second.contains('|') {
        return None;
    }
    let mut metadata = display(
        word(&Value::String(first.into()))?,
        word(&Value::String(second.into()))?,
        months(&value["extra"]["subscriptionMonth"]),
        months(&value["extra"]["accSubscriptionMonth"]),
    );
    metadata.message_color = message_color(&value["extra"]["color"]);
    metadata.nickname_color_key = crate::soop::chat::ogq::nickname_key(&value["extra"]["nicknameColorKey"]);
    metadata.personalcon=crate::soop::chat::personalcon::from_stored(&value["extra"]["personalcon"]);
    if metadata.badge=="streamer" && metadata.personalcon.is_none(){metadata.avatar=crate::soop::chat::portrait::from_stored(&value["extra"]["avatar"]);}
    Some(metadata)
}
pub(super) fn from_stored(value: &Value) -> Option<SoopChatMetadata> {
    if let Some(kind)=value["messageType"].as_str().filter(|s|matches!(*s,"staff"|"police")){
        let mut metadata=display(0,0,None,None);metadata.message_type=Some(kind.into());return Some(metadata);
    }
    // Recompute enums from verified numeric flags, never trust arbitrary stored
    // role/class names as CSS selectors or infer subscription from a label.
    let mut metadata = display(
        word(&value["flag1"])?,
        word(&value["flag2"])?,
        months(&value["subscriptionMonths"]),
        months(&value["accumulatedSubscriptionMonths"]),
    );
    metadata.message_color = literal_color(&value["messageColor"]);
    metadata.nickname_color_key = crate::soop::chat::ogq::nickname_key(&value["nicknameColorKey"]);
    metadata.personalcon=crate::soop::chat::personalcon::from_stored(&value["personalcon"]);
    if metadata.badge=="streamer" && metadata.personalcon.is_none(){metadata.avatar=crate::soop::chat::portrait::from_stored(&value["avatar"]);}
    Some(metadata)
}
pub(super) fn message_color(value: &Value) -> Option<String> {
    literal_color(value).or_else(|| {
        // SVC_CHATMESG packet[2] is a BGR integer. Zero means no explicit color.
        // Random nickname colors are packet[8]/[9], absent from old raw captures.
        let color = number(value).filter(|value| (1..=0xffffff).contains(value))? as u32;
        Some(format!(
            "#{:02X}{:02X}{:02X}",
            color & 255,
            (color >> 8) & 255,
            color >> 16
        ))
    })
}

#[derive(Clone, PartialEq, Eq)]
struct PartStamp {
    stamp: Option<Stamp>,
    bytes: u64,
    hash: Option<String>,
}
#[derive(Clone)]
pub(super) struct Source {
    version: u32,
    directory: PathBuf,
    multipart: Option<chat_parts::ReplaySource>,
    stamps: Vec<PartStamp>,
    pub(super) bytes: u64,
}
impl Source {
    fn file(&self, ordinal: usize) -> Result<File, String> {
        let file = if let Some(parts) = &self.multipart {
            chat_parts::open_replay_source_part(parts, ordinal)?.file
        } else {
            if ordinal != 0 {
                return Err(CHANGED.into());
            }
            open(&self.directory.join("chat.source.jsonl"), MAX_FILE)?
        };
        let expected = self.stamps.get(ordinal).ok_or(CHANGED)?;
        let actual = stamp(&file)?;
        if actual.length != expected.bytes
            || expected
                .stamp
                .as_ref()
                .is_some_and(|expected| expected != &actual)
        {
            return Err(CHANGED.into());
        }
        Ok(file)
    }
    pub(super) fn verify(&self, progress: &Progress) -> Result<(), String> {
        if self.multipart.is_none() {
            self.file(0)?;
        } else {
            // Sealed multipart handles retain FILE_SHARE_READ only on Windows:
            // no writer or replacement can change the verified bytes while the
            // replay index owns them. Do not reopen every part for every second.
            if progress.files.len() != self.stamps.len() {
                return Err(CHANGED.into());
            }
            #[cfg(not(windows))]
            for (ordinal, held) in progress.files.iter().enumerate() {
                if stamp(&held.file)? != held.stamp || stamp(&self.file(ordinal)?)? != held.stamp {
                    return Err(CHANGED.into());
                }
            }
        }
        Ok(())
    }
}
pub(super) fn same_source(left: Option<&Source>, right: Option<&Source>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.version == right.version
                && left.directory == right.directory
                && left.stamps == right.stamps
                && left.multipart.as_ref().map(|source| &source.generation)
                    == right.multipart.as_ref().map(|source| &source.generation)
        }
        _ => false,
    }
}
pub(super) fn source(
    directory: &Path,
    provider: &str,
    multipart: Option<&chat_parts::ReplaySource>,
) -> Result<Option<Source>, String> {
    if provider != "soop" {
        return Ok(None);
    }
    let mut stamps = Vec::new();
    if let Some(parts) = multipart {
        for part in &parts.source {
            stamps.push(PartStamp {
                stamp: None,
                bytes: part.bytes,
                hash: Some(part.sha256.clone()),
            });
        }
    } else {
        let path = directory.join("chat.source.jsonl");
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(CHANGED.into()),
            Ok(_) => {}
        }
        let file = open(&path, MAX_FILE)?;
        stamps.push(PartStamp {
            stamp: Some(stamp(&file)?),
            bytes: file.metadata().map_err(|_| CHANGED)?.len(),
            hash: None,
        });
    }
    if stamps.is_empty() {
        return Ok(None);
    }
    let bytes = stamps.iter().map(|part| part.bytes).sum();
    Ok(Some(Source {
        version: SCHEMA_VERSION,
        directory: directory.into(),
        multipart: multipart.cloned(),
        stamps,
        bytes,
    }))
}

#[derive(Default)]
pub(super) struct Progress {
    part: usize,
    offset: u64,
    pub(super) scanned: u64,
    hash: Sha256,
    pub(super) ready: bool,
    pub(super) failed: bool,
    files: Vec<HeldSource>,
}
struct HeldSource {
    file: File,
    stamp: Stamp,
}
impl Progress {
    pub(super) fn disable(&mut self) {
        self.failed = true;
        self.ready = true;
        self.files.clear();
    }
}
fn fingerprint(row: &Parsed) -> Vec<u8> {
    let mut hash = Sha256::new();
    hash.update(row.wall_ms.unwrap_or(i64::MIN).to_le_bytes());
    hash.update((row.item.author.len() as u64).to_le_bytes());
    hash.update(row.item.author.as_bytes());
    hash.update(row.item.text.as_bytes());
    hash.finalize().to_vec()
}
fn create_table(database: &Connection) -> Result<(), String> {
    // Only digests and bounded public display metadata are retained in the
    // existing auto-deleted temporary index; no nickname, text or account ID.
    database.execute_batch("CREATE TABLE soop_display(id TEXT PRIMARY KEY, fingerprint BLOB NOT NULL, display TEXT NOT NULL);")
        .map_err(|_| LIMIT_ERROR.to_string())
}
pub(super) fn advance(entry: &Entry, index: &mut Index, budget: SliceBudget) -> Result<(), String> {
    let Some(source) = &entry.soop_source else {
        index.soop.ready = true;
        return Ok(());
    };
    if index.soop.ready {
        return Ok(());
    }
    if index.soop.part == 0 && index.soop.offset == 0 {
        // Unlike the mandatory index, this optional phase can fail softly.
        // OFF cannot roll back a failed INSERT/SQLITE_FULL and may damage the
        // already prepared message rows, so require a real rollback journal.
        let mode: String = index
            .database
            .query_row("PRAGMA journal_mode=DELETE", [], |row| row.get(0))
            .map_err(|_| LIMIT_ERROR.to_string())?;
        if mode != "delete" && mode != "memory" {
            return Err(LIMIT_ERROR.into());
        }
        create_table(&index.database)?;
    }
    let started = Instant::now();
    let initial = index.soop.scanned;
    let mut units = 0usize;
    let transaction = index
        .database
        .transaction()
        .map_err(|_| LIMIT_ERROR.to_string())?;
    let mut insert = transaction
        .prepare("INSERT OR IGNORE INTO soop_display(id,fingerprint,display) VALUES(?1,?2,?3)")
        .map_err(|_| LIMIT_ERROR.to_string())?;
    while index.soop.part < source.stamps.len() {
        check_cancelled(entry)?;
        if units > 0
            && (budget.bytes.saturating_sub(index.soop.scanned - initial) < MAX_LINE as u64
                || units >= budget.units
                || started.elapsed() >= budget.time)
        {
            break;
        }
        let expected = &source.stamps[index.soop.part];
        if index.soop.files.len() == index.soop.part {
            let file = source.file(index.soop.part)?;
            let stamp = stamp(&file)?;
            index.soop.files.push(HeldSource { file, stamp });
        }
        let held = index.soop.files.get_mut(index.soop.part).ok_or(CHANGED)?;
        if stamp(&held.file)? != held.stamp {
            return Err(CHANGED.into());
        }
        held.file
            .seek(SeekFrom::Start(index.soop.offset))
            .map_err(|_| CHANGED)?;
        let mut reader = BufReader::new(&mut held.file);
        while index.soop.offset < expected.bytes {
            check_cancelled(entry)?;
            if units > 0
                && (budget.bytes.saturating_sub(index.soop.scanned - initial) < MAX_LINE as u64
                    || units >= budget.units
                    || started.elapsed() >= budget.time)
            {
                break;
            }
            let bytes = line(&mut reader)?;
            if bytes.is_empty()
                || index.soop.offset.saturating_add(bytes.len() as u64) > expected.bytes
                || (expected.hash.is_some() && bytes.last() != Some(&b'\n'))
            {
                return Err(CHANGED.into());
            }
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                if value["format"] != "atsumi-chat-v1" && value["kind"] == "chat" {
                    let parsed = parse_soop(&value);
                    if let Some(row) = parsed.items.first() {
                        if let Some(id) = row.source_id.as_ref().filter(|_| row.wall_ms.is_some()) {
                            let display = row
                                .item
                                .soop
                                .as_ref()
                                .map(serde_json::to_string)
                                .transpose()
                                .map_err(|_| LIMIT_ERROR)?;
                            if let Some(display) = display {
                                insert
                                    .execute(params![
                                        format!("{:x}", Sha256::digest(id.as_bytes())),
                                        fingerprint(row),
                                        display
                                    ])
                                    .map_err(|_| LIMIT_ERROR.to_string())?;
                            }
                        }
                    }
                }
            }
            index.soop.hash.update(&bytes);
            index.soop.offset += bytes.len() as u64;
            index.soop.scanned += bytes.len() as u64;
            units += 1;
        }
        if stamp(reader.get_ref())? != held.stamp {
            return Err(CHANGED.into());
        }
        if index.soop.offset != expected.bytes {
            break;
        }
        if expected.hash.as_ref().is_some_and(|expected| {
            format!("{:x}", index.soop.hash.clone().finalize()) != *expected
        }) {
            return Err("채팅 원본 조각 검증에 실패했습니다. 원본은 보존됩니다.".into());
        }
        index.soop.part += 1;
        units += 1;
        index.soop.offset = 0;
        index.soop.hash = Sha256::new();
    }
    drop(insert);
    check_cancelled(entry)?;
    transaction.commit().map_err(|_| LIMIT_ERROR.to_string())?;
    index.soop.ready = index.soop.part == source.stamps.len();
    if index.soop.ready {
        source.verify(&index.soop)?;
        if source.multipart.is_none() {
            index.soop.files.clear();
        }
    }
    Ok(())
}
pub(super) fn enrich(database: &Connection, row: &mut Parsed) -> Result<(), String> {
    if row.item.kind != "chat" || row.item.soop.is_some() {
        return Ok(());
    }
    let Some(id) = row
        .source_id
        .as_ref()
        .filter(|id| id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))
    else {
        return Ok(());
    };
    let metadata: Option<String> = database
        .query_row(
            "SELECT display FROM soop_display WHERE id=?1 AND fingerprint=?2",
            params![id, fingerprint(row)],
            |value| value.get(0),
        )
        .optional()
        .map_err(|_| "채팅 표시 정보를 읽지 못했습니다.")?;
    if let Some(display) = metadata {
        row.item.soop = serde_json::from_str::<Value>(&display)
            .ok()
            .and_then(|value| from_stored(&value));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn raw(flags: &str) -> Value {
        json!({"kind":"chat","nickname":"Fixture","text":"saved message","messageId":"fixture-id",
            "receivedAtMs":1700000001000i64,"flags":flags,
            "extra":{"service":5,"color":"0","subscriptionMonth":"17","accSubscriptionMonth":"17"}})
    }
    #[test]
    fn personalcon_roundtrip_and_staff_role_do_not_replace_history_with_current_data() {
        let request=crate::soop::chat::personalcon::Context::from_channel(&json!({"PCON_OBJECT":null})).select("32|262144","17","0").0.unwrap();
        let mut value=raw("32|262144");value["extra"]["personalcon"]=serde_json::to_value(request.metadata()).unwrap();
        let meta=from_raw(&value).unwrap();assert_eq!(meta.personalcon.as_ref(),Some(request.metadata()));
        assert_eq!(from_stored(&serde_json::to_value(&meta).unwrap()),Some(meta));
        let old=from_raw(&raw("32|262144")).unwrap();assert!(old.personalcon.is_none());
        value["extra"]["personalcon"]["imageAssetId"]=json!("https://evil.test/current.png");
        assert!(from_raw(&value).unwrap().personalcon.is_none());
        let parsed=parse_line(&serde_json::to_vec(&value).unwrap(),"soop").unwrap();assert_eq!(parsed.items.len(),1);assert_eq!(parsed.unsupported,1);
        for kind in ["staff","police"] {value["extra"]["messageType"]=json!(kind);let m=from_raw(&value).unwrap();assert_eq!(m.message_type.as_deref(),Some(kind));assert!(m.personalcon.is_none()&&m.nickname_color_key.is_none());assert_eq!(m.role,"normal");}
        assert_eq!(from_raw(&raw("0|786432")).unwrap().subscription_tier,Some(2));
    }
    #[test]
    fn broadcaster_avatar_roundtrip_requires_streamer_role_and_never_replaces_personalcon() {
        let context=crate::soop::chat::personalcon::Context::from_channel(&json!({}));let request=context.portrait("fixture","fixture","4|0","0").unwrap();
        let mut value=raw("4|0");value["extra"]["avatar"]=serde_json::to_value(request.metadata()).unwrap();let meta=from_raw(&value).unwrap();assert_eq!(meta.avatar.as_ref(),Some(request.metadata()));assert_eq!(from_stored(&serde_json::to_value(&meta).unwrap()),Some(meta));
        value["flags"]=json!("32|0");assert!(from_raw(&value).unwrap().avatar.is_none());value["flags"]=json!("4|262144");
        let icon=context.select("4|262144","17","0").0.unwrap();value["extra"]["personalcon"]=serde_json::to_value(icon.metadata()).unwrap();assert!(from_raw(&value).unwrap().avatar.is_none());
    }
    fn old_event(raw: &Value) -> Value {
        json!({"format":"atsumi-chat-v1","kind":"chat","author":raw["nickname"],"text":raw["text"],
            "messageId":format!("{:x}",Sha256::digest(raw["messageId"].as_str().unwrap().as_bytes())),
            "sourceTimestampMs":raw["receivedAtMs"],"receivedAtMs":1700000001200i64,
            "amount":null,"badges":["구독 17개월"],"color":null})
    }
    fn write(directory: &Path, name: &str, value: &Value) {
        fs::write(
            directory.join(name),
            format!("{}\n", serde_json::to_string(value).unwrap()),
        )
        .unwrap();
    }
    fn page(cache: &IndexCache, directory: &Path, job: &str) -> Result<ReplayChatPage, String> {
        for _ in 0..1000 {
            let page =
                read_page_for_job_using(cache, directory, "soop", job, 10_000, None, 120, SLICE)?;
            if page.state != "indexing" {
                return Ok(page);
            }
        }
        panic!("bounded metadata preparation did not finish")
    }
    #[test]
    fn official_flags_distinguish_fan_badge_from_subscription_container_and_prioritize_roles() {
        let value = from_raw(&raw("269025824|425984")).unwrap();
        assert_eq!(
            (
                value.badge.as_str(),
                value.role.as_str(),
                value.subscription_tier
            ),
            ("fan", "subscribe", Some(1))
        );
        assert_eq!(
            (
                value.subscription_months,
                value.accumulated_subscription_months
            ),
            (Some(17), Some(17))
        );
        for (flags, badge, role, tier) in [
            ("32|0", "fan", "fan", None),
            ("1048576|0", "support", "support", None),
            ("67108864|0", "nightbot", "nightbot", None),
            ("0|524288", "normal", "subscribe", Some(2)),
            ("32|1048576", "fan", "subscribe", Some(3)),
            ("32800|262144", "vip", "vip", Some(1)),
            ("33056|262144", "manager", "manager", Some(1)),
            ("33060|262144", "streamer", "streamer", Some(1)),
        ] {
            let value = from_raw(&raw(flags)).unwrap();
            assert_eq!(
                (
                    value.badge.as_str(),
                    value.role.as_str(),
                    value.subscription_tier
                ),
                (badge, role, tier)
            );
        }
    }
    #[test]
    fn invalid_flags_do_not_invent_roles_and_color_is_only_the_explicit_bgr_value() {
        for flags in ["", "32", "32|0|4", "x|0", "4294967296|0", "-2147483649|0"] {
            assert!(from_raw(&raw(flags)).is_none(), "{flags}");
        }
        let mut value = raw("32|0");
        value["extra"]["subscriptionMonth"] = json!(-1);
        value["extra"]["accSubscriptionMonth"] = json!(999999);
        assert_eq!(from_raw(&value).unwrap().subscription_months, None);
        assert_eq!(
            from_raw(&value).unwrap().accumulated_subscription_months,
            None
        );
        for value in [
            json!(0),
            json!(-1),
            json!(0x1000000u32),
            json!("red"),
            json!("url(x)"),
        ] {
            assert_eq!(message_color(&value), None);
        }
        assert_eq!(message_color(&json!("1122867")), Some("#332211".into()));
        assert_eq!(message_color(&json!("#55AAFF")), Some("#55AAFF".into()));
        value["extra"]["color"] = json!("1122867");
        let parsed = parse_soop(&value);
        assert_eq!(
            parsed.items[0].item.color, None,
            "body color must not become nickname color"
        );
        assert_eq!(
            parsed.items[0].item.soop.as_ref().unwrap().message_color,
            Some("#332211".into())
        );
        let mut stored = serde_json::to_value(from_raw(&raw("32|0")).unwrap()).unwrap();
        stored["role"] = json!("streamer");
        assert_eq!(from_stored(&stored).unwrap().role, "fan");
    }
    #[test]
    fn normalized_roundtrip_preserves_soop_metadata_but_youtube_never_adopts_it() {
        let events =
            normalize_public_line("soop", &serde_json::to_vec(&raw("32|262144")).unwrap()).unwrap();
        let bytes = serde_json::to_vec(&events[0]).unwrap();
        assert_eq!(
            parse_line(&bytes, "soop").unwrap().items[0]
                .item
                .soop
                .as_ref()
                .unwrap()
                .badge,
            "fan"
        );
        assert!(parse_line(&bytes, "youtube").unwrap().items[0]
            .item
            .soop
            .is_none());
    }
    #[test]
    fn ordinary_palette_normalized_roundtrip_preserves_fields_and_revalidates_provider() {
        let mut raw = raw("32|262144");
        raw["extra"]["nicknameColorKey"] = json!("E12E2E");
        raw["extra"]["color"] = json!("255");
        let normalized = normalize_public_line("soop", &serde_json::to_vec(&raw).unwrap()).unwrap();
        assert_eq!(normalized.len(), 1);
        let bytes = serde_json::to_vec(&normalized[0]).unwrap();
        let parsed = parse_line(&bytes, "soop").unwrap();
        let item = &parsed.items[0].item;
        let display = item.soop.as_ref().unwrap();
        assert_eq!(display.nickname_color_key.as_deref(), Some("E12E2E"));
        assert_eq!(display.message_color.as_deref(), Some("#FF0000"));
        assert_eq!(display.badge, "fan");
        assert_eq!(display.role, "subscribe");
        assert_eq!(item.author, raw["nickname"].as_str().unwrap());
        assert_eq!(item.text, raw["text"].as_str().unwrap());
        assert!(parse_line(&bytes, "youtube").unwrap().items[0].item.soop.is_none());
        let mut forged = serde_json::to_value(&normalized[0]).unwrap();
        forged["soop"]["nicknameColorKey"] = json!("random-color1 injected");
        let forged_row = parse_line(&serde_json::to_vec(&forged).unwrap(), "soop").unwrap();
        assert!(forged_row.items[0].item.soop.as_ref().unwrap().nickname_color_key.is_none());
        assert_eq!(forged_row.items[0].item.text, item.text);
        raw["extra"].as_object_mut().unwrap().remove("nicknameColorKey");
        let legacy = normalize_public_line("soop", &serde_json::to_vec(&raw).unwrap()).unwrap();
        assert!(legacy[0].soop.as_ref().unwrap().nickname_color_key.is_none());
    }
    #[test]
    fn old_saved_events_are_enriched_without_changing_ids_timing_text_or_source_files() {
        let directory = tempfile::tempdir().unwrap();
        let raw = raw("269025824|425984");
        write(directory.path(), "chat.events.jsonl", &old_event(&raw));
        let cache = IndexCache::default();
        let before = page(&cache, directory.path(), "").unwrap();
        assert!(!before.display_metadata_unavailable, "an absent optional sidecar is not a display failure");
        write(directory.path(), "chat.source.jsonl", &raw);
        let event_bytes = fs::read(directory.path().join("chat.events.jsonl")).unwrap();
        let raw_bytes = fs::read(directory.path().join("chat.source.jsonl")).unwrap();
        let after = page(&cache, directory.path(), "").unwrap();
        assert_eq!(after.items.len(), 1);
        let metadata = after.items[0].soop.as_ref().unwrap();
        assert_eq!(
            (metadata.role.as_str(), metadata.badge.as_str()),
            ("subscribe", "fan")
        );
        let mut actual = serde_json::to_value(&after.items[0]).unwrap();
        actual.as_object_mut().unwrap().remove("soop");
        assert_eq!(actual, serde_json::to_value(&before.items[0]).unwrap());
        assert_eq!(
            fs::read(directory.path().join("chat.events.jsonl")).unwrap(),
            event_bytes
        );
        assert_eq!(
            fs::read(directory.path().join("chat.source.jsonl")).unwrap(),
            raw_bytes
        );
    }
    #[test]
    fn raw_identity_changes_invalidate_metadata_and_mismatched_records_do_not_join() {
        let directory = tempfile::tempdir().unwrap();
        let mut raw = raw("32|0");
        write(directory.path(), "chat.events.jsonl", &old_event(&raw));
        write(directory.path(), "chat.source.jsonl", &raw);
        let cache = IndexCache::default();
        assert_eq!(
            page(&cache, directory.path(), "").unwrap().items[0]
                .soop
                .as_ref()
                .unwrap()
                .role,
            "fan"
        );
        raw["flags"] = json!("256|0");
        write(directory.path(), "chat.source.jsonl", &raw);
        assert_eq!(
            page(&cache, directory.path(), "").unwrap().items[0]
                .soop
                .as_ref()
                .unwrap()
                .role,
            "manager"
        );
        raw["text"] = json!("different message with the same id");
        write(directory.path(), "chat.source.jsonl", &raw);
        assert!(page(&cache, directory.path(), "").unwrap().items[0]
            .soop
            .is_none());
        fs::write(directory.path().join("chat.source.jsonl"), b"{malformed}\n").unwrap();
        assert!(page(&cache, directory.path(), "").unwrap().items[0]
            .soop
            .is_none());
    }
    #[test]
    fn source_hardlinks_are_rejected_without_modifying_the_event_archive() {
        let directory = tempfile::tempdir().unwrap();
        let raw = raw("32|0");
        write(directory.path(), "chat.events.jsonl", &old_event(&raw));
        write(directory.path(), "chat.source.jsonl", &raw);
        fs::hard_link(
            directory.path().join("chat.source.jsonl"),
            directory.path().join("raw-alias"),
        )
        .unwrap();
        let result = page(&IndexCache::default(), directory.path(), "").unwrap();
        assert_eq!(result.items.len(), 1);
        assert!(result.display_metadata_unavailable);
        assert!(result.items[0].soop.is_none());
        assert!(directory.path().join("chat.events.jsonl").is_file());
    }
    #[test]
    fn multipart_source_hash_is_verified_before_metadata_is_visible() {
        use super::super::super::chat_status;
        let directory = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let marker = chat_status::begin_multipart(directory.path(), "soop", &id).unwrap();
        let mut writer = chat_parts::Writer::create(directory.path(), &marker).unwrap();
        let raw = raw("32|262144");
        writer
            .append(
                chat_parts::Stream::Events,
                &serde_json::to_vec(&old_event(&raw)).unwrap(),
            )
            .unwrap();
        writer
            .append(
                chat_parts::Stream::Source,
                &serde_json::to_vec(&raw).unwrap(),
            )
            .unwrap();
        chat_status::finish_multipart(
            directory.path(),
            &marker,
            writer.close(),
            None,
            false,
            &AtomicBool::new(false),
        )
        .unwrap();
        let cache = IndexCache::default();
        assert_eq!(
            page(&cache, directory.path(), &id).unwrap().items[0]
                .soop
                .as_ref()
                .unwrap()
                .role,
            "subscribe"
        );
        let mut bytes = fs::read(directory.path().join("chat.source.jsonl")).unwrap();
        let original = bytes.clone();
        #[cfg(windows)]
        assert!(
            OpenOptions::new()
                .write(true)
                .open(directory.path().join("chat.source.jsonl"))
                .is_err(),
            "verified source handle must exclude mutation while cached"
        );
        // Repeated pages reuse verified handles and remain readable.
        assert!(page(&cache, directory.path(), &id).unwrap().items[0]
            .soop
            .is_some());
        cache.cancel_matching(|_| true);
        let at = bytes.iter().position(|byte| *byte == b'F').unwrap();
        bytes[at] = b'G';
        fs::write(directory.path().join("chat.source.jsonl"), bytes).unwrap();
        let degraded = page(&cache, directory.path(), &id).unwrap();
        assert_eq!(degraded.items.len(), 1);
        assert!(degraded.display_metadata_unavailable && degraded.items[0].soop.is_none());
        cache.cancel_matching(|_| true);
        fs::remove_file(directory.path().join("chat.source.jsonl")).unwrap();
        let missing = page(&cache, directory.path(), &id).unwrap();
        assert_eq!(missing.items.len(), 1);
        assert!(missing.display_metadata_unavailable && missing.items[0].soop.is_none());
        // A repaired sealed sidecar is rechecked on the next opened replay.
        cache.cancel_matching(|_| true);
        fs::write(directory.path().join("chat.source.jsonl"), original).unwrap();
        let repaired = page(&cache, directory.path(), &id).unwrap();
        assert!(!repaired.display_metadata_unavailable && repaired.items[0].soop.is_some());
    }
    #[test]
    fn optional_raw_limits_discard_all_metadata_but_keep_normalized_chat_and_allow_repair() {
        let directory = tempfile::tempdir().unwrap();
        let raw = raw("32|0");
        write(directory.path(), "chat.events.jsonl", &old_event(&raw));
        let cache = IndexCache::default();
        let baseline = page(&cache, directory.path(), "").unwrap();
        let path = directory.path().join("chat.source.jsonl");
        // A good first row must not leak its metadata if a later raw line fails.
        let mut bytes = format!("{}\n", serde_json::to_string(&raw).unwrap()).into_bytes();
        bytes.extend(vec![b'x'; MAX_LINE + 1]);
        bytes.push(b'\n');
        fs::write(&path, bytes).unwrap();
        let degraded = page(&cache, directory.path(), "").unwrap();
        assert!(degraded.display_metadata_unavailable);
        assert_eq!(
            serde_json::to_value(&degraded.items).unwrap(),
            serde_json::to_value(&baseline.items).unwrap()
        );
        assert!(!degraded.partial_file);
        File::create(&path).unwrap().set_len(MAX_FILE + 1).unwrap();
        let oversized = page(&cache, directory.path(), "").unwrap();
        assert!(oversized.display_metadata_unavailable && oversized.items[0].soop.is_none());
        write(directory.path(), "chat.source.jsonl", &raw);
        let repaired = page(&cache, directory.path(), "").unwrap();
        assert!(!repaired.display_metadata_unavailable && repaired.items[0].soop.is_some());
        cache.cancel_matching(|_| true);
        fs::remove_file(directory.path().join("chat.events.jsonl")).unwrap();
        // Unlike an optional sidecar, the primary legacy raw chat stays strict.
        File::create(directory.path().join("chat.soop.jsonl"))
            .unwrap()
            .set_len(MAX_FILE + 1)
            .unwrap();
        assert!(page(&cache, directory.path(), "").is_err());
    }
    #[test]
    fn source_preparation_is_chunked_and_never_exposes_partial_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let raw = raw("32|0");
        write(directory.path(), "chat.events.jsonl", &old_event(&raw));
        let line = format!("{}\n", serde_json::to_string(&raw).unwrap());
        fs::write(directory.path().join("chat.source.jsonl"), line.repeat(5)).unwrap();
        let event_bytes = fs::metadata(directory.path().join("chat.events.jsonl"))
            .unwrap()
            .len();
        let cache = IndexCache::default();
        let budget = SliceBudget {
            bytes: MAX_LINE as u64,
            units: 1,
            time: Duration::from_secs(1),
        };
        let first =
            read_page_for_job_using(&cache, directory.path(), "soop", "", 0, None, 120, budget)
                .unwrap();
        assert_eq!(first.state, "indexing");
        assert!(first.items.is_empty());
        let preparation = first.preparation.unwrap();
        assert_eq!(preparation.total_bytes, event_bytes + line.len() as u64 * 5);
        assert_eq!(preparation.scanned_bytes, event_bytes + line.len() as u64);
        let mut result = first.state;
        for _ in 0..4 {
            let next =
                read_page_for_job_using(&cache, directory.path(), "soop", "", 0, None, 120, budget)
                    .unwrap();
            result = next.state;
            if result == "indexing" {
                assert!(next.items.is_empty());
            } else {
                assert_eq!(next.items[0].soop.as_ref().unwrap().badge, "fan");
            }
        }
        assert_eq!(result, "unmapped");
    }
    #[test]
    fn metadata_sqlite_full_rolls_back_without_damaging_prepared_messages() {
        let directory = tempfile::tempdir().unwrap();
        let initial = raw("32|0");
        write(directory.path(), "chat.events.jsonl", &old_event(&initial));
        let cache = IndexCache::default();
        let baseline = page(&cache, directory.path(), "").unwrap();
        let mut source = String::new();
        for index in 0..100 {
            let mut value = initial.clone();
            if index != 0 {
                value["messageId"] = json!(format!("fixture-{index}"));
            }
            source.push_str(&serde_json::to_string(&value).unwrap());
            source.push('\n');
        }
        fs::write(directory.path().join("chat.source.jsonl"), source).unwrap();
        let budget = SliceBudget {
            bytes: MAX_LINE as u64,
            units: 1,
            time: Duration::from_secs(1),
        };
        let first =
            read_page_for_job_using(&cache, directory.path(), "soop", "", 0, None, 120, budget)
                .unwrap();
        assert_eq!(first.state, "indexing");
        let entry = cache
            .registry
            .lock()
            .unwrap()
            .entries
            .back()
            .unwrap()
            .clone();
        {
            let state = entry.index.lock().unwrap();
            let database = &state.as_ref().unwrap().database;
            let mode: String = database
                .pragma_query_value(None, "journal_mode", |row| row.get(0))
                .unwrap();
            assert!(mode == "delete" || mode == "memory");
            let pages: u64 = database
                .pragma_query_value(None, "page_count", |row| row.get(0))
                .unwrap();
            database
                .pragma_update(None, "max_page_count", pages)
                .unwrap();
        }
        let result = page(&cache, directory.path(), "").unwrap();
        assert!(result.display_metadata_unavailable);
        assert_eq!(
            serde_json::to_value(&result.items).unwrap(),
            serde_json::to_value(&baseline.items).unwrap()
        );
        let state = entry.index.lock().unwrap();
        let check: String = state
            .as_ref()
            .unwrap()
            .database
            .pragma_query_value(None, "quick_check", |row| row.get(0))
            .unwrap();
        assert_eq!(check, "ok");
    }
    #[test]
    #[ignore = "read-only actual owned SOOP archive; set ATSUMI_SOOP_REPLAY_FIXTURE"]
    fn actual_soop_display_metadata_counts_only() {
        let directory =
            PathBuf::from(std::env::var_os("ATSUMI_SOOP_REPLAY_FIXTURE").expect("fixture path"));
        let id = directory.file_name().unwrap().to_str().unwrap();
        let names = ["chat.events.jsonl", "chat.source.jsonl"];
        let hashes: Vec<_> = names
            .iter()
            .map(|name| Sha256::digest(fs::read(directory.join(name)).unwrap()))
            .collect();
        let source_rows: Vec<Value> = fs::read_to_string(directory.join("chat.source.jsonl"))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(source_rows.len(), 15);
        assert_eq!(
            source_rows
                .iter()
                .filter_map(from_raw)
                .filter(|row| row.badge == "fan")
                .count(),
            15
        );
        let origin = anchor(&directory, "soop", id).unwrap().0.unwrap();
        assert_eq!(
            source_rows
                .iter()
                .filter(|row| number(&row["receivedAtMs"]).is_some_and(|at| at < origin))
                .count(),
            1
        );
        // The first source row precedes the existing video anchor by 690 ms.
        // Preserve that pre-existing exclusion instead of changing playback time.
        let cache = IndexCache::default();
        let mut actual =
            read_page_for_job_using(&cache, &directory, "soop", id, 0, Some("0"), 120, SLICE)
                .unwrap();
        while actual.state == "indexing" {
            actual =
                read_page_for_job_using(&cache, &directory, "soop", id, 0, Some("0"), 120, SLICE)
                    .unwrap();
        }
        assert_eq!(actual.items.len(), 14);
        let mut expected = Vec::new();
        let mut offset = 0usize;
        for line in fs::read(directory.join("chat.events.jsonl"))
            .unwrap()
            .split_inclusive(|byte| *byte == b'\n')
        {
            for (ordinal, mut row) in parse_line(line, "soop")
                .unwrap()
                .items
                .into_iter()
                .enumerate()
            {
                let at = row
                    .offset_ms
                    .or_else(|| row.wall_ms.and_then(|wall| wall.checked_sub(origin)));
                if let Some(at) = at.filter(|at| *at >= 0) {
                    row.item.id = format!("{offset}:{ordinal}");
                    row.item.time_ms = Some(at as u64);
                    row.item.soop = None;
                    expected.push(serde_json::to_value(row.item).unwrap());
                }
            }
            offset += line.len();
        }
        let returned: Vec<_> = actual
            .items
            .iter()
            .cloned()
            .map(|mut row| {
                row.soop = None;
                serde_json::to_value(row).unwrap()
            })
            .collect();
        // A boolean assertion avoids emitting private source text on failure.
        assert!(
            returned == expected,
            "metadata enrichment changed existing replay fields"
        );
        assert_eq!(
            actual
                .items
                .iter()
                .filter(|row| row.soop.as_ref().is_some_and(|m| m.badge == "fan"))
                .count(),
            14
        );
        assert_eq!(
            actual
                .items
                .iter()
                .filter(|row| row.soop.as_ref().is_some_and(|m| m.role == "fan"))
                .count(),
            13
        );
        assert_eq!(
            actual
                .items
                .iter()
                .filter(
                    |row| row.soop.as_ref().is_some_and(|m| m.role == "subscribe"
                        && m.subscription_tier == Some(1)
                        && m.accumulated_subscription_months == Some(17))
                )
                .count(),
            1
        );
        for (name, expected) in names.iter().zip(hashes) {
            assert_eq!(
                Sha256::digest(fs::read(directory.join(name)).unwrap()),
                expected
            );
        }
        eprintln!("SOOP actual read-only metadata: rawRows=15 rawFanBadges=15 replayRows=14 replayFanBadges=14 replayFanContainers=13 replaySubscriberContainers=1 preAnchorExcluded=1 existingReplayFieldsUnchanged=true originalsUnchanged=true");
    }
}
