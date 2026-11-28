//! Exact selected SOOP source + existing isolated vault preflight. No remote calls.
//! Register as a test child of cloud_recording/tests.rs only after parent review.
use super::super::*;
use crate::platform_media::MediaJob;
use crate::streaming::browser_store;
use serde_json::{json, Value};
use std::{collections::HashSet, fs, io::{Read, Seek, SeekFrom, Write}, path::{Path, PathBuf}};

const PROFILE: &str = "local.atsumi.platformqa.66a77b3fe4ab47cf8a3bb4fcfa86b510";
const SOURCE: &str = "3bae70dd-ca33-401b-aa95-a508bab9ffc4";
const CAPTURE: &str = "0d19e1f866b84ef8a2edef3980bb923b";
const BYTES: u64 = 17_633_383;
const SHA: &str = "08c0087446f4485dd11151f10f31b3396409aaad3055ae0ff02fa153e8c9a106";
const CATALOG_SHA: &str = "f1d987d6f78644f79587bbf5ec08a3567fd46e37f9eeb5102fe1bb04f284ed9f";
const MANIFEST_SHA: &str = "bf892399d2cdd62a0bb856c6975cbb5a23edfcef9cc00f28c4e809b81440b978";
const OLD: [(&str, &str); 5] = [
    ("youtube", "38965dca-b875-4758-8b84-642507653bd1"),
    ("soop", "9011ac2c-8063-4e1a-a85a-3bc449879c8f"),
    ("soop", "241bc41d-d3e6-4bf9-a205-899f2c2d21d5"),
    ("chzzk", "6e82ac06ed1048c6b8201b29892ff659"),
    ("soop", "8fd00700-81d4-401f-a083-0d13fcf286ea"),
];
type Checked<T> = Result<T, String>;
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn plain(path: &Path) -> Checked<()> {
    use std::os::windows::fs::MetadataExt;
    for part in path.ancestors() {
        let meta = fs::symlink_metadata(part).map_err(|_| "QA_PATH")?;
        if meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0 { return Err("QA_LINK".into()); }
    } Ok(())
}
fn read(path: &Path, limit: u64) -> Checked<Vec<u8>> {
    plain(path)?;
    let path = fs::canonicalize(path).map_err(|_| "QA_CANONICAL")?; plain(&path)?;
    let mut file = browser_store::cleanup::verification_guard(&path).map_err(|_| "QA_READ_PIN")?;
    let length = file.metadata().map_err(|_| "QA_STAT")?.len();
    if length == 0 || length > limit { return Err("QA_READ_BOUND".into()); }
    let mut bytes = Vec::new(); (&mut file).take(limit + 1).read_to_end(&mut bytes).map_err(|_| "QA_READ")?;
    if bytes.len() as u64 != length { return Err("QA_READ_CHANGED".into()); } Ok(bytes)
}
struct Original { path: PathBuf, file: File, bytes: u64, sha: String, _parents: Vec<File> }
impl Original {
    fn pin(path: &Path, bytes: u64, sha: &str) -> Checked<Self> {
        plain(path)?;
        let path = fs::canonicalize(path).map_err(|_| "QA_CANONICAL")?; plain(&path)?;
        if bytes == 0 || bytes > 32 * 1024 * 1024 || sha.len() != 64 { return Err("QA_ORIGINAL_BOUND".into()); }
        let parents = browser_store::cleanup::pin_directories(path.parent().ok_or("QA_PARENT")?).map_err(|_| "QA_DIRECTORY_PIN")?;
        let file = browser_store::cleanup::verification_guard(&path).map_err(|_| "QA_FILE_PIN")?;
        let mut result = Self { path: path.to_owned(), file, bytes, sha: sha.into(), _parents: parents };
        result.check()?; Ok(result)
    }
    fn check(&mut self) -> Checked<()> {
        plain(&self.path)?;
        if self.file.metadata().map_err(|_| "QA_STAT")?.len() != self.bytes { return Err("QA_ORIGINAL_CHANGED".into()); }
        self.file.seek(SeekFrom::Start(0)).map_err(|_| "QA_SEEK")?;
        if hash_source(&mut self.file, &AtomicBool::new(false)).map_err(|_| "QA_HASH")? != self.sha { return Err("QA_ORIGINAL_CHANGED".into()); }
        Ok(())
    }
}

#[test]
fn preflight_pins_canonicalize_plain_native_paths_before_guarding() -> Checked<()> {
    let directory = std::env::temp_dir().join(format!("atsumi-private-preflight-{}", uuid::Uuid::new_v4().simple()));
    fs::create_dir(&directory).map_err(|_| "QA_TEST_DIRECTORY")?;
    let path = directory.join("fixture.bin");
    let bytes = b"plain native path";
    fs::write(&path, bytes).map_err(|_| "QA_TEST_FILE")?;
    assert_eq!(read(&path, 1024)?, bytes);
    let mut held = Original::pin(&path, bytes.len() as u64, &digest(bytes))?;
    assert_eq!(held.path, fs::canonicalize(&path).map_err(|_| "QA_TEST_CANONICAL")?);
    held.check()?;
    drop(held);
    fs::remove_file(&path).map_err(|_| "QA_TEST_CLEANUP")?;
    fs::remove_dir(&directory).map_err(|_| "QA_TEST_CLEANUP")?;
    Ok(())
}

#[test]
#[ignore = "Exact saved SOOP 118s source and native isolated-vault preflight; no network/upload/profile actions"]
fn actual_private_selected_soop_preflight_without_network() -> Checked<()> {
    if std::env::var("ATSUMI_PRIVATE_UPLOAD_PREFLIGHT_QA").as_deref() != Ok(SOURCE) { return Err("QA_OPT_IN".into()); }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().ok_or("QA_WORKSPACE")?;
    let audit = workspace.join(".runtime/requirements-audit/20261122-cloud-resume");
    let root = workspace.join(".runtime/soop-original-qa").join(CAPTURE);
    let manifest_bytes = read(&audit.join("private-upload-118-source-files.json"), 64 * 1024)?;
    if digest(&manifest_bytes) != MANIFEST_SHA { return Err("QA_MANIFEST_CHANGED".into()); }
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|_| "QA_MANIFEST")?;
    let entries = manifest["files"].as_array().filter(|v| v.len() == 37).ok_or("QA_ORIGINAL_COUNT")?;
    let mut seen = HashSet::new(); let mut originals = Vec::new();
    for entry in entries {
        let relative = entry["relative"].as_str().ok_or("QA_RELATIVE")?;
        if relative.contains('\\') || relative.len() > 240 || !Path::new(relative).components().all(|c| matches!(c, std::path::Component::Normal(_))) || !seen.insert(relative) { return Err("QA_RELATIVE".into()); }
        originals.push(Original::pin(&root.join(relative), entry["bytes"].as_u64().ok_or("QA_BYTES")?, entry["sha256"].as_str().ok_or("QA_SHA")?)?);
    }
    let source_bytes = read(&root.join(format!("data/platform-media/jobs/{SOURCE}.json")), 1024 * 1024)?;
    // Fixed source metadata belongs to the capture that already contains the real membership event.
    if digest(&source_bytes) != CATALOG_SHA { return Err("QA_SOURCE_CATALOG_HASH".into()); }
    let source: MediaJob = serde_json::from_slice(&source_bytes).map_err(|_| "QA_SOURCE_CATALOG")?;
    let item = source.library.as_ref().ok_or("QA_SOURCE_LIBRARY")?;
    let raw: Value = serde_json::from_slice(&source_bytes).map_err(|_| "QA_SOURCE_CATALOG")?;
    if source.job_id != SOURCE || source.request_id != "d31ecfbc-dfe5-473e-8c1a-be146e4acb78" || source.state != "completed" || source.revision != 16
        || source.reference.provider != "soop" || source.reference.kind != "live" || source.reference.external_id != "hinqocorv"
        || source.broadcast_id.as_deref() != Some("298642901") || item.broadcast_id != source.broadcast_id || item.reference != source.reference
        || item.library_id != SOURCE || item.revision != 16 || item.bytes != BYTES || item.duration != 118.177334 || item.sha256 != SHA || item.modified_millis != 1794872019667
        || fs::canonicalize(&source.download_root).map_err(|_| "QA_SOURCE_ROOT")? != fs::canonicalize(root.join("media")).map_err(|_| "QA_SOURCE_ROOT")?
        || raw["library"]["captureOutcome"]["state"] != "interrupted" || raw["library"]["captureOutcome"]["reasons"] != json!(["source_interrupted", "audio_coverage_tail"])
        { return Err("QA_SOURCE_IDENTITY".into()); }
    let data = PathBuf::from(std::env::var_os("APPDATA").ok_or("QA_APPDATA")?).join(PROFILE);
    plain(&data.join("youtube-original-profile"))?;
    plain(&data.join("cloud-recording/state.lock"))?;
    let ciphertext = read(&data.join("cloud-recording/state.bin"), 8 * 1024 * 1024)?;
    let mut original_vault = Original::pin(&data.join("cloud-recording/state.bin"), ciphertext.len() as u64, &digest(&ciphertext))?;
    let vault = vault::Vault::open(&data).map_err(|_| "QA_VAULT_EXCLUSIVE")?;
    let catalog = vault.load().map_err(|_| "QA_NATIVE_CATALOG")?;
    let selected = catalog.jobs.iter().filter(|job| job.view.source.provider == "soop" && job.view.source.recording_id == SOURCE).count();
    let prior_ready = catalog.jobs.len() == OLD.len() && OLD.iter().all(|(provider, id)| {
        let found = catalog.jobs.iter().filter(|job| job.view.source.provider == *provider && job.view.source.recording_id == *id).collect::<Vec<_>>();
        found.len() == 1 && found[0].view.state == "ready" && found[0].has_upload_receipt() && found[0].view.visibility == CloudVisibility::Unlisted
            && catalog.grant.as_ref().is_some_and(|grant| found[0].channel_id == grant.channel_id)
    });
    let imported = data.join(format!("platform-media/jobs/{SOURCE}.json"));
    let imported_exact = if imported.exists() { read(&imported, 1024 * 1024)? == source_bytes } else { false };
    let selected_metadata_missing = !imported.exists();
    let connected = catalog.client.is_some() && catalog.grant.is_some();
    let prior_live_count = catalog.live_receipts.len();
    drop(catalog); drop(vault);
    original_vault.check()?; for original in &mut originals { original.check()?; }
    let report = json!({
        "sourceId": SOURCE, "sourceProvider": "soop", "sourceDurationSeconds": 118.177334, "sourceBytes": BYTES, "sourceSha256": SHA,
        "sourceOriginals": 37, "originalFilesUnchanged": true, "originalCatalogUnchanged": true,
        "nativeCatalogLoaded": true, "selectedExistingJobs": selected, "selectedSourceAbsent": selected == 0,
        "knownPriorFiveReadyUnlisted": prior_ready, "priorLiveReceiptCount": prior_live_count, "uploadOAuthConfigured": connected,
        "exactSelectedMetadataRegistered": imported_exact, "selectedMetadataMissing": selected_metadata_missing,
        "websiteAuthenticated": null, "ownerChannelVerified": false, "privateVideoAccessVerified": false,
        "uploadReady": false, "requiredNext": "Fresh existing-profile website probe; explicit parent review, exact selected catalog registration if missing, then one Private upload and exact owner playback verification.",
        "timelineQuality": "receive-time-approximate", "captureOutcomePreserved": ["source_interrupted", "audio_coverage_tail"],
        "networkRequests": 0, "uploads": 0, "credentialContentsPrinted": false, "profileCopy": false, "metadataImported": false, "mediaCopied": false
    });
    let target = audit.join(format!("private-source-preflight-{}.json", uuid::Uuid::new_v4().simple()));
    let mut file = fs::OpenOptions::new().create_new(true).write(true).open(&target).map_err(|_| "QA_REPORT")?;
    file.write_all(&serde_json::to_vec_pretty(&report).map_err(|_| "QA_REPORT")?).and_then(|_| file.sync_all()).map_err(|_| "QA_REPORT")?;
    println!("QA_PRIVATE_PREFLIGHT={}", target.display());
    if selected != 0 { return Err("QA_EXISTING_SOURCE_NO_RECREATE".into()); }
    if !prior_ready || !connected || (!imported_exact && !selected_metadata_missing) { return Err("QA_BASELINE_MISMATCH".into()); }
    Ok(())
}
