//! Exact completed QA run, offline read-only diagnostic. Never seals or publishes.
use super::*;
use std::{fs, io::{Read, Seek, SeekFrom, Write}, path::Path};
use sha2::{Digest, Sha256};

const RUN: &str = "327e422347d34cafb18d521008ba0e14";
const SIDECAR: &str = "c3c7d32f-5085-41bd-9d62-95b0479e7b24";
const SELECTION: &str = "2c5e2e6a-e09e-4d6a-aa92-c70d9513bbfe";

fn digest(file: &mut fs::File) -> [u8; 32] {
    file.seek(SeekFrom::Start(0)).expect("QA_READ_SEEK");
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 65536];
    loop {
        let n = file.read(&mut bytes).expect("QA_READ");
        if n == 0 { break; }
        hash.update(&bytes[..n]);
    }
    hash.finalize().into()
}

#[test]
#[ignore = "Exact completed SOOP QA only; normal Vault load, no writes/network/source opening"]
fn actual_soop_327e_bridge_path_readonly() {
    assert_eq!(std::env::var("ATSUMI_SOOP_BRIDGE_PATH_PROBE").as_deref(), Ok(RUN), "QA_OPT_IN");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("QA_WORKSPACE");
    let run = workspace.join(".runtime/soop-event-clock-native-qa").join(RUN);
    let directory = run.join("media/SOOP/Relays").join(SIDECAR);
    let canonical = fs::canonicalize(&directory).expect("QA_SOURCE_EXISTS");
    let native_pin = crate::streaming::browser_store::cleanup::pin_directories(&directory).is_ok();
    let canonical_pins = crate::streaming::browser_store::cleanup::pin_directories(&canonical);
    let canonical_pin = canonical_pins.is_ok();
    let _pins = canonical_pins.expect("QA_CANONICAL_PINS");
    let vault_root = run.join("sealed-vault");
    assert!(vault_root.join("cloud-recording/state.bin").is_file()
        && vault_root.join("cloud-recording/state.lock").is_file(), "QA_EXISTING_VAULT_ONLY");
    let state_path = fs::canonicalize(vault_root.join("cloud-recording/state.bin")).expect("QA_STATE_PATH");
    // This handle blocks all catalog writes throughout the normal Vault read.
    let mut state = crate::streaming::browser_store::cleanup::verification_guard(&state_path).expect("QA_STATE_PIN");
    let state_hash = digest(&mut state);
    let manifest_existed = canonical.join("relay-history-seal.json").exists();
    let mut source_pins = Vec::new();
    for entry in fs::read_dir(&canonical).expect("QA_SOURCE_DIR") {
        let entry = entry.expect("QA_SOURCE_ENTRY");
        let mut paths = Vec::new();
        if entry.file_type().expect("QA_SOURCE_TYPE").is_dir() {
            assert!(matches!(entry.file_name().to_str(), Some("chat-assets" | "replay-assets")), "QA_SUBDIRECTORY");
            for item in fs::read_dir(entry.path()).expect("QA_ASSET_DIR") { paths.push(item.expect("QA_ASSET_ENTRY").path()); }
        } else { paths.push(entry.path()); }
        for path in paths {
            let mut file = crate::streaming::browser_store::cleanup::verification_guard(&path).expect("QA_SOURCE_PIN");
            let hash = digest(&mut file);
            source_pins.push((file, hash));
        }
    }
    let owner: serde_json::Value = serde_json::from_slice(&fs::read(canonical.join("relay-owner.json")).expect("QA_OWNER_READ")).expect("QA_OWNER_SHAPE");
    // The standard encrypted-store reader is used in place. No grant is inspected,
    // serialized or copied; no service scheduler/recovery constructor is started.
    let vault = super::super::vault::Vault::open(&vault_root).expect("QA_VAULT_BUSY_OR_UNAVAILABLE");
    let catalog = vault.load().expect("QA_VAULT_READ");
    assert_eq!(catalog.live_receipts.len(), 1, "QA_SINGLE_RECEIPT");
    let receipt = &catalog.live_receipts[0];
    assert_eq!(receipt.view.selection_id, SELECTION, "QA_SELECTION");
    let binding = &owner["binding"];
    let owner_matches = receipt.source.provider == "soop"
        && binding["receiverId"].as_str() == Some(receipt.source.session_id.as_str())
        && binding["documentEpoch"].as_u64() == Some(receipt.source.epoch)
        && binding["channelId"].as_str() == Some(receipt.source.channel_id.as_str())
        && binding["broadcastId"].as_str() == receipt.source.broadcast_key.as_deref()
        && binding["runId"].as_str() == receipt.view.run_id.as_deref()
        && owner["sidecarId"].as_str() == receipt.sidecar_id.as_deref();
    let capture_present = receipt.sidecar.is_some();
    let capture_valid = receipt.sidecar.as_ref().map(|r| r.valid_for(receipt));
    let sealed = receipt.sidecar.as_ref().is_some_and(|r| r.sealed.is_some());
    let source_count = source_pins.len();
    let originals_unchanged = source_pins.iter_mut().all(|(file, hash)| digest(file) == *hash);
    let state_unchanged = digest(&mut state) == state_hash;
    let manifest_unchanged = canonical.join("relay-history-seal.json").exists() == manifest_existed;
    assert!(originals_unchanged && state_unchanged && manifest_unchanged, "QA_PRESERVATION");
    let report = serde_json::json!({"run":RUN,"ordinaryEqualsCanonical":directory==canonical,
        "ordinaryStrictPinAccepted":native_pin,"canonicalStrictPinAccepted":canonical_pin,
        "storedOwnerBindingMatches":owner_matches,"receiptValid":receipt.valid(),
        "receiptJoined":receipt.view.state=="joined","capturePresent":capture_present,
        "captureValid":capture_valid,"sealed":sealed,"manifestExisted":manifest_existed,
        "originalFilesHeld":source_count,"originalsUnchanged":originals_unchanged,
        "stateUnchanged":state_unchanged,"manifestUnchanged":manifest_unchanged,
        "networkCalls":0,"sourceWrites":0,"catalogSaves":0,"newCapture":false});
    drop(catalog); drop(vault);
    let output = workspace.join(".runtime/requirements-audit/20261122-cloud-resume")
        .join(format!("soop-bridge-path-probe-{}.json", uuid::Uuid::new_v4().simple()));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(output).expect("QA_REPORT_CREATE");
    file.write_all(&serde_json::to_vec_pretty(&report).expect("QA_REPORT_SHAPE")).expect("QA_REPORT_WRITE");
    println!("SOOP_BRIDGE_PATH_PROBE {}", report);
}
