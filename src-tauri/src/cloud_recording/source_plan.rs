//! Immutable recording plans. Capture parts are inputs, never implicit cloud videos.
use super::*;
use crate::streaming::browser_merge::{self, MediaTools};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::{self, OpenOptions}, io::{Read, Seek, SeekFrom, Write}, time::UNIX_EPOCH};

const MAX_SECONDS: f64 = 6.0 * 3600.0;
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const PROVIDER_BYTES: u64 = super::super::upload::MAX_BYTES;
const MAX_PLAN_BYTES: u64 = 4 * 1024 * 1024;
const RESERVE: u64 = 512 * 1024 * 1024;

#[cfg(test)]
#[path = "source_plan_replay_tests.rs"]
mod replay_binding_tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCoverage {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub expected_segments: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub covered_segments: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub missing_from_segment: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")] pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub capture_warning: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcePart {
    pub source: CloudSourceRef,
    pub unit_index: u32,
    pub part_index: Option<u32>,
    pub duration_seconds: f64,
    pub media_offset_seconds: f64,
    pub bytes: u64,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDescription {
    pub source: CloudSourceRef,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")] pub plan_id: Option<String>,
    pub coverage: SourceCoverage,
    pub preparation_required: bool,
    pub duration_seconds: f64,
    pub bytes: u64,
    pub parts: Vec<SourcePart>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Identity { bytes: u64, modified_nanos: u128, digest: Option<String>, duration: f64, offset: f64 }
struct Input { file: File, path: PathBuf, identity: Identity }
struct Inputs {
    reference: CloudSourceRef, title: String, entries: Vec<Input>,
    coverage: SourceCoverage, parts: bool, fingerprint: String,
    lease: Box<dyn Send + Sync>, store: Option<BrowserCaptureStore>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Unit {
    first: usize, end: usize,
    #[serde(default)] cut_start: Option<f64>,
    #[serde(default)] cut_end: Option<f64>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Plan {
    version: u32, description: SourceDescription, fingerprint: String,
    input_parts: bool, inputs: Vec<Identity>, units: Vec<Unit>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Artifact { version: u32, plan_id: String, unit_id: String, file: String, bytes: u64, sha256: String, duration: f64 }
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Publication { artifact: Artifact, partial: String }

fn bare(reference: &CloudSourceRef) -> CloudSourceRef {
    CloudSourceRef { provider: reference.provider.clone(), recording_id: reference.recording_id.clone(), part_index: None, plan_id: None, unit_id: None }
}
fn check(context: &SourceResolveContext) -> Result<(), String> {
    if context.cancel.load(Ordering::Acquire) { Err("CLOUD_SOURCE_PAUSED:업로드 준비를 일시정지했습니다. 원본은 유지됩니다.".into()) } else { Ok(()) }
}
fn progress(context: &SourceResolveContext, phase: &str, bytes: Option<(u64,u64)>, seconds: Option<(f64,f64)>) {
    (context.progress)(super::super::SourcePreparationProgress { phase: phase.into(), completed_bytes: bytes.map(|v| v.0), total_bytes: bytes.map(|v| v.1), completed_seconds: seconds.map(|v| v.0), total_seconds: seconds.map(|v| v.1) });
}
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn id(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes); let mut out = [0u8;16]; out.copy_from_slice(&hash[..16]);
    out[6] = (out[6] & 15) | 0x80; out[8] = (out[8] & 63) | 0x80;
    uuid::Uuid::from_bytes(out).to_string()
}
fn uuid(value: &str) -> bool { uuid::Uuid::parse_str(value).is_ok_and(|v| v.to_string() == value) }
fn hash_text(value: &str) -> bool { value.len() == 64 && value.bytes().all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase()) }
fn identity(file: &File, duration: f64, offset: f64, sha: Option<String>) -> Result<Identity, String> {
    let metadata = file.metadata().map_err(|_| "영상 파일을 확인하지 못했습니다.")?;
    source_bounds(duration, offset, metadata.len())?;
    Ok(Identity { bytes: metadata.len(), modified_nanos: metadata.modified().map_err(|_| "영상 시각을 확인하지 못했습니다.")?.duration_since(UNIX_EPOCH).map_err(|_| "영상 시각이 올바르지 않습니다.")?.as_nanos(), digest: sha, duration, offset })
}
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    bounded_cancel(path,limit,None)
}
fn bounded_cancel(path: &Path, limit: u64, context: Option<&SourceResolveContext>) -> Result<Vec<u8>, String> {
    safe_path(path)?;
    let mut file = OpenOptions::new().read(true).open(path).map_err(|_| "업로드 준비 기록을 읽지 못했습니다.")?;
    if !file.metadata().map_err(|_| "준비 기록을 확인하지 못했습니다.")?.is_file() { return Err("준비 기록 형식이 올바르지 않습니다.".into()); }
    let mut data = Vec::new();let mut block=[0u8;64*1024];
    loop {
        if let Some(context)=context { check(context)?; }
        let count=file.read(&mut block).map_err(|_| "준비 기록을 읽지 못했습니다.")?;if count==0 {break;}
        if (data.len()+count) as u64>limit {return Err("준비 기록의 크기 제한을 초과했습니다.".into());}
        data.extend_from_slice(&block[..count]);
    }
    Ok(data)
}
fn proof_hash(path: &Path, expected: &str, recording: &str, context: Option<&SourceResolveContext>) -> Result<String, String> {
    if !hash_text(expected) { return Err("녹화 검증 기록이 없습니다.".into()); }
    let data = bounded_cancel(path, 64 * 1024 * 1024,context)?;
    if digest(&data) != expected { return Err("녹화 검증 기록이 변경됐습니다.".into()); }
    let value: serde_json::Value = serde_json::from_slice(&data).map_err(|_| "녹화 검증 기록이 올바르지 않습니다.")?;
    let sha = value["mediaSha256"].as_str().filter(|v| hash_text(v)).ok_or("영상 검증 해시가 없습니다.")?;
    if value["recordingId"].as_str() != Some(recording) { return Err("녹화 검증 항목이 일치하지 않습니다.".into()); }
    Ok(sha.into())
}
fn inspect(browser: &DeferredBrowser, platform: &PlatformMediaService, reference: &CloudSourceRef, force_parts: bool, context: Option<&SourceResolveContext>) -> Result<Inputs, String> {
    if let Some(context)=context {check(context)?;}
    validate_reference(reference)?;
    let reference = bare(reference);
    let mut coverage = SourceCoverage { state: "complete".into(), expected_segments: None, covered_segments: None, missing_from_segment: None, reason: None, capture_warning: None };
    let mut entries = Vec::new();
    let (title, parts, lease, store, generation): (String, bool, Box<dyn Send+Sync>, _, String) = if reference.provider != "chzzk" {
        let held = platform.upload_source(&reference.provider, &reference.recording_id)?;
        coverage.capture_warning = held.item.capture_outcome.as_ref().and_then(|outcome| outcome.warning()).map(str::to_owned);
        let file = pin_file(&held.file)?; let path = held_path(&file)?;
        let sha = if hash_text(&held.item.sha256) { Some(held.item.sha256.clone()) } else { None };
        let info = identity(&file, held.item.duration, 0.0, sha)?;
        entries.push(Input { file, path, identity: info });
        (held.item.title, false, Box::new(held.lease), None, String::new())
    } else {
        let mut selector = reference.clone(); if force_parts { selector.part_index = Some(0); }
        let (store, held) = chzzk_source_cancellable(browser, &selector,context.map(|v|v.cancel.as_ref()))?;
        let record = &held.recording;
        let count = record.segment_count;
        coverage.expected_segments = Some(count);
        if record.ending.as_ref().is_some_and(|e| e.reason == "checking") || record.partial.is_some() {
            coverage.state = "incomplete".into();
            coverage.reason = Some("녹화 종료 또는 마지막 조각이 아직 확정되지 않았습니다. 원본 처리가 끝난 뒤 다시 시도해 주세요.".into());
        }
        if held.parts.is_empty() {
            let file = pin_file(&held.media)?; let path = held_path(&file)?;
            let cleanup = record.merge.as_ref().and_then(|m| m.source_cleanup.as_ref()).ok_or("완성본의 검증 기록이 없습니다.")?;
            let sha = proof_hash(&path.parent().ok_or("영상 보관 위치 오류")?.join(&cleanup.proof_file), &cleanup.proof_sha256, &record.id,context)?;
            let info = identity(&file, held.duration, 0.0, Some(sha))?;
            entries.push(Input { file, path, identity: info }); coverage.covered_segments = Some(count);
        } else {
            let covered = held.parts.last().map_or(0, |(_, p)| p.end);
            coverage.covered_segments = Some(covered);
            if covered != count {
                coverage.state = "incomplete".into(); coverage.missing_from_segment = Some(covered);
                coverage.reason = Some(format!("마지막 {}개 원본 조각의 병합 검증이 끝나지 않았습니다. 앞부분만 업로드하지 않습니다.", count.saturating_sub(covered)));
            }
            for (file, part) in &held.parts {
                if let Some(context)=context {check(context)?;}
                let file = pin_file(file)?; let path = held_path(&file)?;
                let sha = proof_hash(&path.parent().ok_or("구간 보관 위치 오류")?.join(&part.proof_file), &part.proof_sha256, &record.id,context)?;
                let info = identity(&file, part.duration, part.offset, Some(sha))?;
                if info.bytes != part.bytes { return Err("녹화 구간 크기가 변경됐습니다.".into()); }
                entries.push(Input { file, path, identity: info });
            }
        }
        let generation = format!("{}:{}:{:.9}", count, record.bytes_written, record.duration_seconds);
        (record.title.clone(), !held.parts.is_empty(), Box::new(held.storage_lease.ok_or("녹화 보호 상태 오류")?), Some(store), generation)
    };
    if entries.is_empty() || entries.len() > 4096 { return Err("검증된 녹화 구간이 없습니다.".into()); }
    let fingerprint = digest(&serde_json::to_vec(&(reference.clone(), &generation, entries.iter().map(|v| &v.identity).collect::<Vec<_>>())).map_err(|_| "원본 식별 오류")?);
    Ok(Inputs { reference, title, entries, coverage, parts, fingerprint, lease, store })
}
fn total(inputs: &Inputs) -> (f64,u64) {
    (inputs.entries.last().map_or(0.0, |v| v.identity.offset + v.identity.duration), inputs.entries.iter().map(|v| v.identity.bytes).sum())
}
fn base(inputs: &Inputs) -> SourceDescription {
    let (duration, bytes) = total(inputs);
    SourceDescription { source: inputs.reference.clone(), title: inputs.title.clone(), plan_id: None, coverage: inputs.coverage.clone(), preparation_required: false, duration_seconds: duration, bytes, parts: Vec::new() }
}
fn groups(inputs: &[Identity]) -> Result<Vec<Unit>, String> {
    let mut result = Vec::new(); let mut first = 0; let mut seconds = 0.0; let mut bytes = 0u64;
    for (index, input) in inputs.iter().enumerate() {
        if input.duration > MAX_SECONDS || estimated(input.bytes) > MAX_BYTES { return Err("큰 단일 영상 구간을 나누는 준비가 필요합니다.".into()); }
        if index > first && (seconds + input.duration > MAX_SECONDS || estimated(bytes.saturating_add(input.bytes)) > MAX_BYTES) {
            result.push(Unit { first, end: index, cut_start: None, cut_end: None }); first = index; seconds = 0.0; bytes = 0;
        }
        if index > 0 && (input.offset - (inputs[index-1].offset + inputs[index-1].duration)).abs() > 0.001 { return Err("녹화 구간 사이의 시간축이 연속적이지 않습니다.".into()); }
        seconds += input.duration; bytes = bytes.checked_add(input.bytes).ok_or("영상 크기 제한 초과")?;
    }
    result.push(Unit { first, end: inputs.len(), cut_start: None, cut_end: None }); Ok(result)
}
fn estimated(bytes: u64) -> u64 { bytes.saturating_add(bytes / 20).saturating_add(64 * 1024 * 1024) }
fn build(inputs: &Inputs, units: Vec<Unit>) -> Result<Plan, String> {
    let plan_id = id(&serde_json::to_vec(&(4u32, &inputs.fingerprint, MAX_SECONDS, MAX_BYTES, &units)).map_err(|_| "계획 식별 오류")?);
    let mut description = base(inputs); description.plan_id = Some(plan_id.clone());
    for (index, unit) in units.iter().enumerate() {
        let selected = inputs.entries.get(unit.first..unit.end).filter(|v| !v.is_empty()).ok_or("영상 구간 계획이 올바르지 않습니다.")?;
        let first = &selected[0].identity;
        let duration = match (unit.cut_start, unit.cut_end) { (Some(start), Some(end)) => end-start, (None,None) => selected.iter().map(|v| v.identity.duration).sum(), _ => return Err("영상 분할 계획 오류".into()) };
        let offset = first.offset + unit.cut_start.unwrap_or(0.0);
        let direct = selected.len() == 1 && unit.cut_start.is_none();
        let bytes = if unit.cut_start.is_some() { estimated(((duration / first.duration) * first.bytes as f64).ceil() as u64) } else if direct { first.bytes } else { estimated(selected.iter().map(|v| v.identity.bytes).sum()) };
        source_bounds(duration, offset, bytes)?;
        let mut source = inputs.reference.clone(); source.plan_id = Some(plan_id.clone()); source.unit_id = Some(id(format!("{plan_id}:{index}").as_bytes()));
        description.parts.push(SourcePart { source, unit_index: index as u32, part_index: None, duration_seconds: duration, media_offset_seconds: offset, bytes, kind: if direct { "direct" } else { "derived" }.into() });
    }
    Ok(Plan { version: 1, description, fingerprint: inputs.fingerprint.clone(), input_parts: inputs.parts, inputs: inputs.entries.iter().map(|v| v.identity.clone()).collect(), units })
}
fn cheap_plan(inputs: &Inputs) -> Result<Option<Plan>, String> {
    if inputs.coverage.state != "complete" { return Ok(None); }
    let identities: Vec<_> = inputs.entries.iter().map(|v| v.identity.clone()).collect();
    let (duration, bytes) = total(inputs);
    if identities.len() == 1 && duration <= 43_200.0 && bytes <= PROVIDER_BYTES {
        return build(inputs, vec![Unit { first:0,end:1,cut_start:None,cut_end:None }]).map(Some);
    }
    if identities.len() > 1 { return build(inputs, groups(&identities)?).map(Some); }
    Ok(None)
}
pub(super) fn describe(browser: &DeferredBrowser, platform: &PlatformMediaService, _data_dir: &Path, reference: &CloudSourceRef) -> Result<SourceDescription,String> {
    let inputs = inspect(browser, platform, reference, false,None)?;
    if let Some(plan) = cheap_plan(&inputs)? { return Ok(plan.description); }
    let mut description = base(&inputs); description.preparation_required = description.coverage.state == "complete";
    Ok(description)
}
fn root(data_dir: &Path) -> PathBuf { data_dir.join("cloud-recording").join("plans") }
fn safe_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute() || path.components().any(|p| matches!(p, std::path::Component::ParentDir)) { return Err("업로드 보관 경로가 올바르지 않습니다.".into()); }
    for parent in path.ancestors() {
        let Ok(meta) = fs::symlink_metadata(parent) else { continue; };
        #[cfg(windows)] { use std::os::windows::fs::MetadataExt; if meta.file_attributes() & 0x400 != 0 { return Err("연결된 보관 경로는 사용할 수 없습니다.".into()); } }
        if meta.file_type().is_symlink() { return Err("연결된 보관 경로는 사용할 수 없습니다.".into()); }
    } Ok(())
}
fn save(data_dir: &Path, plan: &Plan) -> Result<(),String> {
    let root = root(data_dir); safe_path(&root)?; fs::create_dir_all(&root).map_err(|_| "업로드 계획 폴더를 만들지 못했습니다.")?; safe_path(&root)?;
    let id = plan.description.plan_id.as_ref().ok_or("계획 식별자 없음")?; let path = root.join(format!("{id}.json"));
    let data = serde_json::to_vec(plan).map_err(|_| "업로드 계획 저장 오류")?;
    if data.len() as u64 > MAX_PLAN_BYTES { return Err("업로드 계획 크기 제한을 초과했습니다.".into()); }
    if path.exists() { let old = load(data_dir, id)?; if old.fingerprint != plan.fingerprint || serde_json::to_vec(&old.units).ok() != serde_json::to_vec(&plan.units).ok() { return Err("기존 업로드 계획과 원본이 일치하지 않습니다.".into()); } return Ok(()); }
    let temporary = root.join(format!("{id}.{}.pending", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new().write(true).create_new(true).open(&temporary).map_err(|_| "업로드 계획을 저장하지 못했습니다.")?;
    file.write_all(&data).and_then(|_| file.sync_all()).map_err(|_| "업로드 계획을 확정하지 못했습니다.")?; drop(file);
    fs::rename(&temporary, &path).map_err(|_| "업로드 계획을 확정하지 못했습니다.".to_string())
}
fn load(data_dir: &Path, plan_id: &str) -> Result<Plan,String> {
    if !uuid(plan_id) { return Err("업로드 계획 식별자가 올바르지 않습니다.".into()); }
    let plan: Plan = serde_json::from_slice(&bounded(&root(data_dir).join(format!("{plan_id}.json")), MAX_PLAN_BYTES)?).map_err(|_| "업로드 계획이 올바르지 않습니다.")?;
    let d = &plan.description;
    if plan.version != 1 || d.plan_id.as_deref() != Some(plan_id) || d.coverage.state != "complete" || d.parts.is_empty() || d.parts.len() != plan.units.len() || d.parts.len() > 2048 || !hash_text(&plan.fingerprint) { return Err("업로드 계획을 검증하지 못했습니다.".into()); }
    validate_reference(&d.source)?;
    if d.source != bare(&d.source) || plan.inputs.is_empty() || plan.inputs.len() > 4096
        || id(&serde_json::to_vec(&(4u32,&plan.fingerprint,MAX_SECONDS,MAX_BYTES,&plan.units)).map_err(|_|"계획 식별 오류")?) != plan_id {
        return Err("업로드 계획의 원본 식별자가 일치하지 않습니다.".into());
    }
    let mut offset = 0.0;
    for (index, p) in d.parts.iter().enumerate() {
        validate_reference(&p.source)?; source_bounds(p.duration_seconds,p.media_offset_seconds,p.bytes)?;
        if p.unit_index as usize != index || bare(&p.source) != d.source || p.source.plan_id.as_deref() != Some(plan_id) || p.source.unit_id.as_deref() != Some(&id(format!("{plan_id}:{index}").as_bytes())) || (p.media_offset_seconds-offset).abs() > 0.001 { return Err("업로드 구간의 시간축이 올바르지 않습니다.".into()); }
        let unit = &plan.units[index];
        let inputs = plan.inputs.get(unit.first..unit.end).filter(|v|!v.is_empty()).ok_or("계획의 원본 범위 오류")?;
        let (start,length,direct) = match (unit.cut_start,unit.cut_end) {
            (None,None)=>(inputs[0].offset,inputs.iter().map(|v|v.duration).sum::<f64>(),inputs.len()==1),
            (Some(start),Some(end)) if inputs.len()==1 && start.is_finite() && end.is_finite() && start>=0.0 && end>start && end<=inputs[0].duration+0.001 => (inputs[0].offset+start,end-start,false),
            _=>return Err("계획의 영상 분할 범위 오류".into()),
        };
        if (p.media_offset_seconds-start).abs()>0.001 || (p.duration_seconds-length).abs()>0.001 || p.kind != if direct {"direct"}else{"derived"} { return Err("계획의 영상 구간 정보가 변조되었습니다.".into()); }
        offset += p.duration_seconds;
    }
    if (offset-d.duration_seconds).abs() > 0.001 { return Err("업로드 계획에 누락된 구간이 있습니다.".into()); }
    Ok(plan)
}
pub(super) fn get(data_dir: &Path, plan_id: &str) -> Result<SourceDescription,String> { Ok(load(data_dir,plan_id)?.description) }
/// Validates persisted upload identity only. Source video and derivative artifacts
/// are deliberately not opened by a remote chat read.
pub(super) fn replay_binding(data_dir: &Path, reference: &CloudSourceRef, sha256: &str,
    bytes: u64, duration: f64, offset: f64,
) -> Result<(SourceDescription, Vec<super::super::replay::CloudReplayInput>, Option<String>, bool), String> {
    validate_reference(reference)?;
    let plan = load(data_dir, reference.plan_id.as_deref().ok_or("업로드 계획 없음")?)?;
    let index = plan.description.parts.iter().position(|part| part.source == *reference).ok_or("계획에 없는 보관 구간입니다.")?;
    let part = &plan.description.parts[index];
    if !hash_text(sha256) || bytes == 0 || !duration.is_finite() || !offset.is_finite()
        || (part.duration_seconds - duration).abs() > 0.001
        || (part.media_offset_seconds - offset).abs() > 0.001 {
        return Err("보관 영상과 업로드 계획의 구간이 다릅니다.".into());
    }
    if reference.provider != "chzzk" && (plan.input_parts || plan.inputs.len() != 1 || plan.inputs[0].offset != 0.0) {
        return Err("원본 녹화의 시간축을 확인하지 못했습니다.".into());
    }
    let mut inputs = Vec::with_capacity(plan.inputs.len());
    for (index, input) in plan.inputs.iter().enumerate() {
        source_bounds(input.duration, input.offset, input.bytes)?;
        if input.offset > 1_000_000_000.0 || input.modified_nanos == 0 {
            return Err("원본 녹화의 검증 정보가 올바르지 않습니다.".into());
        }
        let digest = input.digest.as_ref().filter(|value| hash_text(value)).ok_or("원본 영상의 검증 해시가 없습니다.")?;
        inputs.push(super::super::replay::CloudReplayInput {
            index: index as u32, part_index: plan.input_parts.then_some(index as u32),
            sha256: digest.clone(), bytes: input.bytes, duration_seconds: input.duration,
            media_offset_seconds: input.offset, modified_nanos: Some(input.modified_nanos),
        });
    }
    if part.kind == "direct" {
        let input = &inputs[plan.units[index].first];
        if input.sha256 != sha256 || input.bytes != bytes || part.bytes != bytes {
            return Err("보관 영상과 원본 검증 정보가 다릅니다.".into());
        }
    }
    Ok((plan.description, inputs, Some(plan.fingerprint), plan.input_parts))
}
pub(super) fn prepare(browser: &DeferredBrowser, platform: &PlatformMediaService, data_dir: &Path, reference: &CloudSourceRef, context: &SourceResolveContext) -> Result<Vec<CloudSourceRef>,String> {
    check(context)?; progress(context,"checking",None,None);
    let inputs = inspect(browser,platform,reference,false,Some(context))?;
    if inputs.coverage.state != "complete" { return Err(format!("CLOUD_SOURCE_COVERAGE:{}",inputs.coverage.reason.clone().unwrap_or("원본 녹화에 누락된 구간이 있습니다.".into()))); }
    let plan = if let Some(plan) = cheap_plan(&inputs)? { plan } else {
        let input = &inputs.entries[0];
        let tools = platform.cloud_media_tools();
        let callback = context.progress.clone(); let duration = input.identity.duration;
        let cuts = browser_merge::cloud_upload_cuts(&tools, &input.path, duration, input.identity.bytes, MAX_SECONDS, MAX_BYTES / 20 * 19, &context.cancel,
            Arc::new(move |at| callback(super::super::SourcePreparationProgress { phase:"checking".into(),completed_bytes:None,total_bytes:None,completed_seconds:Some(at.clamp(0.0,duration)),total_seconds:Some(duration) }))).map_err(|e| e.message)?;
        build(&inputs, cuts.windows(2).map(|cut| Unit{first:0,end:1,cut_start:Some(cut[0]),cut_end:Some(cut[1])}).collect())?
    };
    check(context)?;
    if let Some(store) = &inputs.store { store.preserve_recording_sources(&reference.recording_id).map_err(|e| e.message)?; }
    save(data_dir,&plan)?;
    Ok(plan.description.parts.into_iter().map(|p| p.source).collect())
}
fn file_hash(file: &mut File, context: &SourceResolveContext) -> Result<String,String> {
    let total = file.metadata().map_err(|_| "영상 크기 확인 오류")?.len(); let mut done = 0u64;
    let budget = crate::storage_io_budget::BulkReadBudget::for_file(file);
    file.seek(SeekFrom::Start(0)).map_err(|_| "영상 읽기 오류")?;
    let mut hash = Sha256::new(); let mut block = [0u8;128*1024];
    loop { check(context)?; let started = std::time::Instant::now(); let n = file.read(&mut block).map_err(|_| "영상 검증 읽기 오류")?; if n == 0 { break; }
        hash.update(&block[..n]); done += n as u64;
        if !budget.account(n,started.elapsed(),||context.cancel.load(Ordering::Acquire)) { return Err("업로드 준비를 일시정지했습니다.".into()); }
        progress(context,"verifying",Some((done,total)),None);
    }
    file.seek(SeekFrom::Start(0)).map_err(|_| "영상 읽기 오류")?; Ok(format!("{:x}",hash.finalize()))
}
pub(super) fn resolve(browser: &DeferredBrowser, platform: &PlatformMediaService, data_dir: &Path, reference: &CloudSourceRef, context: &SourceResolveContext) -> Result<CloudSource,String> {
    let plan = load(data_dir,reference.plan_id.as_deref().ok_or("업로드 계획 없음")?)?;
    let index = plan.description.parts.iter().position(|p| p.source == *reference).ok_or("계획에 없는 업로드 구간입니다.")?;
    let mut inputs = inspect(browser,platform,&plan.description.source,plan.input_parts,Some(context))?;
    if inputs.fingerprint != plan.fingerprint || inputs.coverage.state != "complete" || inputs.entries.iter().map(|v| &v.identity).ne(plan.inputs.iter()) { return Err("계획 이후 원본이 변경됐습니다. 기존 전송을 다른 파일로 이어서 보내지 않습니다.".into()); }
    let view = &plan.description.parts[index]; let unit = &plan.units[index];
    let file = if view.kind == "direct" {
        let mut file = pin_file(&inputs.entries[unit.first].file)?;
        if let Some(expected) = &inputs.entries[unit.first].identity.digest {
            if file_hash(&mut file,context)? != *expected { return Err("완성본의 검증 해시가 일치하지 않습니다.".into()); }
        }
        file
    } else {
        materialize(&mut inputs, &plan, index, &platform.cloud_media_tools(), context)?
    };
    check(context)?;
    Ok(CloudSource { file,source:reference.clone(),title:inputs.title,duration_seconds:view.duration_seconds,media_offset_seconds:view.media_offset_seconds,lease:inputs.lease })
}
fn materialize(inputs: &mut Inputs, plan: &Plan, index: usize, tools: &MediaTools, context: &SourceResolveContext) -> Result<File,String> {
    let view = &plan.description.parts[index]; let unit = &plan.units[index];
    let plan_id = plan.description.plan_id.as_ref().ok_or("계획 없음")?; let unit_id = view.source.unit_id.as_ref().ok_or("구간 없음")?;
    let parent = inputs.entries[0].path.parent().ok_or("영상 위치 오류")?;
    let root = parent.join(".atsumi-cloud-upload").join(plan_id); safe_path(&root)?;
    fs::create_dir_all(&root).map_err(|_| "CLOUD_SOURCE_IO:업로드 준비 공간을 만들지 못했습니다.")?; safe_path(&root)?;
    let _directory_pins = crate::streaming::browser_store::cleanup::pin_directories(&root).map_err(|e| e.message)?;
    let receipt = root.join(format!("{unit_id}.json"));
    recover_publication(&root,plan_id,unit_id,view.duration_seconds,context)?;
    if receipt.exists() {
        let saved: Artifact = serde_json::from_slice(&bounded(&receipt, 16*1024)?).map_err(|_| "준비한 영상 검증 기록 오류")?;
        if saved.version != 1 || saved.plan_id != *plan_id || saved.unit_id != *unit_id || saved.file != format!("{unit_id}.mp4") || saved.bytes==0 || saved.bytes>MAX_BYTES || !hash_text(&saved.sha256) || !saved.duration.is_finite() || (saved.duration-view.duration_seconds).abs()>0.001 { return Err("준비한 영상의 식별 정보가 다릅니다.".into()); }
        let path = root.join(&saved.file); safe_path(&path)?;
        let file = File::open(path).map_err(|_| "준비한 영상이 없어 중단했습니다. 기존 전송은 다른 파일로 이어지지 않습니다.")?;
        let mut file = pin_file(&file)?;
        if file.metadata().map_err(|_| "준비 영상 크기 오류")?.len()!=saved.bytes || file_hash(&mut file,context)? != saved.sha256 { return Err("준비한 영상이 변경됐습니다.".into()); }
        return Ok(file);
    }
    let expected = view.bytes.min(MAX_BYTES);
    if fs2::available_space(&root).map_err(|_| "업로드 준비 공간 확인 오류")? < expected.saturating_add(RESERVE) { return Err(format!("CLOUD_SOURCE_SPACE:업로드 준비 공간이 부족합니다. 약 {} MiB와 512 MiB 여유가 필요하며 기존 파일은 보존됩니다.",expected.div_ceil(1024*1024))); }
    let mut sources = Vec::new();
    for input in inputs.entries.get_mut(unit.first..unit.end).ok_or("업로드 구간 범위 오류")? {
        check(context)?;
        if let Some(expected) = &input.identity.digest { if file_hash(&mut input.file,context)? != *expected { return Err("원본 영상의 검증 해시가 일치하지 않습니다.".into()); } }
        sources.push((input.path.clone(),input.identity.duration));
    }
    let output = root.join(format!("{unit_id}.{}.partial",uuid::Uuid::new_v4()));
    let callback = context.progress.clone(); let duration = view.duration_seconds;
    browser_merge::cloud_upload_remux(tools,&sources,&output,unit.cut_start.zip(unit.cut_end),duration,MAX_BYTES,&context.cancel,
        Arc::new(move |phase,at| callback(super::super::SourcePreparationProgress { phase:phase.into(),completed_bytes:None,total_bytes:None,completed_seconds:Some(at.clamp(0.0,duration)),total_seconds:Some(duration) }))).map_err(|e| if e.code=="BROWSER_MERGE_SPACE" {format!("CLOUD_SOURCE_SPACE:{}",e.message)} else {e.message})?;
    check(context)?;
    let file = File::open(&output).map_err(|_| "준비한 영상 읽기 오류")?; let mut file = pin_file(&file)?;
    let bytes = file.metadata().map_err(|_| "준비한 영상 크기 오류")?.len(); let sha = file_hash(&mut file,context)?; drop(file);
    let final_path = root.join(format!("{unit_id}.mp4"));
    if final_path.exists() { return Err("검증 기록 없는 기존 준비 영상이 있습니다. 원본과 임시 파일을 보존했습니다.".into()); }
    let artifact = Artifact { version:1,plan_id:plan_id.clone(),unit_id:unit_id.clone(),file:format!("{unit_id}.mp4"),bytes,sha256:sha,duration };
    let pending=Publication {artifact,partial:output.file_name().and_then(|v|v.to_str()).ok_or("준비 영상 이름 오류")?.into()};
    let staged=root.join(format!("{unit_id}.publication.json"));
    write_new_json(&staged,&pending)?;
    recover_publication(&root,plan_id,unit_id,duration,context)?;
    pin_file(&File::open(&final_path).map_err(|_| "준비 영상 열기 오류")?)
}
fn write_new_json(path:&Path,value:&impl Serialize)->Result<(),String> {
    let data=serde_json::to_vec(value).map_err(|_|"준비 영상 검증 기록 오류")?;
    let pending=path.with_extension(format!("{}.pending",uuid::Uuid::new_v4()));
    let mut file=OpenOptions::new().write(true).create_new(true).open(&pending).map_err(|_|"CLOUD_SOURCE_IO:준비 영상 검증 기록 저장 오류")?;
    file.write_all(&data).and_then(|_|file.sync_all()).map_err(|_|"CLOUD_SOURCE_IO:준비 영상 검증 기록 확정 오류")?;drop(file);
    if path.exists() {return Err("기존 검증 기록을 덮어쓰지 않았습니다.".into());}
    fs::rename(pending,path).map_err(|_|"CLOUD_SOURCE_IO:준비 영상 검증 기록 확정 오류".into())
}
fn recover_publication(root:&Path,plan_id:&str,unit_id:&str,duration:f64,context:&SourceResolveContext)->Result<(),String> {
    let receipt=root.join(format!("{unit_id}.json"));if receipt.exists() {return Ok(());}
    let staged=root.join(format!("{unit_id}.publication.json"));if !staged.exists() {return Ok(());}
    let publication:Publication=serde_json::from_slice(&bounded_cancel(&staged,16*1024,Some(context))?).map_err(|_|"업로드 영상 확정 기록 오류")?;
    let artifact=&publication.artifact;
    let suffix=publication.partial.strip_prefix(&format!("{unit_id}.")).and_then(|v|v.strip_suffix(".partial"));
    if artifact.version!=1||artifact.plan_id!=plan_id||artifact.unit_id!=unit_id||artifact.file!=format!("{unit_id}.mp4")||artifact.bytes==0||artifact.bytes>MAX_BYTES||!hash_text(&artifact.sha256)||!artifact.duration.is_finite()||(artifact.duration-duration).abs()>0.001||!suffix.is_some_and(uuid) {return Err("업로드 영상 확정 기록이 일치하지 않습니다.".into());}
    let final_path=root.join(&artifact.file);let partial=root.join(&publication.partial);
    let source=if final_path.exists(){&final_path}else{&partial};safe_path(source)?;
    let file=File::open(source).map_err(|_|"확정 중이던 업로드 영상이 없습니다. 원본은 유지됩니다.")?;let mut file=pin_file(&file)?;
    if file.metadata().map_err(|_|"업로드 영상 확인 오류")?.len()!=artifact.bytes||file_hash(&mut file,context)?!=artifact.sha256 {return Err("확정 중이던 업로드 영상이 변경됐습니다.".into());}drop(file);
    check(context)?;if !final_path.exists(){fs::rename(&partial,&final_path).map_err(|_|"CLOUD_SOURCE_IO:업로드 영상 확정 오류")?;}
    write_new_json(&receipt,artifact)?;Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context(cancelled:bool)->SourceResolveContext { SourceResolveContext{cancel:Arc::new(std::sync::atomic::AtomicBool::new(cancelled)),progress:Arc::new(|_|{})} }
    fn input(seconds:f64,offset:f64,bytes:u64)->Identity { Identity{bytes,modified_nanos:1,digest:None,duration:seconds,offset} }
    #[test] fn capture_parts_form_bounded_larger_units() {
        let source:Vec<_>=(0..804).map(|i|input(60.0,i as f64*60.0,48*1024*1024)).collect();
        let units=groups(&source).unwrap(); assert!(units.len()<10); assert_eq!(units[0].first,0); assert_eq!(units.last().unwrap().end,804);
        for pair in units.windows(2) { assert_eq!(pair[0].end,pair[1].first); }
    }
    #[test] fn grouping_rejects_gaps_and_oversized_atomic_input() {
        assert!(groups(&[input(60.0,0.0,100),input(60.0,61.0,100)]).is_err());
        assert!(groups(&[input(43201.0,0.0,100)]).is_err());
    }
    #[test] fn identities_are_stable_valid_and_domain_separated() {
        assert_eq!(id(b"source:unit:1"),id(b"source:unit:1")); assert_ne!(id(b"source:unit:1"),id(b"source:unit:2")); assert!(uuid(&id(b"x")));
    }
    #[test] fn interrupted_derivative_publication_recovers_before_or_after_rename() {
        for already_renamed in [false,true] {
            let directory=tempfile::tempdir().unwrap();let root=directory.path();
            let plan_id=uuid::Uuid::new_v4().to_string();let unit_id=uuid::Uuid::new_v4().to_string();
            let partial=format!("{unit_id}.{}.partial",uuid::Uuid::new_v4());
            let final_name=format!("{unit_id}.mp4");let bytes=b"verified upload derivative";
            fs::write(root.join(if already_renamed{&final_name}else{&partial}),bytes).unwrap();
            let publication=Publication{artifact:Artifact{version:1,plan_id:plan_id.clone(),unit_id:unit_id.clone(),file:final_name.clone(),bytes:bytes.len() as u64,sha256:digest(bytes),duration:1.0},partial};
            write_new_json(&root.join(format!("{unit_id}.publication.json")),&publication).unwrap();
            recover_publication(root,&plan_id,&unit_id,1.0,&context(false)).unwrap();
            assert_eq!(fs::read(root.join(&final_name)).unwrap(),bytes);
            let receipt:Artifact=serde_json::from_slice(&fs::read(root.join(format!("{unit_id}.json"))).unwrap()).unwrap();assert_eq!(receipt.sha256,digest(bytes));
            // Idempotent restart, and the publication evidence is retained.
            recover_publication(root,&plan_id,&unit_id,1.0,&context(false)).unwrap();
            assert!(root.join(format!("{unit_id}.publication.json")).exists());
        }
    }
    #[test] fn publication_rejects_changed_bytes_and_parent_escape() {
        let directory=tempfile::tempdir().unwrap();let root=directory.path();
        let plan_id=uuid::Uuid::new_v4().to_string();let unit_id=uuid::Uuid::new_v4().to_string();
        let partial=format!("{unit_id}.{}.partial",uuid::Uuid::new_v4());fs::write(root.join(&partial),b"changed").unwrap();
        let publication=Publication{artifact:Artifact{version:1,plan_id:plan_id.clone(),unit_id:unit_id.clone(),file:format!("{unit_id}.mp4"),bytes:7,sha256:digest(b"correct"),duration:1.0},partial};
        write_new_json(&root.join(format!("{unit_id}.publication.json")),&publication).unwrap();
        assert!(recover_publication(root,&plan_id,&unit_id,1.0,&context(false)).is_err());
        assert!(!root.join(format!("{unit_id}.mp4")).exists());
        assert!(!root.join(format!("{unit_id}.json")).exists());
        assert!(safe_path(&root.join("../escape")).is_err());
    }
    #[test] fn preparation_cancel_is_checked_before_reading_large_proof() {
        let directory=tempfile::tempdir().unwrap();let file=directory.path().join("proof");fs::write(&file,b"proof").unwrap();
        assert!(bounded_cancel(&file,1024,Some(&context(true))).unwrap_err().starts_with("CLOUD_SOURCE_PAUSED:"));
    }
}
