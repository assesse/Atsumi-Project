use std::{
    borrow::Cow,
    fs::{self, File, OpenOptions},
    io::{BufWriter, Cursor, ErrorKind, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use image::{
    codecs::webp::WebPEncoder, ExtendedColorType, GenericImageView, ImageEncoder, ImageFormat,
    ImageReader, Limits,
};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    application::{
        ArtifactLayout, ArtifactStore, DownloadPagePayload, DownloadPipelineError,
        DownloadPipelineErrorCode, DownloadSourceImageFormat, ExistingPageVerification, StoredPage,
    },
    domain::{
        ArtifactBundle, ArtifactManifest, ArtifactRelativePath, ArtifactSha256,
        ArtifactStorageFormat, DownloadArtifactState, PageArtifact, PageArtifactState,
        SourcePageNumber, MAX_MANAGED_ABSOLUTE_PATH_UTF16,
    },
    thumbnail::CancellationToken,
};

const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_DECODE_ALLOC: u64 = 256 * 1024 * 1024;
const MAX_MANAGED_PAGE_BYTES: u64 = 64 * 1024 * 1024;
const MANIFEST_FILE_NAME: &str = "manifest.json";

#[derive(Debug, Default)]
pub struct FilesystemArtifactStore;

impl FilesystemArtifactStore {
    pub const fn new() -> Self {
        Self
    }
}

impl ArtifactStore for FilesystemArtifactStore {
    fn validate_download_root(&self, root: &Path) -> Result<PathBuf, DownloadPipelineError> {
        if root.as_os_str().is_empty() || !root.is_absolute() {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::RootUnavailable,
                "The download folder must be an absolute path",
                false,
            ));
        }
        fs::create_dir_all(root)
            .map_err(|_| filesystem_error("The download folder could not be created"))?;
        let root = root
            .canonicalize()
            .map_err(|_| filesystem_error("The download folder could not be resolved"))?;
        if !root.is_dir() {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::RootUnavailable,
                "The selected download path is not a folder",
                false,
            ));
        }

        let probe = root.join(format!(".atsumi-write-probe-{}", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&probe)
            .map_err(|_| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::RootUnavailable,
                    "The selected download folder is not writable",
                    false,
                )
            })?;
        let probe_result = file.write_all(b"atsumi").and_then(|()| file.sync_all());
        drop(file);
        let cleanup_result = fs::remove_file(&probe);
        if probe_result.is_err() || cleanup_result.is_err() {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::RootUnavailable,
                "The selected download folder could not complete a safe write test",
                false,
            ));
        }
        Ok(root)
    }

    fn prepare_layout(
        &self,
        root: &Path,
        relative_directory: &ArtifactRelativePath,
        allow_existing: bool,
    ) -> Result<ArtifactLayout, DownloadPipelineError> {
        let root = self.validate_download_root(root)?;
        let directory = root.join(relative_directory.as_str());
        if directory.as_os_str().encode_wide_units() > MAX_MANAGED_ABSOLUTE_PATH_UTF16 {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::PathOutsideRoot,
                "The planned artifact path is too long for the Windows safety budget",
                false,
            ));
        }
        if directory.exists() {
            if !allow_existing || !directory.is_dir() {
                return Err(DownloadPipelineError::new(
                    DownloadPipelineErrorCode::DestinationOccupied,
                    "The planned artifact folder already exists and was not modified",
                    false,
                ));
            }
        } else {
            fs::create_dir(&directory)
                .map_err(|_| filesystem_error("The gallery folder could not be created"))?;
        }
        let canonical_directory = directory
            .canonicalize()
            .map_err(|_| filesystem_error("The gallery folder could not be resolved"))?;
        ensure_descendant(&root, &canonical_directory)?;
        let manifest_relative_path = ArtifactRelativePath::new(format!(
            "{}/{}",
            relative_directory.as_str(),
            MANIFEST_FILE_NAME
        ))
        .map_err(|error| invalid_path(error.to_string()))?;
        Ok(ArtifactLayout {
            root,
            relative_directory: relative_directory.clone(),
            manifest_relative_path,
        })
    }

    fn verify_existing_page(
        &self,
        layout: &ArtifactLayout,
        source_page_number: SourcePageNumber,
        source_revision: &str,
        expected: Option<&StoredPage>,
    ) -> Result<ExistingPageVerification, DownloadPipelineError> {
        let relative_path = page_relative_path(layout, source_page_number)?;
        let part_relative_path = ArtifactRelativePath::new(format!(
            "{}/.{:04}.webp.part",
            layout.relative_directory.as_str(),
            source_page_number.get()
        ))
        .map_err(|error| invalid_path(error.to_string()))?;
        let path = resolve_managed_path(&layout.root, &relative_path, false)?;
        let part_path = resolve_managed_path(&layout.root, &part_relative_path, false)?;
        let final_exists = fs::symlink_metadata(&path).is_ok();
        let part_exists = fs::symlink_metadata(&part_path).is_ok();
        if !final_exists && !part_exists {
            return Ok(ExistingPageVerification::Missing);
        }
        if expected.is_none()
            || expected.is_some_and(|checkpoint| checkpoint.source_revision != source_revision)
            || part_exists
        {
            recover_conflicting_page_files(layout, &[relative_path.clone(), part_relative_path])?;
            return Ok(ExistingPageVerification::Invalid {
                relative_path,
                reason: "ambiguous page files were moved to recovery review storage",
            });
        }
        let expected = expected.expect("the ambiguous checkpoint branch returned above");
        let stored =
            match verify_checkpoint_webp_file(&layout.root, relative_path.clone(), expected) {
                Ok(stored) => stored,
                Err(_) => {
                    recover_conflicting_page_files(layout, std::slice::from_ref(&relative_path))?;
                    return Ok(ExistingPageVerification::Invalid {
                        relative_path,
                        reason: "an unverifiable page was moved to recovery review storage",
                    });
                }
            };
        Ok(ExistingPageVerification::Verified(stored))
    }

    fn store_page(
        &self,
        layout: &ArtifactLayout,
        page: &DownloadPagePayload,
        cancellation: &CancellationToken,
    ) -> Result<StoredPage, DownloadPipelineError> {
        ensure_not_cancelled(cancellation)?;
        let final_relative_path = page_relative_path(layout, page.source_page_number)?;
        let part_relative_path = ArtifactRelativePath::new(format!(
            "{}/.{:04}.webp.part",
            layout.relative_directory.as_str(),
            page.source_page_number.get()
        ))
        .map_err(|error| invalid_path(error.to_string()))?;
        let final_path = resolve_managed_path(&layout.root, &final_relative_path, false)?;
        let part_path = resolve_managed_path(&layout.root, &part_relative_path, false)?;
        reject_symlink_leaf(&final_path)?;
        reject_symlink_leaf(&part_path)?;
        let bytes = normalized_webp_bytes(page)?;
        let expected_part = StoredPage {
            source_page_number: page.source_page_number,
            relative_path: part_relative_path.clone(),
            byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            sha256: ArtifactSha256::new(format!("{:x}", Sha256::digest(bytes.as_ref()))).map_err(
                |error| {
                    DownloadPipelineError::new(
                        DownloadPipelineErrorCode::HashMismatch,
                        error.to_string(),
                        false,
                    )
                },
            )?,
            storage_format: ArtifactStorageFormat::Webp,
            source_revision: page.source_revision.clone(),
            verified_at: now_unix_ms(),
        };
        ensure_not_cancelled(cancellation)?;

        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&part_path)
            .map_err(|_| filesystem_error("The temporary page file could not be created"))?;
        let mut writer = BufWriter::with_capacity(64 * 1024, file);
        for chunk in bytes.chunks(64 * 1024) {
            ensure_not_cancelled(cancellation)?;
            writer
                .write_all(chunk)
                .map_err(|_| filesystem_error("The temporary page file could not be written"))?;
        }
        writer
            .flush()
            .map_err(|_| filesystem_error("The temporary page file could not be flushed"))?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|_| filesystem_error("The temporary page file could not be synchronized"))?;
        drop(writer);
        ensure_not_cancelled(cancellation)?;

        let part_stored =
            verify_checkpoint_webp_file(&layout.root, part_relative_path.clone(), &expected_part)?;
        if final_path.exists() {
            let final_stored = verify_checkpoint_webp_file(
                &layout.root,
                final_relative_path.clone(),
                &expected_part,
            )?;
            if final_stored.sha256 != part_stored.sha256 {
                return Err(DownloadPipelineError::new(
                    DownloadPipelineErrorCode::HashMismatch,
                    "An existing final page differs from the verified temporary file",
                    false,
                ));
            }
            fs::remove_file(&part_path).map_err(|_| {
                filesystem_error("The duplicate temporary page could not be removed")
            })?;
            return Ok(final_stored);
        }
        fs::rename(&part_path, &final_path)
            .map_err(|_| filesystem_error("The verified page could not be finalized atomically"))?;
        Ok(StoredPage {
            relative_path: final_relative_path,
            ..part_stored
        })
    }

    fn write_manifest(
        &self,
        layout: &ArtifactLayout,
        manifest: &ArtifactManifest,
    ) -> Result<(), DownloadPipelineError> {
        let final_path = resolve_managed_path(&layout.root, &layout.manifest_relative_path, false)?;
        let temp_relative = ArtifactRelativePath::new(format!(
            "{}/.manifest-{}.json.part",
            layout.relative_directory.as_str(),
            Uuid::new_v4()
        ))
        .map_err(|error| invalid_path(error.to_string()))?;
        let temp_path = resolve_managed_path(&layout.root, &temp_relative, false)?;
        let bytes = serde_json::to_vec_pretty(manifest).map_err(|_| {
            DownloadPipelineError::new(
                DownloadPipelineErrorCode::ManifestInvalid,
                "The artifact manifest could not be serialized",
                false,
            )
        })?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|_| filesystem_error("The manifest temporary file could not be created"))?;
        file.write_all(&bytes)
            .and_then(|()| file.flush())
            .and_then(|()| file.sync_all())
            .map_err(|_| {
                filesystem_error("The manifest temporary file could not be synchronized")
            })?;
        drop(file);

        let parsed = read_manifest_file(&temp_path)?;
        if parsed != *manifest {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::ManifestInvalid,
                "The manifest verification round trip did not match",
                false,
            ));
        }
        atomic_replace(&temp_path, &final_path)?;
        Ok(())
    }

    fn read_manifest(
        &self,
        layout: &ArtifactLayout,
    ) -> Result<Option<ArtifactManifest>, DownloadPipelineError> {
        let path = resolve_managed_path(&layout.root, &layout.manifest_relative_path, false)?;
        if !path.exists() {
            return Ok(None);
        }
        read_manifest_file(&path).map(Some)
    }

    fn first_verified_page_path(
        &self,
        root: &Path,
        bundle: &ArtifactBundle,
    ) -> Result<PathBuf, DownloadPipelineError> {
        if bundle.artifact.state != DownloadArtifactState::Complete {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::ArtifactMissing,
                "Only a verified complete artifact can be opened",
                false,
            ));
        }
        let root = self.validate_download_root(root)?;
        let page = bundle
            .pages
            .iter()
            .filter(|page| !page.excluded && page.state == PageArtifactState::Present)
            .min_by_key(|page| page.page_id.source_page_number)
            .ok_or_else(|| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ArtifactMissing,
                    "The artifact has no verified page to open",
                    false,
                )
            })?;
        let expected = StoredPage {
            source_page_number: page.page_id.source_page_number,
            relative_path: page.relative_path.clone(),
            byte_length: page.byte_length.ok_or_else(|| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ManifestInvalid,
                    "The first page is missing its byte length",
                    false,
                )
            })?,
            sha256: page.sha256.clone().ok_or_else(|| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ManifestInvalid,
                    "The first page is missing its SHA-256 digest",
                    false,
                )
            })?,
            storage_format: page.storage_format.ok_or_else(|| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ManifestInvalid,
                    "The first page is missing its storage format",
                    false,
                )
            })?,
            source_revision: page.source_revision.clone().ok_or_else(|| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ManifestInvalid,
                    "The first page is missing its source revision",
                    false,
                )
            })?,
            verified_at: page.verified_at.clone().unwrap_or_default(),
        };
        let layout = ArtifactLayout {
            root,
            relative_directory: bundle.artifact.relative_directory.clone(),
            manifest_relative_path: bundle.artifact.manifest_relative_path.clone().ok_or_else(
                || {
                    DownloadPipelineError::new(
                        DownloadPipelineErrorCode::ManifestInvalid,
                        "The artifact is missing its manifest path",
                        false,
                    )
                },
            )?,
        };
        match self.verify_existing_page(
            &layout,
            page.page_id.source_page_number,
            &expected.source_revision,
            Some(&expected),
        )? {
            ExistingPageVerification::Verified(_) => {
                resolve_managed_path(&layout.root, &page.relative_path, true)
            }
            ExistingPageVerification::Missing => Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::ArtifactMissing,
                "The first verified page is missing from disk",
                false,
            )),
            ExistingPageVerification::Invalid { .. } => Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::HashMismatch,
                "The first page no longer matches its verified digest",
                false,
            )),
        }
    }

    fn artifact_directory_path(
        &self,
        root: &Path,
        relative_directory: &ArtifactRelativePath,
    ) -> Result<PathBuf, DownloadPipelineError> {
        let directory = resolve_managed_path(root, relative_directory, true).map_err(|error| {
            if error.code == DownloadPipelineErrorCode::ArtifactMissing {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ArtifactMissing,
                    "The gallery storage folder is not ready yet",
                    false,
                )
            } else {
                error
            }
        })?;
        if !directory.is_dir() {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::ArtifactMissing,
                "The managed gallery storage path is not a folder",
                false,
            ));
        }
        Ok(directory)
    }

    fn open_with_default_viewer(&self, path: &Path) -> Result<(), DownloadPipelineError> {
        open_default_viewer(path)
    }

    fn move_managed_directory(
        &self,
        root: &Path,
        source: &ArtifactRelativePath,
        destination: &ArtifactRelativePath,
    ) -> Result<(), DownloadPipelineError> {
        let root = self.validate_download_root(root)?;
        let source_path = resolve_managed_path(&root, source, true)?;
        if !source_path.is_dir() {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::ArtifactMissing,
                "The managed artifact folder is missing",
                false,
            ));
        }
        let destination_path = resolve_managed_path(&root, destination, false)?;
        if destination_path.exists() {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::QuarantineConflict,
                "The quarantine destination already exists",
                false,
            ));
        }
        fs::rename(&source_path, &destination_path).map_err(|_| {
            DownloadPipelineError::new(
                DownloadPipelineErrorCode::Filesystem,
                "The managed artifact folder could not be moved atomically",
                true,
            )
        })?;
        let canonical_destination = destination_path
            .canonicalize()
            .map_err(|_| filesystem_error("The moved artifact folder could not be resolved"))?;
        ensure_descendant(&root, &canonical_destination)?;
        Ok(())
    }

    fn move_managed_file(
        &self,
        root: &Path,
        source: &ArtifactRelativePath,
        destination: &ArtifactRelativePath,
    ) -> Result<(), DownloadPipelineError> {
        let root = self.validate_download_root(root)?;
        let source_path = resolve_managed_path(&root, source, true)?;
        if !source_path.is_file() {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::ArtifactMissing,
                "The managed artifact page is missing",
                false,
            ));
        }
        let destination_path = resolve_managed_path(&root, destination, false)?;
        if destination_path.exists() {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::QuarantineConflict,
                "The page quarantine destination already exists",
                false,
            ));
        }
        let parent = destination_path.parent().ok_or_else(|| {
            DownloadPipelineError::new(
                DownloadPipelineErrorCode::PathOutsideRoot,
                "The page quarantine destination has no managed parent",
                false,
            )
        })?;
        fs::create_dir_all(parent)
            .map_err(|_| filesystem_error("The page quarantine folder could not be created"))?;
        let canonical_parent = parent
            .canonicalize()
            .map_err(|_| filesystem_error("The page quarantine folder could not be resolved"))?;
        ensure_descendant(&root, &canonical_parent)?;
        fs::rename(&source_path, &destination_path).map_err(|_| {
            DownloadPipelineError::new(
                DownloadPipelineErrorCode::Filesystem,
                "The managed artifact page could not be moved atomically",
                true,
            )
        })?;
        let canonical_destination = destination_path
            .canonicalize()
            .map_err(|_| filesystem_error("The moved artifact page could not be resolved"))?;
        ensure_descendant(&root, &canonical_destination)?;
        Ok(())
    }

    fn managed_path_exists(
        &self,
        root: &Path,
        relative_path: &ArtifactRelativePath,
    ) -> Result<bool, DownloadPipelineError> {
        let root = self.validate_download_root(root)?;
        let candidate = resolve_managed_path(&root, relative_path, false)?;
        if !candidate.exists() {
            return Ok(false);
        }
        let canonical = candidate
            .canonicalize()
            .map_err(|_| filesystem_error("The managed artifact path could not be resolved"))?;
        ensure_descendant(&root, &canonical)?;
        Ok(true)
    }

    fn read_verified_page_bytes(
        &self,
        root: &Path,
        page: &PageArtifact,
    ) -> Result<Vec<u8>, DownloadPipelineError> {
        if page.state != PageArtifactState::Present
            || page.excluded
            || page.storage_format != Some(ArtifactStorageFormat::Webp)
            || page.source_revision.is_none()
            || page.verified_at.is_none()
        {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::ArtifactMissing,
                "Only a present verified non-excluded page can be scanned",
                false,
            ));
        }
        let expected_length = page.byte_length.ok_or_else(|| {
            DownloadPipelineError::new(
                DownloadPipelineErrorCode::ManifestInvalid,
                "The verified page is missing its byte length",
                false,
            )
        })?;
        let expected_sha = page.sha256.as_ref().ok_or_else(|| {
            DownloadPipelineError::new(
                DownloadPipelineErrorCode::ManifestInvalid,
                "The verified page is missing its SHA-256 digest",
                false,
            )
        })?;
        if expected_length == 0 || expected_length > MAX_MANAGED_PAGE_BYTES {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::ImageDecodeFailed,
                "The verified page exceeds the safe scan byte limit",
                false,
            ));
        }
        // Duplicate scans and local previews are read-only.  Do not run the
        // write-probe used by download preparation once per page; the scan
        // supervisor validates the configured root once before dispatching.
        let root = resolve_existing_download_root(root)?;
        let path = resolve_managed_path(&root, &page.relative_path, true)?;
        let metadata = fs::metadata(&path)
            .map_err(|_| filesystem_error("The verified page metadata could not be read"))?;
        if metadata.len() != expected_length {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::HashMismatch,
                "The verified page byte length changed before duplicate scanning",
                false,
            ));
        }
        let bytes = fs::read(&path)
            .map_err(|_| filesystem_error("The verified page could not be read for scanning"))?;
        let actual_sha = format!("{:x}", Sha256::digest(&bytes));
        if actual_sha != expected_sha.as_str() {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::HashMismatch,
                "The verified page SHA-256 changed before duplicate scanning",
                false,
            ));
        }
        decode_image(&bytes, ImageFormat::WebP)?;
        Ok(bytes)
    }
}

trait OsStrUtf16Units {
    fn encode_wide_units(&self) -> usize;
}

impl OsStrUtf16Units for std::ffi::OsStr {
    fn encode_wide_units(&self) -> usize {
        self.to_string_lossy().encode_utf16().count()
    }
}

fn resolve_existing_download_root(root: &Path) -> Result<PathBuf, DownloadPipelineError> {
    if root.as_os_str().is_empty() || !root.is_absolute() {
        return Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::RootUnavailable,
            "The download folder must be an absolute path",
            false,
        ));
    }
    let root = root
        .canonicalize()
        .map_err(|_| filesystem_error("The download folder could not be resolved"))?;
    if !root.is_dir() {
        return Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::RootUnavailable,
            "The selected download path is not a folder",
            false,
        ));
    }
    Ok(root)
}

fn reject_symlink_leaf(path: &Path) -> Result<(), DownloadPipelineError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::PathOutsideRoot,
            "A managed artifact leaf cannot be a symbolic link or reparse point",
            false,
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(_) => Err(filesystem_error(
            "The managed artifact leaf could not be inspected safely",
        )),
    }
}

fn recover_conflicting_page_files(
    layout: &ArtifactLayout,
    relative_paths: &[ArtifactRelativePath],
) -> Result<(), DownloadPipelineError> {
    let recovery_parent = layout.root.join(".atsumi-recovery").join("conflicts");
    fs::create_dir_all(&recovery_parent)
        .map_err(|_| filesystem_error("The recovery conflict folder could not be created"))?;
    let recovery_parent = recovery_parent
        .canonicalize()
        .map_err(|_| filesystem_error("The recovery conflict folder could not be resolved"))?;
    ensure_descendant(&layout.root, &recovery_parent)?;
    let recovery_directory = recovery_parent.join(Uuid::new_v4().to_string());
    fs::create_dir(&recovery_directory)
        .map_err(|_| filesystem_error("A unique recovery conflict folder could not be created"))?;
    for relative_path in relative_paths {
        let source = resolve_managed_path(&layout.root, relative_path, false)?;
        if fs::symlink_metadata(&source).is_err() {
            continue;
        }
        let leaf = source.file_name().ok_or_else(|| {
            invalid_path("A recovery source did not have a safe file name".into())
        })?;
        let destination = recovery_directory.join(leaf);
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(filesystem_error(
                "A recovery destination already exists and was not overwritten",
            ));
        }
        fs::rename(&source, &destination)
            .map_err(|_| filesystem_error("An ambiguous page could not be moved for review"))?;
    }
    Ok(())
}

pub(crate) fn normalized_webp_bytes(
    page: &DownloadPagePayload,
) -> Result<Cow<'_, [u8]>, DownloadPipelineError> {
    if page.source_format == DownloadSourceImageFormat::Webp {
        decode_image(&page.bytes, ImageFormat::WebP)?;
        return Ok(Cow::Borrowed(&page.bytes));
    }
    let format = match page.source_format {
        DownloadSourceImageFormat::Webp => ImageFormat::WebP,
        DownloadSourceImageFormat::Jpeg => ImageFormat::Jpeg,
        DownloadSourceImageFormat::Png => ImageFormat::Png,
        DownloadSourceImageFormat::Avif => {
            let image = super::avif_decode::decode_avif_rgba(&page.bytes).map_err(|_| {
                DownloadPipelineError::new(
                    DownloadPipelineErrorCode::ImageDecodeFailed,
                    "The AVIF page could not be decoded safely",
                    false,
                )
            })?;
            let rgba = image.to_rgba8();
            let mut output = Vec::new();
            WebPEncoder::new_lossless(&mut output)
                .write_image(
                    rgba.as_raw(),
                    rgba.width(),
                    rgba.height(),
                    ExtendedColorType::Rgba8,
                )
                .map_err(|_| {
                    DownloadPipelineError::new(
                        DownloadPipelineErrorCode::ImageEncodeFailed,
                        "The decoded page could not be encoded as WebP",
                        false,
                    )
                })?;
            decode_image(&output, ImageFormat::WebP)?;
            return Ok(Cow::Owned(output));
        }
    };
    let image = decode_image(&page.bytes, format)?;
    let rgba = image.to_rgba8();
    let mut output = Vec::new();
    WebPEncoder::new_lossless(&mut output)
        .write_image(
            rgba.as_raw(),
            rgba.width(),
            rgba.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|_| {
            DownloadPipelineError::new(
                DownloadPipelineErrorCode::ImageEncodeFailed,
                "The decoded page could not be encoded as WebP",
                false,
            )
        })?;
    decode_image(&output, ImageFormat::WebP)?;
    Ok(Cow::Owned(output))
}

fn decode_image(
    bytes: &[u8],
    format: ImageFormat,
) -> Result<image::DynamicImage, DownloadPipelineError> {
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_DECODE_ALLOC);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| {
        DownloadPipelineError::new(
            DownloadPipelineErrorCode::ImageDecodeFailed,
            "The page image could not be decoded safely",
            false,
        )
    })?;
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::ImageDecodeFailed,
            "The decoded page has invalid dimensions",
            false,
        ));
    }
    Ok(image)
}

/// Re-validates an already decoded and checkpointed page without decoding the
/// same immutable payload again. Length, WebP signature and SHA-256 must all
/// still match the canonical checkpoint before the page is reused or a bundle
/// can complete.
fn verify_checkpoint_webp_file(
    root: &Path,
    relative_path: ArtifactRelativePath,
    expected: &StoredPage,
) -> Result<StoredPage, DownloadPipelineError> {
    if expected.storage_format != ArtifactStorageFormat::Webp {
        return Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::ManifestInvalid,
            "The checkpoint storage format is not WebP",
            false,
        ));
    }
    let path = resolve_managed_path(root, &relative_path, true)?;
    let mut file = File::open(&path)
        .map_err(|_| filesystem_error("The checkpointed page could not be opened"))?;
    let byte_length = file
        .metadata()
        .map_err(|_| filesystem_error("The checkpointed page length could not be read"))?
        .len();
    if byte_length != expected.byte_length {
        return Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::HashMismatch,
            "The checkpointed page byte length changed",
            false,
        ));
    }

    let mut digest = Sha256::new();
    let mut signature = [0_u8; 12];
    let mut signature_length = 0_usize;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| filesystem_error("The checkpointed page could not be hashed"))?;
        if read == 0 {
            break;
        }
        if signature_length < signature.len() {
            let copied = (signature.len() - signature_length).min(read);
            signature[signature_length..signature_length + copied]
                .copy_from_slice(&buffer[..copied]);
            signature_length += copied;
        }
        digest.update(&buffer[..read]);
    }
    if signature_length != signature.len()
        || &signature[..4] != b"RIFF"
        || &signature[8..] != b"WEBP"
    {
        return Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::ImageDecodeFailed,
            "The checkpointed page is not a WebP image",
            false,
        ));
    }
    let actual = ArtifactSha256::new(format!("{:x}", digest.finalize())).map_err(|error| {
        DownloadPipelineError::new(
            DownloadPipelineErrorCode::HashMismatch,
            error.to_string(),
            false,
        )
    })?;
    if actual != expected.sha256 {
        return Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::HashMismatch,
            "The checkpointed page SHA-256 changed",
            false,
        ));
    }
    Ok(StoredPage {
        relative_path,
        verified_at: now_unix_ms(),
        ..expected.clone()
    })
}

fn page_relative_path(
    layout: &ArtifactLayout,
    source_page_number: SourcePageNumber,
) -> Result<ArtifactRelativePath, DownloadPipelineError> {
    ArtifactRelativePath::new(format!(
        "{}/{:04}.webp",
        layout.relative_directory.as_str(),
        source_page_number.get()
    ))
    .map_err(|error| invalid_path(error.to_string()))
}

fn resolve_managed_path(
    root: &Path,
    relative: &ArtifactRelativePath,
    must_exist: bool,
) -> Result<PathBuf, DownloadPipelineError> {
    let root = root
        .canonicalize()
        .map_err(|_| filesystem_error("The download root could not be resolved"))?;
    let candidate = root.join(relative.as_str());
    if must_exist {
        let candidate = candidate.canonicalize().map_err(|_| {
            DownloadPipelineError::new(
                DownloadPipelineErrorCode::ArtifactMissing,
                "A managed artifact file is missing",
                false,
            )
        })?;
        ensure_descendant(&root, &candidate)?;
        return Ok(candidate);
    }
    let parent = candidate.parent().ok_or_else(|| {
        DownloadPipelineError::new(
            DownloadPipelineErrorCode::PathOutsideRoot,
            "The artifact path has no managed parent",
            false,
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|_| filesystem_error("The managed artifact directory could not be created"))?;
    let parent = parent
        .canonicalize()
        .map_err(|_| filesystem_error("The managed artifact directory could not be resolved"))?;
    if parent != root {
        ensure_descendant(&root, &parent)?;
    }
    Ok(candidate)
}

fn ensure_descendant(root: &Path, candidate: &Path) -> Result<(), DownloadPipelineError> {
    if candidate == root || !candidate.starts_with(root) {
        return Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::PathOutsideRoot,
            "The managed artifact path escapes the configured download folder",
            false,
        ));
    }
    Ok(())
}

fn ensure_not_cancelled(cancellation: &CancellationToken) -> Result<(), DownloadPipelineError> {
    if cancellation.is_cancelled() {
        Err(DownloadPipelineError::cancelled())
    } else {
        Ok(())
    }
}

fn read_manifest_file(path: &Path) -> Result<ArtifactManifest, DownloadPipelineError> {
    let file = File::open(path)
        .map_err(|_| filesystem_error("The artifact manifest could not be opened"))?;
    serde_json::from_reader(file).map_err(|_| {
        DownloadPipelineError::new(
            DownloadPipelineErrorCode::ManifestInvalid,
            "The artifact manifest is malformed or uses an unsupported schema",
            false,
        )
    })
}

fn atomic_replace(source: &Path, destination: &Path) -> Result<(), DownloadPipelineError> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::PCWSTR,
            Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            },
        };

        let source = source
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let destination = destination
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        unsafe {
            MoveFileExW(
                PCWSTR(source.as_ptr()),
                PCWSTR(destination.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|_| filesystem_error("The artifact manifest could not be finalized atomically"))?;
        Ok(())
    }

    #[cfg(not(windows))]
    {
        fs::rename(source, destination).map_err(|_| {
            filesystem_error("The artifact manifest could not be finalized atomically")
        })
    }
}

fn open_default_viewer(path: &Path) -> Result<(), DownloadPipelineError> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            core::{w, PCWSTR},
            Win32::{
                Foundation::HWND,
                UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
            },
        };

        let path = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let result = unsafe {
            ShellExecuteW(
                Some(HWND::default()),
                w!("open"),
                PCWSTR(path.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize <= 32 {
            return Err(DownloadPipelineError::new(
                DownloadPipelineErrorCode::Filesystem,
                "Windows could not open the selected artifact item",
                false,
            ));
        }
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = path;
        Err(DownloadPipelineError::new(
            DownloadPipelineErrorCode::Filesystem,
            "Opening artifact files and folders is supported only on Windows",
            false,
        ))
    }
}

fn now_unix_ms() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|_| "0".to_owned())
}

fn filesystem_error(message: &'static str) -> DownloadPipelineError {
    DownloadPipelineError::new(DownloadPipelineErrorCode::Filesystem, message, true)
}

fn invalid_path(_detail: String) -> DownloadPipelineError {
    DownloadPipelineError::new(
        DownloadPipelineErrorCode::PathOutsideRoot,
        "The artifact path is outside the configured download folder",
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_page() -> DownloadPagePayload {
        let image = image::DynamicImage::new_rgba8(1, 1);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        DownloadPagePayload {
            source_page_number: SourcePageNumber::new(1).unwrap(),
            bytes: bytes.into_inner(),
            source_revision: "fixture-v1".into(),
            source_format: DownloadSourceImageFormat::Png,
            width: 1,
            height: 1,
            candidate_index: 0,
            candidate_diagnostics: Vec::new(),
        }
    }

    #[test]
    fn prepare_layout_reports_an_occupied_destination_without_modifying_it() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("downloads");
        fs::create_dir(&root).unwrap();
        let relative = ArtifactRelativePath::new("Existing 42").unwrap();
        let occupied = root.join(relative.as_str());
        fs::create_dir(&occupied).unwrap();
        fs::write(occupied.join("owned-by-user.txt"), b"keep").unwrap();

        let error = FilesystemArtifactStore::new()
            .prepare_layout(&root, &relative, false)
            .unwrap_err();

        assert_eq!(error.code, DownloadPipelineErrorCode::DestinationOccupied);
        assert!(!error.retryable);
        assert_eq!(
            fs::read(occupied.join("owned-by-user.txt")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn artifact_directory_path_requires_an_existing_root_bound_folder() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("downloads");
        fs::create_dir(&root).unwrap();
        let relative = ArtifactRelativePath::new("gallery-42").unwrap();
        let store = FilesystemArtifactStore::new();

        let missing = store.artifact_directory_path(&root, &relative).unwrap_err();
        assert_eq!(missing.code, DownloadPipelineErrorCode::ArtifactMissing);

        let expected = root.join(relative.as_str());
        fs::create_dir(&expected).unwrap();
        assert_eq!(
            store.artifact_directory_path(&root, &relative).unwrap(),
            expected.canonicalize().unwrap()
        );

        fs::remove_dir(&expected).unwrap();
        fs::write(&expected, b"not a directory").unwrap();
        let not_a_directory = store.artifact_directory_path(&root, &relative).unwrap_err();
        assert_eq!(
            not_a_directory.code,
            DownloadPipelineErrorCode::ArtifactMissing
        );
    }

    #[test]
    fn page_without_checkpoint_is_moved_to_unique_recovery_storage() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("downloads");
        fs::create_dir(&root).unwrap();
        let relative = ArtifactRelativePath::new("reserved-42").unwrap();
        let store = FilesystemArtifactStore::new();
        let layout = store.prepare_layout(&root, &relative, false).unwrap();
        fs::write(root.join("reserved-42").join("0001.webp"), b"ambiguous").unwrap();

        assert!(matches!(
            store
                .verify_existing_page(
                    &layout,
                    SourcePageNumber::new(1).unwrap(),
                    "fixture-v1",
                    None,
                )
                .unwrap(),
            ExistingPageVerification::Invalid { .. }
        ));
        assert!(!root.join("reserved-42").join("0001.webp").exists());
        let conflicts = fs::read_dir(root.join(".atsumi-recovery").join("conflicts"))
            .unwrap()
            .count();
        assert_eq!(conflicts, 1);
    }

    #[test]
    fn checkpoint_reuse_streams_hash_and_preserves_corruption_for_recovery() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("downloads");
        fs::create_dir(&root).unwrap();
        let store = FilesystemArtifactStore::new();
        let relative = ArtifactRelativePath::new("reserved-43").unwrap();
        let layout = store.prepare_layout(&root, &relative, false).unwrap();
        let source_page = SourcePageNumber::new(1).unwrap();
        let stored = store
            .store_page(&layout, &png_page(), &CancellationToken::new())
            .unwrap();

        assert!(matches!(
            store
                .verify_existing_page(
                    &layout,
                    source_page,
                    &stored.source_revision,
                    Some(&stored),
                )
                .unwrap(),
            ExistingPageVerification::Verified(ref verified)
                if verified.byte_length == stored.byte_length && verified.sha256 == stored.sha256
        ));

        let final_path = root.join("reserved-43").join("0001.webp");
        let mut corrupted = fs::read(&final_path).unwrap();
        let last = corrupted.last_mut().expect("stored WebP is non-empty");
        *last ^= 0x01;
        fs::write(&final_path, corrupted).unwrap();

        assert!(matches!(
            store
                .verify_existing_page(&layout, source_page, &stored.source_revision, Some(&stored),)
                .unwrap(),
            ExistingPageVerification::Invalid { .. }
        ));
        assert!(!final_path.exists());
        assert_eq!(
            fs::read_dir(root.join(".atsumi-recovery").join("conflicts"))
                .unwrap()
                .count(),
            1
        );
    }

    #[cfg(windows)]
    #[test]
    fn page_part_symlink_never_truncates_its_external_target() {
        use std::os::windows::fs::symlink_file;

        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("downloads");
        fs::create_dir(&root).unwrap();
        let store = FilesystemArtifactStore::new();
        let relative = ArtifactRelativePath::new("reserved-42").unwrap();
        let layout = store.prepare_layout(&root, &relative, false).unwrap();
        let outside = temporary.path().join("outside.txt");
        fs::write(&outside, b"must remain unchanged").unwrap();
        let part = root.join("reserved-42").join(".0001.webp.part");
        if symlink_file(&outside, &part).is_err() {
            return;
        }

        let error = store
            .store_page(&layout, &png_page(), &CancellationToken::new())
            .unwrap_err();
        assert_eq!(error.code, DownloadPipelineErrorCode::PathOutsideRoot);
        assert_eq!(fs::read(outside).unwrap(), b"must remain unchanged");
    }
}
