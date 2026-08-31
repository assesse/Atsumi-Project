use std::fmt;

use serde::Serialize;

use super::{Language, ValidationError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct GalleryId(i64);

impl GalleryId {
    pub fn new(value: i64) -> Result<Self, ValidationError> {
        if value <= 0 {
            return Err(ValidationError::new(
                "galleryId",
                "must be a positive integer",
            ));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> i64 {
        self.0
    }
}

impl fmt::Display for GalleryId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourcePageNumber(u32);

impl SourcePageNumber {
    pub fn new(value: u32) -> Result<Self, ValidationError> {
        if value == 0 {
            return Err(ValidationError::new(
                "sourcePageNumber",
                "must be one-based",
            ));
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for SourcePageNumber {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GalleryPageId {
    pub gallery_id: GalleryId,
    pub source_page_number: SourcePageNumber,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GalleryMetadata {
    pub title: String,
    pub primary_artist: Option<String>,
    pub artists: Vec<String>,
    pub primary_group: Option<String>,
    pub source_page_count: u32,
    /// Persisted list presentation metadata. Legacy artifacts legitimately
    /// omit these values and are enriched when their source metadata is next
    /// resolved.
    pub language: Option<Language>,
    pub published_rank: Option<u32>,
}

impl GalleryMetadata {
    pub fn new(
        title: impl Into<String>,
        primary_artist: Option<String>,
        primary_group: Option<String>,
        source_page_count: u32,
    ) -> Result<Self, ValidationError> {
        let title = title.into().trim().to_owned();
        if title.is_empty() {
            return Err(ValidationError::new("title", "must not be empty"));
        }
        if source_page_count == 0 {
            return Err(ValidationError::new(
                "sourcePageCount",
                "must be greater than zero",
            ));
        }
        let primary_artist = normalized_optional_metadata(primary_artist);
        let primary_group = normalized_optional_metadata(primary_group);

        Ok(Self {
            title,
            artists: primary_artist.iter().cloned().collect(),
            primary_artist,
            primary_group,
            source_page_count,
            language: None,
            published_rank: None,
        })
    }

    /// Adds source presentation fields without widening the artifact naming
    /// contract. They are used by the local Downloads list projection only.
    pub fn with_list_presentation(
        mut self,
        language: Option<Language>,
        published_rank: Option<u32>,
    ) -> Self {
        self.language = language;
        self.published_rank = published_rank;
        self
    }

    /// Replaces the source artist list while retaining the established primary
    /// artist projection for legacy storage and folder naming.
    pub fn with_artists(mut self, artists: Vec<String>) -> Self {
        let mut normalized = artists
            .into_iter()
            .map(|artist| artist.trim().to_owned())
            .filter(|artist| !artist.is_empty())
            .collect::<Vec<_>>();
        normalized.sort_unstable();
        normalized.dedup();
        if self.primary_artist.is_none() {
            self.primary_artist = normalized.first().cloned();
        }
        if normalized.is_empty() {
            normalized.extend(self.primary_artist.iter().cloned());
        }
        self.artists = normalized;
        self
    }
}

fn normalized_optional_metadata(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_owned();
        (!value.is_empty()).then_some(value)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gallery {
    pub id: GalleryId,
    pub revision: u64,
    pub metadata: GalleryMetadata,
}

impl Gallery {
    pub fn new(id: GalleryId, revision: u64, metadata: GalleryMetadata) -> Self {
        Self {
            id,
            revision,
            metadata,
        }
    }
}
