//! Fixed YouTube recording entry points. Identities read here are not consent,
//! current recording ownership, packet certificates or publication authority.
#[cfg(all(test, windows))]
mod actual_boundary_test;

use super::{
    files::{Directories, GuardedFile},
    finalize::{self, Fault, VerifiedCandidate},
    managed_probe::{self, ManagedProbe},
    packet_probe,
    recovery::selection::{self, CompletedSelection},
    schema::{Proof, PROOF},
    vp9_admission, Error, SpoolIdentity, LIMITS,
};
use crate::platform_media::{storage, PlatformMediaTools};
use std::{
    fs,
    io::Read,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

/// A malformed/unreadable raw proof must not fall through to the legacy path.
pub(in crate::platform_media) fn has_raw_proof(directory: &Path) -> Result<bool, Error> {
    match fs::symlink_metadata(directory.join(PROOF)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(Error::Path),
    }
}

/// Reads only the fixed owned job proof. The caller must cross-check the job and
/// owner, then run full held receipt/structure/packet verification before use.
pub(in crate::platform_media) fn recovery_identity(
    root: &Path,
    job_id: &str,
    cancel: &AtomicBool,
) -> Result<SpoolIdentity, Error> {
    if cancel.load(Ordering::Acquire) {
        return Err(Error::Cancelled);
    }
    let directory = storage::job_dir(root, "youtube", job_id).map_err(|_| Error::Path)?;
    let directories = Directories::hold(&directory)?;
    let mut file = GuardedFile::open(&directory.join(PROOF), false)?;
    let length = file.check()?;
    if length == 0 || length > 8192 {
        return Err(Error::Proof);
    }
    let mut bytes = vec![0; length as usize];
    file.file.read_exact(&mut bytes).map_err(|_| Error::Io)?;
    let proof: Proof = serde_json::from_slice(&bytes).map_err(|_| Error::Proof)?;
    if proof.version != 1 || proof.identity.job_id != job_id || proof.limits != LIMITS {
        return Err(Error::Proof);
    }
    proof.identity.validate()?;
    proof.clock.validate()?;
    directories.check()?;
    if file.check()? != length {
        return Err(Error::Corrupt);
    }
    if cancel.load(Ordering::Acquire) {
        return Err(Error::Cancelled);
    }
    Ok(proof.identity)
}

fn selected_error(error: selection::Fault) -> Fault {
    match error {
        selection::Fault::Source(Error::Cancelled) => Fault::Cancelled,
        selection::Fault::Source(error) => Fault::Source(error),
        selection::Fault::Overflow => Fault::Bounds,
        selection::Fault::Poisoned => Fault::Terminal,
        selection::Fault::Changed => Fault::Changed,
        _ => Fault::Coverage,
    }
}
fn checkpoint(cancel: &AtomicBool, deadline: Instant) -> Result<(), Fault> {
    if cancel.load(Ordering::Acquire) {
        return Err(Fault::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(Fault::Deadline);
    }
    Ok(())
}

/// Same entry for a just-stopped raw archive and an explicit local retry. No
/// network, browser URL, fake encoded segment, cleanup or path-only publisher.
pub(in crate::platform_media) fn prepare_recording(
    root: &Path,
    identity: &SpoolIdentity,
    tools: &PlatformMediaTools,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<VerifiedCandidate, Fault> {
    checkpoint(cancel, deadline)?;
    identity.validate().map_err(Fault::Source)?;
    let mut source = CompletedSelection::prepare(root, identity, cancel).map_err(selected_error)?;
    checkpoint(cancel, deadline)?;
    let admission =
        vp9_admission::admit_selected(&mut source, cancel).map_err(|error| match error {
            vp9_admission::Fault::Cancelled => Fault::Cancelled,
            vp9_admission::Fault::Bounds => Fault::Bounds,
            vp9_admission::Fault::Io => Fault::Io,
            _ => Fault::UnsupportedAdmission,
        })?;
    checkpoint(cancel, deadline)?;
    let tool = ManagedProbe::open(tools, cancel, deadline).map_err(|error| match error {
        managed_probe::Fault::Cancelled => Fault::Cancelled,
        managed_probe::Fault::Deadline => Fault::Deadline,
        _ => Fault::Probe(packet_probe::Fault::ToolUnavailable),
    })?;
    finalize::prepare_selected(root, source, &admission, &tool, cancel, deadline)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_media::youtube_spool::{ClockDescriptor, Mode, Owner, SpoolWriter, Window};
    use std::{sync::Arc, time::Duration};
    fn identity() -> SpoolIdentity {
        SpoolIdentity {
            job_id: uuid::Uuid::new_v4().to_string(),
            request_id: uuid::Uuid::new_v4().to_string(),
            receiver_id: uuid::Uuid::new_v4().to_string(),
            source_id: uuid::Uuid::new_v4().to_string(),
            document_id: uuid::Uuid::new_v4().to_string(),
            document_epoch: 1,
            source_generation: 1,
            video_id: "abcdefghijk".into(),
            channel_id: format!("UC{}", "a".repeat(22)),
            owner: Owner::Manual,
        }
    }
    #[test]
    fn fixed_raw_proof_dispatch_and_identity_are_not_media_admission() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("YouTube")).unwrap();
        let identity = identity();
        let cancel = Arc::new(AtomicBool::new(false));
        let writer = SpoolWriter::create_new(
            root.path(),
            identity.clone(),
            ClockDescriptor {
                video_timestamp_scale_ns: 1_000_000,
                audio_timescale: 44_100,
                mode: Mode::Segments,
                append_window: Window::Default,
                video_timestamp_offset: 0,
                audio_timestamp_offset: 0,
            },
            cancel.clone(),
        )
        .unwrap();
        let directory = storage::job_dir(root.path(), "youtube", &identity.job_id).unwrap();
        drop(writer);
        assert!(has_raw_proof(&directory).unwrap());
        assert_eq!(
            recovery_identity(root.path(), &identity.job_id, &cancel).unwrap(),
            identity
        );
        let tools = PlatformMediaTools {
            yt_dlp: root.path().join("unused"),
            deno: root.path().join("unused"),
            ffmpeg: root.path().join("unused"),
            ffprobe: root.path().join("unused"),
        };
        assert!(matches!(
            prepare_recording(
                root.path(),
                &identity,
                &tools,
                &cancel,
                Instant::now() + Duration::from_secs(1)
            ),
            Err(Fault::Coverage)
        ));
        cancel.store(true, Ordering::Release);
        assert_eq!(
            recovery_identity(root.path(), &identity.job_id, &cancel),
            Err(Error::Cancelled)
        );
    }
    #[test]
    fn invalid_raw_presence_never_means_legacy_fallback() {
        let directory = tempfile::tempdir().unwrap();
        assert!(!has_raw_proof(directory.path()).unwrap());
        fs::create_dir(directory.path().join(PROOF)).unwrap();
        assert!(has_raw_proof(directory.path()).unwrap());
    }
}
