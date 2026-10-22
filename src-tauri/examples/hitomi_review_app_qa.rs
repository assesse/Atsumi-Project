//! Full production UI/IPC with synthetic, genuinely verified review artifacts.
//! Build/run only in debug, with the ordinary app closed and a workspace Vite
//! server ready. Optional argument: its loopback port (default 14200).
//! No existing DB/profile is reused; no decisions, UI inputs or cleanup occur.

#[cfg(all(windows, debug_assertions))]
mod qa {
    use atsumi_lib::{
        application::{
            ApplicationService, DownloadGallerySnapshot, DownloadOverlapRepository,
            DownloadPagePayload, DownloadPipelineRepository, DownloadSourceImageFormat,
            DownloadSourcePage, DownloadSourcePort, DownloadSupervisor,
        },
        domain::{
            ArtifactStorageFormat, DownloadArtifactState, DownloadEntry, DownloadEntryId, DownloadListRequest,
            DownloadOverlapAutoMode, DownloadOverlapReviewState, Gallery, GalleryId,
            GalleryMetadata, JobState, PageArtifactState, SettingsPatch, SourcePageNumber,
        },
        infrastructure::{FilesystemArtifactStore, SqliteRepository},
        source::SourceContractError,
        thumbnail::CancellationToken,
    };
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use serde_json::json;
    use std::{
        fs::{self, OpenOptions},
        io::{Cursor, Read, Write},
        net::TcpListener,
        os::windows::fs::MetadataExt,
        path::{Path, PathBuf},
        sync::{atomic::{AtomicBool, AtomicUsize, Ordering}, mpsc, Arc},
        thread,
        time::{Duration, Instant},
    };
    use windows::{core::GUID, Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{SHGetKnownFolderPath, FOLDERID_LocalAppData, FOLDERID_RoamingAppData, KF_FLAG_DONT_VERIFY},
    }};

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
    const ALBUM_A: i64 = 9_900_000_501;
    const ALBUM_B: i64 = 9_900_000_502;
    const PAGE_COUNT: u32 = 2;

    // Only synthetic source bytes are substituted. Storage, hashes, duplicate
    // analysis, review persistence, finalization and later UI IPC are real.
    struct SyntheticSource {
        pages: Vec<Vec<u8>>,
        page_calls: AtomicUsize,
        metadata_calls: AtomicUsize,
    }

    impl SyntheticSource {
        fn new() -> Result<Self> {
            let mut pages = Vec::new();
            for page in 1..=PAGE_COUNT {
                let image = RgbaImage::from_fn(64, 96, |x, y| Rgba([
                    ((x * 13 + y * 7 + page * 61) % 256) as u8,
                    ((x * 3 + y * 17 + page * 47) % 256) as u8,
                    ((x * 19 + y * 5 + page * 23) % 256) as u8,
                    255,
                ]));
                let mut bytes = Cursor::new(Vec::new());
                DynamicImage::ImageRgba8(image).write_to(&mut bytes, ImageFormat::Png)?;
                pages.push(bytes.into_inner());
            }
            Ok(Self { pages, page_calls: AtomicUsize::new(0), metadata_calls: AtomicUsize::new(0) })
        }
    }

    fn check_request(id: GalleryId, cancellation: &CancellationToken) -> std::result::Result<(), SourceContractError> {
        if cancellation.is_cancelled() { return Err(SourceContractError::cancelled()); }
        if !matches!(id.get(), ALBUM_A | ALBUM_B) {
            return Err(SourceContractError::validation("galleryId", "not an isolated fixture album"));
        }
        Ok(())
    }

    impl DownloadSourcePort for SyntheticSource {
        fn gallery_snapshot(&self, id: GalleryId, cancellation: &CancellationToken)
            -> std::result::Result<DownloadGallerySnapshot, SourceContractError> {
            check_request(id, cancellation)?;
            self.metadata_calls.fetch_add(1, Ordering::Relaxed);
            let metadata = GalleryMetadata::new(
                if id.get() == ALBUM_A { "ISOLATED REVIEW QA · A" } else { "ISOLATED REVIEW QA · B" },
                Some("Atsumi synthetic review fixture".into()), None, PAGE_COUNT,
            ).map_err(|_| SourceContractError::invalid_data("metadata", "invalid synthetic fixture"))?;
            let pages = (1..=PAGE_COUNT).map(|page| DownloadSourcePage {
                source_page_number: SourcePageNumber::new(page).expect("bounded positive fixture page"),
                source_revision: format!("isolated-review-page-v1:{page}"),
            }).collect();
            Ok(DownloadGallerySnapshot {
                gallery: Gallery::new(id, 1, metadata),
                source_revision: "isolated-review-gallery-v1".into(), pages,
            })
        }

        fn download_page(&self, id: GalleryId, page: SourcePageNumber, cancellation: &CancellationToken)
            -> std::result::Result<DownloadPagePayload, SourceContractError> {
            check_request(id, cancellation)?;
            let bytes = self.pages.get((page.get() - 1) as usize)
                .ok_or_else(|| SourceContractError::validation("sourcePage", "not a fixture page"))?;
            self.page_calls.fetch_add(1, Ordering::Relaxed);
            Ok(DownloadPagePayload::for_isolated_qa(
                page, bytes.clone(), format!("isolated-review-page-v1:{}", page.get()),
                DownloadSourceImageFormat::Png, 64, 96,
            ))
        }
    }

    struct PreparationSupervisor(DownloadSupervisor);
    impl Drop for PreparationSupervisor {
        fn drop(&mut self) { self.0.shutdown_and_wait(); }
    }

    fn wait_for_state(service: &ApplicationService, id: &str, state: JobState) -> Result<DownloadEntry> {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let page = service.download_entries_list(DownloadListRequest {
                state: None, query: None, page: 1, page_size: 10,
            })?;
            if let Some(entry) = page.entries.into_iter().find(|entry| entry.entry_id.as_str() == id) {
                if entry.state == state { return Ok(entry); }
                if matches!(entry.state, JobState::Failed | JobState::Interrupted | JobState::Cancelled) {
                    return Err(format!("Synthetic preparation stopped in {}", entry.state).into());
                }
            }
            thread::sleep(Duration::from_millis(25));
        }
        Err("Synthetic preparation timed out; all isolated evidence is retained".into())
    }

    fn prepare(repository: &Arc<SqliteRepository>, service: &ApplicationService, media: &Path) -> Result<serde_json::Value> {
        let source = Arc::new(SyntheticSource::new()?);
        let (events, receiver) = mpsc::channel();
        let supervisor = PreparationSupervisor(DownloadSupervisor::new(
            repository.clone(), repository.clone(), source.clone(),
            Arc::new(FilesystemArtifactStore::new()), events, 1,
        )?);
        let first = service.download_queue_add(vec![ALBUM_A], "isolated-review-existing".into())?;
        let first_id = first.entries[0].entry_id.to_string();
        supervisor.0.enqueue_all(first.jobs)?;
        let completed = wait_for_state(service, &first_id, JobState::Completed)?;
        let second = service.download_queue_add(vec![ALBUM_B], "isolated-review-incoming".into())?;
        let second_id = second.entries[0].entry_id.to_string();
        supervisor.0.enqueue_all(second.jobs)?;
        let paused = wait_for_state(service, &second_id, JobState::ReviewRequired)?;
        let review_id = paused.review_id.as_deref().ok_or("Pending review ID missing")?;
        let review = supervisor.0.overlap_review_get(review_id)?.ok_or("Pending review missing")?;
        if review.state != DownloadOverlapReviewState::Pending || review.candidates.len() != 1
            || review.incoming.gallery_id.get() != ALBUM_B || review.candidates[0].existing.gallery_id.get() != ALBUM_A
            || review.candidates[0].exact_pages != PAGE_COUNT {
            return Err("Unexpected review identity/candidate set".into());
        }
        // Finish every preparation worker before the ordinary app owns this DB.
        drop(supervisor);
        let a = repository.pipeline_artifact_bundle(&DownloadEntryId::new(first_id.clone())?)?.ok_or("A artifact missing")?;
        let b = repository.pipeline_artifact_bundle(&DownloadEntryId::new(second_id.clone())?)?.ok_or("B artifact missing")?;
        for bundle in [&a, &b] {
            if bundle.artifact.expected_page_count != PAGE_COUNT
                || bundle.pages.len() != PAGE_COUNT as usize || bundle.pages.iter().any(|page|
                page.state != PageArtifactState::Present || page.sha256.is_none() || page.excluded
                || page.source_revision.is_none() || page.verified_at.is_none()
                || page.storage_format != Some(ArtifactStorageFormat::Webp)
                || page.page_id.gallery_id != bundle.artifact.gallery_id
                || page.relative_path.as_str() != format!("{}/{:04}.webp", bundle.artifact.relative_directory, page.page_id.source_page_number.get())
                || page.byte_length.unwrap_or(0) == 0) {
                return Err("Real pipeline did not leave complete verified page checkpoints".into());
            }
            for page in &bundle.pages {
                let stored = media.join(page.relative_path.as_str());
                reject_reparse_ancestors(&stored)?;
                if !stored.is_file() || Some(fs::metadata(&stored)?.len()) != page.byte_length {
                    return Err("Verified checkpoint file is missing or its length changed".into());
                }
            }
        }
        let manifest_a = media.join(a.artifact.manifest_relative_path.as_ref().ok_or("A manifest path missing")?.as_str());
        let manifest_b = media.join(b.artifact.manifest_relative_path.as_ref().ok_or("B reserved manifest path missing")?.as_str());
        if a.artifact.state != DownloadArtifactState::Complete || !manifest_a.is_file()
            || b.artifact.state != DownloadArtifactState::Incomplete || manifest_b.exists() {
            return Err("Preparation must preserve completed A and pre-manifest review B".into());
        }
        let review_still_pending = repository.overlap_review_get(review_id)?.ok_or("Review lost after graceful drain")?;
        if review_still_pending.state != DownloadOverlapReviewState::Pending { return Err("Preparation changed the pending decision".into()); }
        let event_count = receiver.try_iter().count();
        let page_calls = source.page_calls.load(Ordering::Relaxed);
        if page_calls != (PAGE_COUNT * 2) as usize { return Err("Unexpected synthetic download count".into()); }
        Ok(json!({
            "preparationOnly": true, "uiAcceptance": "pending", "galleryA": ALBUM_A, "galleryB": ALBUM_B,
            "entryA": first_id, "entryB": second_id, "reviewId": review_id,
            "candidateId": review.candidates[0].candidate_id, "reviewRevision": review.revision,
            "completedRevisionA": completed.revision, "reviewRevisionB": paused.revision,
            "syntheticPageCalls": page_calls, "syntheticMetadataCalls": source.metadata_calls.load(Ordering::Relaxed),
            "realNativeEvents": event_count, "manifestA": manifest_a, "reservedManifestB": manifest_b,
            "reviewSnapshot": review,
        }))
    }

    fn known_folder(id: &GUID) -> Result<PathBuf> {
        // Resolve the same Windows known folder as Tauri; APPDATA overrides do
        // not redirect it. Free the OS allocation even when decoding fails.
        let raw = unsafe { SHGetKnownFolderPath(id, KF_FLAG_DONT_VERIFY, None)? };
        let path = unsafe { raw.to_string() };
        unsafe { CoTaskMemFree(Some(raw.0.cast())) };
        Ok(PathBuf::from(path?))
    }

    fn reject_reparse_ancestors(path: &Path) -> Result<()> {
        for ancestor in path.ancestors() {
            match fs::symlink_metadata(ancestor) {
                Ok(metadata) if metadata.file_attributes() & 0x400 != 0 => return Err("QA path traverses a reparse point".into()),
                Ok(_) => {},
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    // Never forwards or resolves a remote host. Hitomi's normal reqwest client
    // honors this process-local proxy; unrelated providers with no_proxy must
    // not be used during this specific Downloads/review/activity acceptance.
    struct DenyProxy { stopping: Arc<AtomicBool>, worker: Option<thread::JoinHandle<()>>, address: String }
    impl DenyProxy {
        fn new() -> Result<Self> {
            let listener = TcpListener::bind(("127.0.0.1", 0))?;
            listener.set_nonblocking(true)?;
            let address = format!("http://{}", listener.local_addr()?);
            let stopping = Arc::new(AtomicBool::new(false));
            let stop = stopping.clone();
            let worker = thread::spawn(move || {
                let mut denied = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut socket, _)) => {
                            let _ = socket.set_read_timeout(Some(Duration::from_millis(100)));
                            let _ = socket.set_write_timeout(Some(Duration::from_millis(100)));
                            let mut discard = [0u8; 1024];
                            let _ = socket.read(&mut discard);
                            let _ = socket.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            denied += 1;
                            if denied <= 20 { println!("{}", json!({"kind":"hitomi_review_qa_network_denied","count":denied})); }
                        },
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(25)),
                        Err(_) => break,
                    }
                }
            });
            Ok(Self { stopping, worker: Some(worker), address })
        }
    }
    impl Drop for DenyProxy {
        fn drop(&mut self) {
            self.stopping.store(true, Ordering::Relaxed);
            if let Some(worker) = self.worker.take() { let _ = worker.join(); }
        }
    }

    pub fn run() -> Result<()> {
        let arguments: Vec<_> = std::env::args().skip(1).collect();
        if arguments.len() > 1 { return Err("Expected one optional workspace Vite port".into()); }
        let port: u16 = arguments.first().map(|value| value.parse()).transpose()?.unwrap_or(14200);
        if port < 1024 { return Err("Non-system loopback port required".into()); }
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().ok_or("Workspace missing")?.canonicalize()?;
        let identity = format!("local.atsumi.hitomireviewqa.{}", uuid::Uuid::new_v4().simple());
        let parent = workspace.join(".runtime/hitomi-review-app-qa");
        let root = parent.join(&identity);
        let data = known_folder(&FOLDERID_RoamingAppData)?.join(&identity);
        let profile = known_folder(&FOLDERID_LocalAppData)?.join(&identity);
        for path in [&root, &data, &profile] {
            reject_reparse_ancestors(path)?;
            if path.try_exists()? { return Err("QA identity path already exists; nothing overwritten".into()); }
        }
        fs::create_dir_all(&parent)?;
        // Atomic create_dir fails if another owner won a race; never adopt it.
        fs::create_dir(&root)?;
        fs::create_dir(&data)?;
        fs::create_dir(&profile)?;
        let media = root.join("media");
        let local = root.join("diagnostics-local");
        fs::create_dir(&media)?;
        fs::create_dir(&local)?;
        std::env::set_var("LOCALAPPDATA", &local);
        let proxy = DenyProxy::new()?;
        for key in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"] {
            std::env::set_var(key, &proxy.address);
        }
        for key in ["NO_PROXY", "no_proxy"] { std::env::set_var(key, "127.0.0.1,localhost,::1"); }
        // A parent shell must not silently substitute an ordinary browser profile.
        std::env::remove_var("WEBVIEW2_USER_DATA_FOLDER");
        std::env::remove_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS");
        let repository = Arc::new(SqliteRepository::open(data.join("atsumi-next.sqlite3"))?);
        let service = ApplicationService::new(repository.clone()).with_download_repository(repository.clone());
        service.settings_update(SettingsPatch {
            download_root: Some(media.to_string_lossy().into_owned()), privacy_on_startup: Some(false),
            download_overlap_auto_mode: Some(DownloadOverlapAutoMode::Off),
            ..Default::default()
        }, service.settings_get()?.revision)?;
        let mut evidence = prepare(&repository, &service, &media)?;
        evidence["identity"] = json!(identity);
        evidence["root"] = json!(root);
        evidence["database"] = json!(data.join("atsumi-next.sqlite3"));
        evidence["profile"] = json!(profile);
        evidence["diagnostics"] = json!(local.join("Atsumi/Logs/diagnostics"));
        let mut file = OpenOptions::new().write(true).create_new(true).open(root.join("preparation.json"))?;
        serde_json::to_writer_pretty(&mut file, &evidence)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        println!("{}", json!({"kind":"hitomi_review_app_qa_prepared","root":root,"database":data.join("atsumi-next.sqlite3"),"profile":profile,"uiAcceptance":"pending"}));
        println!("Downloads → ISOLATED REVIEW QA · B → 검토 → 둘 다 보존. Then observe real completion in ActivityDrawer; do not open other platforms or use real gallery IDs.");
        drop(service);
        drop(repository);
        let mut context = tauri::generate_context!();
        context.config_mut().identifier = identity;
        context.config_mut().build.dev_url = Some(format!("http://127.0.0.1:{port}").parse()?);
        for window in &mut context.config_mut().app.windows {
            window.title = "Atsumi · Hitomi 검토 완료 검증 (격리)".into();
            window.data_directory = None; // new UUID's default LocalAppData profile
            window.additional_browser_args = Some(format!("--proxy-server={} --proxy-bypass-list=127.0.0.1;localhost;[::1] --disable-background-networking", proxy.address));
        }
        // Normal Tauri quit may exit directly. Neither this return nor exit0 is
        // a UI pass. Root inspects preparation.json, saved state and the screen.
        atsumi_lib::run_with_context(context)?;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(all(windows, debug_assertions))]
    { qa::run() }
    #[cfg(not(all(windows, debug_assertions)))]
    { Err("Isolated review QA requires a Windows debug build".into()) }
}
