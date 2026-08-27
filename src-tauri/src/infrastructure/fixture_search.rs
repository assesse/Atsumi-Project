use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{Mutex, MutexGuard},
};

use serde::Deserialize;

use crate::{
    application::{RepositoryError, SearchRepository},
    domain::{
        GalleryDetail, GalleryId, GalleryPage, GalleryPageDimension, GallerySummary, Language,
        SearchRequest, SearchSort, SearchSubmission,
    },
};

const SEARCH_FIXTURE: &str = include_str!("../../fixtures/search_galleries.json");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FixtureGallery {
    id: i64,
    title: String,
    artist: String,
    group: Option<String>,
    #[serde(default)]
    series: Vec<String>,
    #[serde(default)]
    characters: Vec<String>,
    pages: u32,
    language: Language,
    tags: Vec<String>,
    published_rank: u32,
    popularity: u32,
    related: Vec<i64>,
}

pub struct FixtureSearchRepository {
    galleries: BTreeMap<GalleryId, FixtureGallery>,
    queries: Mutex<HashMap<String, SearchRequest>>,
}

impl FixtureSearchRepository {
    pub fn new() -> Result<Self, RepositoryError> {
        Self::from_json(SEARCH_FIXTURE)
    }

    fn from_json(json: &str) -> Result<Self, RepositoryError> {
        let fixtures: Vec<FixtureGallery> = serde_json::from_str(json).map_err(|error| {
            RepositoryError::Corrupt(format!("search fixture is invalid: {error}"))
        })?;
        let mut galleries = BTreeMap::new();

        for mut fixture in fixtures {
            let id = GalleryId::new(fixture.id).map_err(fixture_corruption)?;
            fixture.title = required_fixture_text(fixture.title, "title")?;
            fixture.artist = required_fixture_text(fixture.artist, "artist")?;
            fixture.group = normalized_fixture_text(fixture.group);
            fixture.series = normalized_fixture_names(fixture.series);
            fixture.characters = normalized_fixture_names(fixture.characters);
            if fixture.pages == 0 {
                return Err(RepositoryError::Corrupt(format!(
                    "search fixture gallery {id} has no pages"
                )));
            }
            fixture.tags = fixture
                .tags
                .into_iter()
                .map(|tag| tag.trim().to_lowercase())
                .filter(|tag| !tag.is_empty())
                .collect();
            fixture.tags.sort_unstable();
            fixture.tags.dedup();
            if galleries.insert(id, fixture).is_some() {
                return Err(RepositoryError::Corrupt(format!(
                    "search fixture gallery {id} is duplicated"
                )));
            }
        }

        let known_ids: HashSet<i64> = galleries.keys().map(|id| id.get()).collect();
        for (id, gallery) in &galleries {
            if let Some(unknown) = gallery
                .related
                .iter()
                .find(|related_id| !known_ids.contains(related_id))
            {
                return Err(RepositoryError::Corrupt(format!(
                    "search fixture gallery {id} references unknown gallery {unknown}"
                )));
            }
        }

        Ok(Self {
            galleries,
            queries: Mutex::new(HashMap::new()),
        })
    }

    fn queries(&self) -> Result<MutexGuard<'_, HashMap<String, SearchRequest>>, RepositoryError> {
        self.queries
            .lock()
            .map_err(|_| RepositoryError::Other("search query mutex was poisoned".into()))
    }

    fn query_id(request: &SearchRequest) -> Result<String, RepositoryError> {
        let canonical = serde_json::to_vec(request).map_err(|error| {
            RepositoryError::Other(format!("could not serialize search query: {error}"))
        })?;
        let first = fnv1a(&canonical, 0xcbf2_9ce4_8422_2325);
        let second = fnv1a(&canonical, 0x8422_2325_cbf2_9ce4);
        Ok(format!("fixture-{first:016x}{second:016x}"))
    }

    fn page_for(&self, request: &SearchRequest, page: u32) -> GalleryPage {
        let mut matches: Vec<&FixtureGallery> = self
            .galleries
            .values()
            .filter(|gallery| gallery_matches(gallery, request))
            .collect();
        sort_galleries(&mut matches, request.sort);

        let page_size = request.page_size as usize;
        let total_pages = if matches.is_empty() {
            0
        } else {
            matches.len().div_ceil(page_size) as u32
        };
        let start = (page as usize - 1).saturating_mul(page_size);
        let items = matches
            .into_iter()
            .skip(start)
            .take(page_size)
            .map(gallery_summary)
            .collect();

        GalleryPage {
            page,
            total_pages,
            items,
        }
    }
}

impl SearchRepository for FixtureSearchRepository {
    fn search_submit(&self, request: &SearchRequest) -> Result<SearchSubmission, RepositoryError> {
        let query_id = Self::query_id(request)?;
        self.queries()?.insert(query_id.clone(), request.clone());
        Ok(SearchSubmission {
            query_id,
            first_page: self.page_for(request, 1),
        })
    }

    fn search_page_get(
        &self,
        query_id: &str,
        page: u32,
    ) -> Result<Option<GalleryPage>, RepositoryError> {
        let request = self.queries()?.get(query_id).cloned();
        Ok(request.map(|request| self.page_for(&request, page)))
    }

    fn gallery_detail_get(
        &self,
        gallery_id: GalleryId,
    ) -> Result<Option<GalleryDetail>, RepositoryError> {
        let Some(gallery) = self.galleries.get(&gallery_id) else {
            return Ok(None);
        };
        let related = gallery
            .related
            .iter()
            .filter_map(|id| GalleryId::new(*id).ok())
            .filter_map(|id| self.galleries.get(&id))
            .map(gallery_summary)
            .collect();

        Ok(Some(GalleryDetail {
            summary: gallery_summary(gallery),
            related,
            page_dimensions: (1..=gallery.pages)
                .map(|source_page| GalleryPageDimension {
                    source_page,
                    width: Some(512),
                    height: Some(512),
                })
                .collect(),
        }))
    }
}

fn required_fixture_text(value: String, field: &str) -> Result<String, RepositoryError> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(RepositoryError::Corrupt(format!(
            "search fixture {field} must not be empty"
        )))
    } else {
        Ok(value)
    }
}

fn normalized_fixture_text(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let value = value.trim().to_owned();
        (!value.is_empty()).then_some(value)
    })
}

fn normalized_fixture_names(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter_map(|value| normalized_fixture_text(Some(value)))
        .filter(|value| seen.insert(value.to_lowercase()))
        .collect()
}

fn fixture_corruption(error: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::Corrupt(format!("search fixture is invalid: {error}"))
}

fn gallery_matches(gallery: &FixtureGallery, request: &SearchRequest) -> bool {
    if !request.languages.is_empty() && !request.languages.contains(&gallery.language) {
        return false;
    }

    let tags: HashSet<&str> = gallery.tags.iter().map(String::as_str).collect();
    if request
        .include_tags
        .iter()
        .any(|tag| !tags.contains(tag.as_str()))
    {
        return false;
    }
    if request
        .exclude_tags
        .iter()
        .any(|tag| tags.contains(tag.as_str()))
    {
        return false;
    }
    if request.text.is_empty() {
        return true;
    }

    let artist = gallery.artist.to_lowercase();
    let mut searchable = format!(
        "{} {artist} artist:{artist} artist:{}",
        gallery.title.to_lowercase(),
        artist.replace(' ', "_")
    );
    if let Some(group) = &gallery.group {
        let group = group.to_lowercase();
        searchable.push_str(&format!(
            " {group} group:{group} group:{}",
            group.replace(' ', "_")
        ));
    }
    for series in &gallery.series {
        let series = series.to_lowercase();
        searchable.push_str(&format!(
            " {series} series:{series} series:{}",
            series.replace(' ', "_")
        ));
    }
    for character in &gallery.characters {
        let character = character.to_lowercase();
        searchable.push_str(&format!(
            " {character} character:{character} character:{}",
            character.replace(' ', "_")
        ));
    }
    for tag in &gallery.tags {
        searchable.push(' ');
        searchable.push_str(tag);
    }
    searchable.contains(&request.text)
}

fn sort_galleries(galleries: &mut [&FixtureGallery], sort: SearchSort) {
    match sort {
        SearchSort::Recent => galleries.sort_by_key(|gallery| {
            (
                std::cmp::Reverse(gallery.published_rank),
                std::cmp::Reverse(gallery.id),
            )
        }),
        SearchSort::PopularToday
        | SearchSort::PopularWeek
        | SearchSort::PopularMonth
        | SearchSort::PopularYear => galleries.sort_by_key(|gallery| {
            (
                std::cmp::Reverse(gallery.popularity),
                std::cmp::Reverse(gallery.id),
            )
        }),
        SearchSort::Random => galleries.sort_by_key(|gallery| stable_random_rank(gallery.id)),
    }
}

fn gallery_summary(gallery: &FixtureGallery) -> GallerySummary {
    let id = GalleryId::new(gallery.id).expect("validated fixture gallery ID");
    GallerySummary {
        id,
        title: gallery.title.clone(),
        artist: gallery.artist.clone(),
        group: gallery.group.clone(),
        series: gallery.series.clone(),
        characters: gallery.characters.clone(),
        pages: gallery.pages,
        language: gallery.language,
        tags: gallery.tags.clone(),
        published_rank: gallery.published_rank,
        popularity: gallery.popularity,
        thumbnail_key: Some(format!("fixture-gallery-{}-cover", gallery.id)),
        thumbnail_width: 512,
        thumbnail_height: 512,
    }
}

fn stable_random_rank(id: i64) -> u64 {
    let mut value = id as u64 ^ 0x9e37_79b9_7f4a_7c15;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn fnv1a(bytes: &[u8], offset: u64) -> u64 {
    bytes.iter().fold(offset, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}
