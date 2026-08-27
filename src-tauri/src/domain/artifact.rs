use std::{collections::BTreeSet, fmt, path::Component, str::FromStr};

use serde::{Deserialize, Serialize};

use super::{Gallery, GalleryId, GalleryPageId, SourcePageNumber, ValidationError};

pub const ARTIFACT_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const HASH_PROFILE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactSha256(String);

impl ArtifactSha256 {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into();
        if value.len() != 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ValidationError::new(
                "sha256",
                "must be a lowercase 64-character hexadecimal digest",
            ));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ArtifactSha256 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStorageFormat {
    Webp,
}

impl ArtifactStorageFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Webp => "webp",
        }
    }
}

impl FromStr for ArtifactStorageFormat {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "webp" => Ok(Self::Webp),
            _ => Err(ValidationError::new(
                "storageFormat",
                "contains an unsupported value",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct DownloadEntryId(String);

impl DownloadEntryId {
    pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
        let value = value.into().trim().to_owned();
        if value.is_empty() {
            return Err(ValidationError::new("entryId", "must not be empty"));
        }
        if value.len() > 200 {
            return Err(ValidationError::new("entryId", "must be at most 200 bytes"));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DownloadEntryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArtifactRelativePath(String);

impl ArtifactRelativePath {
    pub fn new(value: impl AsRef<str>) -> Result<Self, ValidationError> {
        let path = std::path::Path::new(value.as_ref().trim());
        let mut parts = Vec::new();

        for component in path.components() {
            match component {
                Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(ValidationError::new(
                        "relativePath",
                        "must stay within the configured download root",
                    ));
                }
            }
        }

        if parts.is_empty() {
            return Err(ValidationError::new("relativePath", "must not be empty"));
        }
        Ok(Self(parts.join("/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn is_descendant_of(&self, directory: &Self) -> bool {
        self.0
            .strip_prefix(&directory.0)
            .is_some_and(|suffix| suffix.starts_with('/'))
    }
}

impl fmt::Display for ArtifactRelativePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadArtifactState {
    Incomplete,
    Complete,
    MissingArtifacts,
    Quarantined,
}

impl DownloadArtifactState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Incomplete => "incomplete",
            Self::Complete => "complete",
            Self::MissingArtifacts => "missing_artifacts",
            Self::Quarantined => "quarantined",
        }
    }
}

impl FromStr for DownloadArtifactState {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "incomplete" => Ok(Self::Incomplete),
            "complete" => Ok(Self::Complete),
            "missing_artifacts" => Ok(Self::MissingArtifacts),
            "quarantined" => Ok(Self::Quarantined),
            _ => Err(ValidationError::new(
                "artifactState",
                "contains an unsupported value",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageArtifactState {
    Pending,
    Present,
    Missing,
    Quarantined,
}

impl PageArtifactState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Present => "present",
            Self::Missing => "missing",
            Self::Quarantined => "quarantined",
        }
    }
}

impl FromStr for PageArtifactState {
    type Err = ValidationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "pending" => Ok(Self::Pending),
            "present" => Ok(Self::Present),
            "missing" => Ok(Self::Missing),
            "quarantined" => Ok(Self::Quarantined),
            _ => Err(ValidationError::new(
                "pageArtifactState",
                "contains an unsupported value",
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadArtifact {
    pub entry_id: DownloadEntryId,
    pub gallery_id: GalleryId,
    pub revision: u64,
    pub relative_directory: ArtifactRelativePath,
    pub expected_page_count: u32,
    pub state: DownloadArtifactState,
    pub manifest_relative_path: Option<ArtifactRelativePath>,
    pub manifest_schema_version: Option<u32>,
    pub writer_version: Option<String>,
    pub hash_profile_version: u32,
    pub completed_at: Option<String>,
}

impl DownloadArtifact {
    pub fn new(
        entry_id: DownloadEntryId,
        gallery_id: GalleryId,
        revision: u64,
        relative_directory: ArtifactRelativePath,
        expected_page_count: u32,
        state: DownloadArtifactState,
    ) -> Result<Self, ValidationError> {
        if expected_page_count == 0 {
            return Err(ValidationError::new(
                "expectedPageCount",
                "must be greater than zero",
            ));
        }
        Ok(Self {
            entry_id,
            gallery_id,
            revision,
            relative_directory,
            expected_page_count,
            state,
            manifest_relative_path: None,
            manifest_schema_version: None,
            writer_version: None,
            hash_profile_version: HASH_PROFILE_VERSION,
            completed_at: None,
        })
    }

    pub fn with_manifest(
        mut self,
        relative_path: ArtifactRelativePath,
        schema_version: u32,
        writer_version: impl Into<String>,
        hash_profile_version: u32,
        completed_at: impl Into<String>,
    ) -> Result<Self, ValidationError> {
        let writer_version = writer_version.into().trim().to_owned();
        let completed_at = completed_at.into().trim().to_owned();
        if schema_version == 0 {
            return Err(ValidationError::new(
                "manifestSchemaVersion",
                "must be greater than zero",
            ));
        }
        if hash_profile_version == 0 {
            return Err(ValidationError::new(
                "hashProfileVersion",
                "must be greater than zero",
            ));
        }
        if writer_version.is_empty() {
            return Err(ValidationError::new("writerVersion", "must not be empty"));
        }
        if completed_at.is_empty() {
            return Err(ValidationError::new("completedAt", "must not be empty"));
        }
        self.manifest_relative_path = Some(relative_path);
        self.manifest_schema_version = Some(schema_version);
        self.writer_version = Some(writer_version);
        self.hash_profile_version = hash_profile_version;
        self.completed_at = Some(completed_at);
        Ok(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageArtifact {
    pub entry_id: DownloadEntryId,
    pub page_id: GalleryPageId,
    pub relative_path: ArtifactRelativePath,
    pub state: PageArtifactState,
    pub byte_length: Option<u64>,
    pub sha256: Option<ArtifactSha256>,
    pub storage_format: Option<ArtifactStorageFormat>,
    pub source_revision: Option<String>,
    pub verified_at: Option<String>,
    pub excluded: bool,
}

impl PageArtifact {
    pub fn new(
        entry_id: DownloadEntryId,
        gallery_id: GalleryId,
        source_page_number: SourcePageNumber,
        relative_path: ArtifactRelativePath,
        state: PageArtifactState,
        byte_length: Option<u64>,
    ) -> Result<Self, ValidationError> {
        if matches!(byte_length, Some(0)) {
            return Err(ValidationError::new(
                "byteLength",
                "must be greater than zero when known",
            ));
        }
        Ok(Self {
            entry_id,
            page_id: GalleryPageId {
                gallery_id,
                source_page_number,
            },
            relative_path,
            state,
            byte_length,
            sha256: None,
            storage_format: None,
            source_revision: None,
            verified_at: None,
            excluded: false,
        })
    }

    pub fn with_verification(
        mut self,
        sha256: ArtifactSha256,
        storage_format: ArtifactStorageFormat,
        source_revision: impl Into<String>,
        verified_at: impl Into<String>,
    ) -> Result<Self, ValidationError> {
        let source_revision = source_revision.into().trim().to_owned();
        let verified_at = verified_at.into().trim().to_owned();
        if !matches!(
            self.state,
            PageArtifactState::Present | PageArtifactState::Quarantined
        ) || self.byte_length.is_none()
        {
            return Err(ValidationError::new(
                "pageArtifactState",
                "verification requires a present or quarantined page with a byte length",
            ));
        }
        if source_revision.is_empty() {
            return Err(ValidationError::new("sourceRevision", "must not be empty"));
        }
        if verified_at.is_empty() {
            return Err(ValidationError::new("verifiedAt", "must not be empty"));
        }
        self.sha256 = Some(sha256);
        self.storage_format = Some(storage_format);
        self.source_revision = Some(source_revision);
        self.verified_at = Some(verified_at);
        Ok(self)
    }

    pub fn with_excluded(mut self, excluded: bool) -> Self {
        self.excluded = excluded;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactManifest {
    pub schema_version: u32,
    pub writer_version: String,
    pub hash_profile_version: u32,
    pub conversion_policy: ArtifactConversionPolicy,
    pub gallery: ArtifactManifestGallery,
    pub expected_page_count: u32,
    pub pages: Vec<ArtifactManifestPage>,
    pub completed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactConversionPolicy {
    pub storage_format: ArtifactStorageFormat,
    pub webp_encoding: String,
    pub alpha_policy: String,
    pub animation_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactManifestGallery {
    pub gallery_id: i64,
    pub title: String,
    pub primary_artist: Option<String>,
    pub primary_group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactManifestPage {
    pub source_page_number: u32,
    pub relative_path: String,
    pub byte_length: u64,
    pub sha256: ArtifactSha256,
    pub storage_format: ArtifactStorageFormat,
    pub source_revision: String,
    pub excluded: bool,
    pub quarantined: bool,
}

impl ArtifactManifest {
    pub fn from_bundle(bundle: &ArtifactBundle) -> Result<Self, ValidationError> {
        bundle.validate()?;
        let artifact = &bundle.artifact;
        if artifact.state != DownloadArtifactState::Complete {
            return Err(ValidationError::new(
                "artifactState",
                "manifest requires a complete artifact",
            ));
        }
        let mut pages = Vec::with_capacity(bundle.pages.len());
        for page in &bundle.pages {
            pages.push(ArtifactManifestPage {
                source_page_number: page.page_id.source_page_number.get(),
                relative_path: page.relative_path.as_str().to_owned(),
                byte_length: page.byte_length.ok_or_else(|| {
                    ValidationError::new("byteLength", "complete pages require a byte length")
                })?,
                sha256: page.sha256.clone().ok_or_else(|| {
                    ValidationError::new("sha256", "complete pages require a digest")
                })?,
                storage_format: page.storage_format.ok_or_else(|| {
                    ValidationError::new("storageFormat", "complete pages require a format")
                })?,
                source_revision: page.source_revision.clone().ok_or_else(|| {
                    ValidationError::new(
                        "sourceRevision",
                        "complete pages require a source revision",
                    )
                })?,
                excluded: page.excluded,
                quarantined: page.state == PageArtifactState::Quarantined,
            });
        }
        Ok(Self {
            schema_version: artifact.manifest_schema_version.ok_or_else(|| {
                ValidationError::new(
                    "manifestSchemaVersion",
                    "complete artifacts require a manifest version",
                )
            })?,
            writer_version: artifact.writer_version.clone().ok_or_else(|| {
                ValidationError::new(
                    "writerVersion",
                    "complete artifacts require a writer version",
                )
            })?,
            hash_profile_version: artifact.hash_profile_version,
            conversion_policy: ArtifactConversionPolicy {
                storage_format: ArtifactStorageFormat::Webp,
                webp_encoding: "preserve_verified_source_or_lossless_rgba".into(),
                alpha_policy: "preserve".into(),
                animation_policy: "preserve_source_webp_otherwise_first_frame".into(),
            },
            gallery: ArtifactManifestGallery {
                gallery_id: bundle.gallery.id.get(),
                title: bundle.gallery.metadata.title.clone(),
                primary_artist: bundle.gallery.metadata.primary_artist.clone(),
                primary_group: bundle.gallery.metadata.primary_group.clone(),
            },
            expected_page_count: artifact.expected_page_count,
            pages,
            completed_at: artifact.completed_at.clone().ok_or_else(|| {
                ValidationError::new(
                    "completedAt",
                    "complete artifacts require a completion time",
                )
            })?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactBundle {
    pub gallery: Gallery,
    pub artifact: DownloadArtifact,
    pub pages: Vec<PageArtifact>,
}

impl ArtifactBundle {
    pub fn new(
        gallery: Gallery,
        artifact: DownloadArtifact,
        pages: Vec<PageArtifact>,
    ) -> Result<Self, ValidationError> {
        let bundle = Self {
            gallery,
            artifact,
            pages,
        };
        bundle.validate()?;
        Ok(bundle)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.gallery.id != self.artifact.gallery_id {
            return Err(ValidationError::new(
                "galleryId",
                "gallery and download artifact must match",
            ));
        }
        let mut page_numbers = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for page in &self.pages {
            if page.entry_id != self.artifact.entry_id {
                return Err(ValidationError::new(
                    "entryId",
                    "page and download artifact must match",
                ));
            }
            if page.page_id.gallery_id != self.gallery.id {
                return Err(ValidationError::new(
                    "galleryId",
                    "page and gallery must match",
                ));
            }
            if page.page_id.source_page_number.get() > self.artifact.expected_page_count {
                return Err(ValidationError::new(
                    "sourcePageNumber",
                    "must not exceed the expected page count",
                ));
            }
            if !page_numbers.insert(page.page_id.source_page_number) {
                return Err(ValidationError::new(
                    "sourcePageNumber",
                    "must be unique within a download artifact",
                ));
            }
            if !paths.insert(&page.relative_path) {
                return Err(ValidationError::new(
                    "relativePath",
                    "must be unique within a download artifact",
                ));
            }
            if !page
                .relative_path
                .is_descendant_of(&self.artifact.relative_directory)
            {
                return Err(ValidationError::new(
                    "relativePath",
                    "page path must be inside the download artifact directory",
                ));
            }
        }

        if self.artifact.state == DownloadArtifactState::Complete {
            let manifest_ready = self.artifact.manifest_relative_path.is_some()
                && self.artifact.manifest_schema_version.is_some()
                && self.artifact.writer_version.is_some()
                && self.artifact.completed_at.is_some();
            let pages_ready = self.pages.len() == self.artifact.expected_page_count as usize
                && self.pages.iter().all(|page| {
                    ((page.state == PageArtifactState::Present && !page.excluded)
                        || (page.state == PageArtifactState::Quarantined && page.excluded))
                        && page.byte_length.is_some()
                        && page.sha256.is_some()
                        && page.storage_format.is_some()
                        && page.source_revision.is_some()
                        && page.verified_at.is_some()
                });
            if !manifest_ready || !pages_ready {
                return Err(ValidationError::new(
                    "artifactState",
                    "complete artifacts require a manifest and every verified source page",
                ));
            }
        }
        Ok(())
    }
}
