use serde::{Deserialize, Serialize};

use crate::source::SourceContractError;

pub const HITOMI_ORIGIN: &str = "https://hitomi.la";
pub const HITOMI_CONTENT_DOMAIN: &str = "gold-usergeneratedcontent.net";

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SourceRevision(String);

impl SourceRevision {
    pub(crate) fn gallery(gallery_id: u64, fingerprint: u64) -> Self {
        Self(format!("hitomi-gallery-v1:{gallery_id}:{fingerprint:016x}"))
    }

    pub(crate) fn page(hash: &str) -> Self {
        Self(format!("hitomi-page-v1:{hash}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl std::fmt::Display for SourceRevision {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitomiTagKind {
    General,
    Female,
    Male,
}

impl HitomiTagKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Female => "female",
            Self::Male => "male",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitomiTag {
    pub name: String,
    pub kind: HitomiTagKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitomiPageFile {
    /// Immutable one-based page identity from the source gallery.
    pub source_page: u32,
    pub name: String,
    pub hash: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// `None` is distinct from `false`: current galleryinfo payloads can omit
    /// `haswebp` while the WebP endpoints remain addressable.
    pub has_webp: Option<bool>,
    pub has_avif: bool,
    pub has_jxl: bool,
    pub source_revision: SourceRevision,
}

impl HitomiPageFile {
    pub fn aspect_ratio(&self) -> Option<f64> {
        Some(f64::from(self.width?) / f64::from(self.height?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitomiGalleryMetadata {
    pub id: u64,
    pub title: String,
    pub alternate_title: Option<String>,
    pub gallery_type: Option<String>,
    pub language: Option<String>,
    pub published_at: Option<String>,
    pub gallery_path: Option<String>,
    pub artists: Vec<String>,
    pub groups: Vec<String>,
    pub series: Vec<String>,
    pub characters: Vec<String>,
    pub tags: Vec<HitomiTag>,
    pub related_gallery_ids: Vec<u64>,
    pub pages: Vec<HitomiPageFile>,
    pub source_revision: SourceRevision,
}

impl HitomiGalleryMetadata {
    pub fn summary(&self) -> HitomiGallerySummary {
        HitomiGallerySummary {
            id: self.id,
            title: self.title.clone(),
            primary_artist: self.artists.first().cloned(),
            primary_group: self.groups.first().cloned(),
            series: self.series.clone(),
            characters: self.characters.clone(),
            language: self.language.clone(),
            page_count: self.pages.len() as u32,
            tags: self.tags.clone(),
            cover: self.pages.first().cloned(),
            source_url: self.source_url(),
            source_revision: self.source_revision.clone(),
        }
    }

    pub fn detail(&self) -> HitomiGalleryDetail {
        HitomiGalleryDetail {
            id: self.id,
            title: self.title.clone(),
            alternate_title: self.alternate_title.clone(),
            gallery_type: self.gallery_type.clone(),
            language: self.language.clone(),
            published_at: self.published_at.clone(),
            artists: self.artists.clone(),
            groups: self.groups.clone(),
            series: self.series.clone(),
            characters: self.characters.clone(),
            tags: self.tags.clone(),
            related_gallery_ids: self.related_gallery_ids.clone(),
            pages: self.pages.clone(),
            source_url: self.source_url(),
            source_revision: self.source_revision.clone(),
        }
    }

    pub fn page(&self, source_page: u32) -> Result<&HitomiPageFile, SourceContractError> {
        if source_page == 0 {
            return Err(SourceContractError::validation(
                "sourcePage",
                "must be one-based",
            ));
        }

        self.pages
            .get(source_page.saturating_sub(1) as usize)
            .ok_or_else(|| {
                SourceContractError::not_found(
                    format!("gallery {} source page {source_page}", self.id),
                    None,
                )
            })
    }

    pub fn source_url(&self) -> String {
        self.gallery_path.as_ref().map_or_else(
            || format!("{HITOMI_ORIGIN}/galleries/{}.html", self.id),
            |path| format!("{HITOMI_ORIGIN}{path}"),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitomiGallerySummary {
    pub id: u64,
    pub title: String,
    pub primary_artist: Option<String>,
    pub primary_group: Option<String>,
    pub series: Vec<String>,
    pub characters: Vec<String>,
    pub language: Option<String>,
    pub page_count: u32,
    pub tags: Vec<HitomiTag>,
    pub cover: Option<HitomiPageFile>,
    pub source_url: String,
    pub source_revision: SourceRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HitomiGalleryDetail {
    pub id: u64,
    pub title: String,
    pub alternate_title: Option<String>,
    pub gallery_type: Option<String>,
    pub language: Option<String>,
    pub published_at: Option<String>,
    pub artists: Vec<String>,
    pub groups: Vec<String>,
    pub series: Vec<String>,
    pub characters: Vec<String>,
    pub tags: Vec<HitomiTag>,
    pub related_gallery_ids: Vec<u64>,
    pub pages: Vec<HitomiPageFile>,
    pub source_url: String,
    pub source_revision: SourceRevision,
}

pub(crate) struct RevisionFingerprint(u64);

impl RevisionFingerprint {
    // Stable FNV-1a is sufficient here: this token invalidates caches and is not
    // used as a cryptographic integrity digest.
    pub(crate) const fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn field(&mut self, bytes: impl AsRef<[u8]>) {
        let bytes = bytes.as_ref();
        self.raw(&(bytes.len() as u64).to_be_bytes());
        self.raw(bytes);
    }

    pub(crate) fn boolean(&mut self, value: bool) {
        self.field([u8::from(value)]);
    }

    pub(crate) fn number(&mut self, value: u64) {
        self.field(value.to_be_bytes());
    }

    pub(crate) const fn finish(self) -> u64 {
        self.0
    }

    fn raw(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}
