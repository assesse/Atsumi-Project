use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
};

use uuid::Uuid;

use crate::{
    application::{
        AutoFindSource, AutoFindSourceRequest, AutoFindSourceResult, RepositoryError,
        SearchRepository,
    },
    domain::{
        GalleryDetail, GalleryId, GalleryPage, GalleryPageDimension, GallerySummary, Language,
        SearchRequest, SearchSort, SearchSubmission,
    },
    source::{
        hitomi::{HitomiGalleryMetadata, HitomiTagKind},
        SourceContractError, SourceErrorCode,
    },
    thumbnail::CancellationToken,
};

use super::{check_cancelled, unpoison, HitomiLiveAdapter};

const AUTO_FIND_CANDIDATE_LIMIT: u32 = 50_000;

pub(super) struct QueryCache {
    capacity: usize,
    values: HashMap<String, Arc<QuerySnapshot>>,
    order: VecDeque<String>,
}

impl QueryCache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            values: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn insert(&mut self, id: String, query: Arc<QuerySnapshot>) {
        self.values.insert(id.clone(), query);
        self.order.retain(|candidate| candidate != &id);
        self.order.push_back(id);
        while self.values.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.values.remove(&oldest);
            }
        }
    }

    fn get(&mut self, id: &str) -> Option<Arc<QuerySnapshot>> {
        let value = Arc::clone(self.values.get(id)?);
        self.order.retain(|candidate| candidate != id);
        self.order.push_back(id.to_owned());
        Some(value)
    }

    pub(super) fn clear(&mut self) {
        self.values.clear();
        self.order.clear();
    }
}

struct QuerySnapshot {
    request: SearchRequest,
    candidate_ids: Vec<u64>,
    residual_terms: Vec<ResidualTerm>,
    progress: Mutex<QueryProgress>,
}

#[derive(Default)]
struct QueryProgress {
    cursor: usize,
    matches: Vec<GallerySummary>,
}

#[derive(Debug, Clone)]
struct ResidualTerm {
    value: String,
    negative: bool,
}

impl HitomiLiveAdapter {
    fn build_query_snapshot(
        &self,
        request: &SearchRequest,
    ) -> Result<Arc<QuerySnapshot>, SourceContractError> {
        let direct_gallery_id = direct_gallery_id(request);
        let languages = requested_languages(request);
        let mut ordered = match direct_gallery_id {
            Some(gallery_id) => vec![gallery_id],
            None => self.order_ids(request.sort, &languages)?,
        };
        let language_ids = ordered.iter().copied().collect::<HashSet<_>>();
        let mut structured = Vec::new();
        let mut residual_terms = Vec::new();

        for value in &request.include_tags {
            structured.push((tag_nozomi_path(value), false));
        }
        for value in &request.exclude_tags {
            structured.push((tag_nozomi_path(value), true));
        }
        for token in request
            .text
            .split_whitespace()
            .filter(|_| direct_gallery_id.is_none())
        {
            let (negative, token) = token
                .strip_prefix('-')
                .map_or((false, token), |value| (true, value));
            if token.is_empty() {
                continue;
            }
            if let Some(path) = prefixed_nozomi_path(token) {
                structured.push((Some(path), negative));
            } else {
                residual_terms.push(ResidualTerm {
                    value: normalize_text(token),
                    negative,
                });
            }
        }

        // The selected-language order is canonical for paging. Structured
        // indexes only remove IDs from that order, so recent/popular rank stays stable.
        ordered.retain(|id| language_ids.contains(id));
        for (path, negative) in structured {
            let Some(path) = path else {
                continue;
            };
            let ids = self
                .fetch_optional_nozomi_path(&path)?
                .into_iter()
                .collect::<HashSet<_>>();
            if negative {
                ordered.retain(|id| !ids.contains(id));
            } else {
                ordered.retain(|id| ids.contains(id));
            }
        }

        ordered.dedup();
        if request.sort == SearchSort::Random {
            // Keep paging stable inside one query snapshot while giving every new
            // random search an independent ordering across the full source index.
            // The previous fixed rank made a "random" first page repeat forever.
            let uuid = Uuid::new_v4().as_u128();
            let seed = uuid as u64 ^ (uuid >> 64) as u64;
            ordered.sort_by_key(|id| stable_random_rank(*id ^ seed));
        }
        ordered.truncate(self.config.max_candidate_ids);

        Ok(Arc::new(QuerySnapshot {
            request: request.clone(),
            candidate_ids: ordered,
            residual_terms,
            progress: Mutex::new(QueryProgress::default()),
        }))
    }

    fn order_ids(
        &self,
        sort: SearchSort,
        languages: &[Language],
    ) -> Result<Vec<u64>, SourceContractError> {
        let popular_key = match sort {
            SearchSort::PopularToday => Some("today"),
            SearchSort::PopularWeek => Some("week"),
            SearchSort::PopularMonth => Some("month"),
            SearchSort::PopularYear => Some("year"),
            SearchSort::Recent | SearchSort::Random => None,
        };
        let mut ids = Vec::new();
        for language in languages {
            let path = popular_key.map_or_else(
                || format!("n/index-{}.nozomi", language_slug(*language)),
                |key| format!("n/popular/{key}-{}.nozomi", language_slug(*language)),
            );
            ids.extend(self.fetch_optional_nozomi_path(&path)?);
        }
        if popular_key.is_none() {
            ids.sort_unstable_by(|left, right| right.cmp(left));
        }
        let mut seen = HashSet::new();
        ids.retain(|id| seen.insert(*id));
        Ok(ids)
    }

    fn query_page(
        &self,
        snapshot: &QuerySnapshot,
        page: u32,
    ) -> Result<GalleryPage, SourceContractError> {
        self.query_page_with_cancellation(snapshot, page, None)
    }

    fn query_page_with_cancellation(
        &self,
        snapshot: &QuerySnapshot,
        page: u32,
        cancellation: Option<&CancellationToken>,
    ) -> Result<GalleryPage, SourceContractError> {
        if page == 0 {
            return Err(SourceContractError::validation("page", "must be one-based"));
        }
        if let Some(cancellation) = cancellation {
            check_cancelled(cancellation)?;
        }
        let page_size = snapshot.request.page_size as usize;
        let wanted = page as usize * page_size;
        let mut progress = unpoison(snapshot.progress.lock());
        while progress.matches.len() < wanted && progress.cursor < snapshot.candidate_ids.len() {
            if let Some(cancellation) = cancellation {
                check_cancelled(cancellation)?;
            }
            let rank = progress.cursor;
            let id = snapshot.candidate_ids[rank];
            match self.fetch_metadata_with_cancellation(id, cancellation) {
                Ok(metadata) => {
                    if let Some(cancellation) = cancellation {
                        check_cancelled(cancellation)?;
                    }
                    if metadata_matches(&metadata, &snapshot.residual_terms) {
                        progress.matches.push(gallery_summary(
                            &metadata,
                            snapshot.request.sort,
                            rank,
                        )?);
                    }
                    progress.cursor += 1;
                }
                Err(error) if error.code == SourceErrorCode::NotFound => {
                    progress.cursor += 1;
                }
                Err(error) => return Err(error),
            }
        }

        let complete = progress.cursor == snapshot.candidate_ids.len();
        let total_items = if complete {
            progress.matches.len()
        } else {
            snapshot.candidate_ids.len()
        };
        let total_pages = if total_items == 0 {
            0
        } else {
            total_items.div_ceil(page_size) as u32
        };
        let start = (page as usize - 1).saturating_mul(page_size);
        let items = progress
            .matches
            .iter()
            .skip(start)
            .take(page_size)
            .cloned()
            .collect();
        Ok(GalleryPage {
            page,
            total_pages,
            items,
        })
    }
}

impl SearchRepository for HitomiLiveAdapter {
    fn search_submit(&self, request: &SearchRequest) -> Result<SearchSubmission, RepositoryError> {
        let snapshot = self.build_query_snapshot(request)?;
        let first_page = self.query_page(&snapshot, 1)?;
        let query_id = format!("hitomi-{}", Uuid::new_v4());
        unpoison(self.queries.lock()).insert(query_id.clone(), snapshot);
        Ok(SearchSubmission {
            query_id,
            first_page,
        })
    }

    fn search_page_get(
        &self,
        query_id: &str,
        page: u32,
    ) -> Result<Option<GalleryPage>, RepositoryError> {
        let snapshot = unpoison(self.queries.lock()).get(query_id);
        snapshot
            .map(|snapshot| {
                self.query_page(&snapshot, page)
                    .map_err(RepositoryError::Source)
            })
            .transpose()
    }

    fn search_page_get_cancellable(
        &self,
        query_id: &str,
        page: u32,
        cancellation: &CancellationToken,
    ) -> Result<Option<GalleryPage>, RepositoryError> {
        check_cancelled(cancellation)?;
        let snapshot = unpoison(self.queries.lock()).get(query_id);
        snapshot
            .map(|snapshot| {
                self.query_page_with_cancellation(&snapshot, page, Some(cancellation))
                    .map_err(RepositoryError::Source)
            })
            .transpose()
    }

    fn gallery_detail_get(
        &self,
        gallery_id: GalleryId,
    ) -> Result<Option<GalleryDetail>, RepositoryError> {
        let id = u64::try_from(gallery_id.get())
            .map_err(|_| SourceContractError::validation("galleryId", "must be positive"))?;
        let metadata = match self.fetch_metadata(id) {
            Ok(metadata) => metadata,
            Err(error) if error.code == SourceErrorCode::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let summary = gallery_summary(&metadata, SearchSort::Recent, 0)?;
        let mut related = Vec::new();
        for related_id in metadata
            .related_gallery_ids
            .iter()
            .copied()
            .filter(|related_id| *related_id != id)
            .take(self.config.related_gallery_limit)
        {
            match self.fetch_metadata(related_id) {
                Ok(metadata) => related.push(gallery_summary(&metadata, SearchSort::Recent, 0)?),
                Err(error) if error.code == SourceErrorCode::NotFound => {}
                Err(error) => {
                    // Related galleries are supplemental. A transient or
                    // malformed related item must not withhold the main
                    // gallery's page dimensions and leave Detail loading
                    // forever. Keep diagnostics sanitized to IDs and code.
                    tracing::debug!(
                        gallery_id = id,
                        related_gallery_id = related_id,
                        code = ?error.code,
                        "skipping unavailable related gallery metadata"
                    );
                }
            }
        }
        let mut known_pages = HashSet::new();
        let page_dimensions = metadata
            .pages
            .iter()
            .filter_map(|page| {
                (page.source_page > 0 && known_pages.insert(page.source_page)).then_some(
                    GalleryPageDimension {
                        source_page: page.source_page,
                        width: page.width.filter(|value| *value > 0),
                        height: page.height.filter(|value| *value > 0),
                    },
                )
            })
            .collect();
        Ok(Some(GalleryDetail {
            summary,
            related,
            page_dimensions,
        }))
    }
}

impl AutoFindSource for HitomiLiveAdapter {
    fn auto_find_artist_plan(
        &self,
        request: &AutoFindSourceRequest,
        cancellation: &CancellationToken,
    ) -> Result<AutoFindSourceResult, RepositoryError> {
        let artist_path = prefixed_nozomi_path(&format!("artist:{}", request.artist))
            .ok_or_else(|| SourceContractError::validation("artist", "must not be empty"))?;
        // Build the complete ID set first. Gallery metadata is deliberately
        // fetched only after language, artist and history constraints apply.
        let artist_ids = self
            .fetch_optional_nozomi_path_with_cancellation(&artist_path, cancellation)?
            .into_iter()
            .collect::<HashSet<_>>();
        let languages = if request.languages.is_empty() {
            vec![Language::Korean]
        } else {
            request.languages.clone()
        };
        let cutoff = request.newer_than_gallery_id.map(|id| id.get() as u64);
        let mut ids = Vec::new();
        for language in languages {
            check_auto_find_cancelled(cancellation)?;
            let path = format!("n/index-{}.nozomi", language_slug(language));
            ids.extend(self.fetch_optional_nozomi_path_with_cancellation(&path, cancellation)?);
        }
        ids.sort_unstable_by(|left, right| right.cmp(left));
        let mut seen = HashSet::new();
        ids.retain(|id| seen.insert(*id));
        ids.retain(|id| artist_ids.contains(id) && cutoff.is_none_or(|minimum| *id > minimum));
        ids.dedup();
        let eligible_count = u32::try_from(ids.len()).unwrap_or(u32::MAX);
        let bounded_limit = request.candidate_limit.min(AUTO_FIND_CANDIDATE_LIMIT);
        let limit = usize::try_from(bounded_limit).unwrap_or(usize::MAX);
        let truncated_reason =
            (ids.len() > limit).then_some("candidate_limit_after_cutoff".to_owned());
        ids.truncate(limit);

        Ok(AutoFindSourceResult {
            candidate_ids: ids
                .into_iter()
                .map(|id| {
                    let id = i64::try_from(id).map_err(|_| {
                        SourceContractError::invalid_data(
                            "gallery ID",
                            "does not fit the application domain",
                        )
                    })?;
                    GalleryId::new(id).map_err(|error| {
                        SourceContractError::invalid_data("gallery ID", error.to_string())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
            eligible_count,
            limit: bounded_limit,
            truncated_reason,
        })
    }

    fn auto_find_gallery_summary(
        &self,
        gallery_id: GalleryId,
        cancellation: &CancellationToken,
    ) -> Result<Option<GallerySummary>, RepositoryError> {
        check_auto_find_cancelled(cancellation)?;
        let source_id = u64::try_from(gallery_id.get())
            .map_err(|_| SourceContractError::validation("galleryId", "must be positive"))?;
        match self.fetch_metadata_with_cancellation(source_id, Some(cancellation)) {
            Ok(metadata) => {
                check_auto_find_cancelled(cancellation)?;
                Ok(Some(gallery_summary(&metadata, SearchSort::Recent, 0)?))
            }
            Err(error) if error.code == SourceErrorCode::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

fn direct_gallery_id(request: &SearchRequest) -> Option<u64> {
    let text = request.text.trim();
    (text.len() == 7 && text.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

fn check_auto_find_cancelled(cancellation: &CancellationToken) -> Result<(), SourceContractError> {
    if cancellation.is_cancelled() {
        Err(SourceContractError::cancelled())
    } else {
        Ok(())
    }
}

fn requested_languages(request: &SearchRequest) -> Vec<Language> {
    if request.languages.is_empty() {
        vec![Language::Korean]
    } else {
        request.languages.clone()
    }
}

fn language_slug(language: Language) -> &'static str {
    match language {
        Language::Korean => "korean",
        Language::Japanese => "japanese",
        Language::Chinese => "chinese",
        Language::English => "english",
    }
}

pub(super) fn tag_nozomi_path(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some((kind, name)) = value.split_once(':') {
        let kind = kind.trim().to_ascii_lowercase();
        if matches!(kind.as_str(), "female" | "male") {
            return Some(format!(
                "n/tag/{kind}%3A{}-all.nozomi",
                percent_encode(&normalize_text(name))
            ));
        }
    }
    let normalized = normalize_text(value);
    (!normalized.is_empty()).then(|| format!("n/tag/{}-all.nozomi", percent_encode(&normalized)))
}

pub(super) fn prefixed_nozomi_path(value: &str) -> Option<String> {
    let (prefix, name) = value.split_once(':')?;
    let prefix = prefix.trim().to_ascii_lowercase();
    let name = percent_encode(&normalize_text(name));
    if name.is_empty() {
        return None;
    }
    match prefix.as_str() {
        "female" | "male" => Some(format!("n/tag/{prefix}%3A{name}-all.nozomi")),
        "tag" => Some(format!("n/tag/{name}-all.nozomi")),
        "artist" => Some(format!("n/artist/{name}-all.nozomi")),
        "group" => Some(format!("n/group/{name}-all.nozomi")),
        "series" => Some(format!("n/series/{name}-all.nozomi")),
        "character" => Some(format!("n/character/{name}-all.nozomi")),
        _ => None,
    }
}

fn normalize_text(value: &str) -> String {
    value
        .trim()
        .to_lowercase()
        .replace("\\_", "_")
        .replace('_', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        let character = byte as char;
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '~') {
            encoded.push(character);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

fn metadata_matches(metadata: &HitomiGalleryMetadata, terms: &[ResidualTerm]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let mut fields = vec![metadata.title.clone()];
    fields.extend(metadata.alternate_title.clone());
    fields.extend(metadata.artists.clone());
    fields.extend(metadata.groups.clone());
    fields.extend(metadata.series.clone());
    fields.extend(metadata.characters.clone());
    fields.extend(metadata.tags.iter().map(|tag| tag.name.clone()));
    fields.extend(metadata.language.clone());
    let haystack = normalize_text(&fields.join(" "));
    terms.iter().all(|term| {
        let found = haystack.contains(&term.value);
        if term.negative {
            !found
        } else {
            found
        }
    })
}

fn gallery_summary(
    metadata: &HitomiGalleryMetadata,
    sort: SearchSort,
    rank: usize,
) -> Result<GallerySummary, SourceContractError> {
    let id_value = i64::try_from(metadata.id).map_err(|_| {
        SourceContractError::invalid_data("gallery ID", "does not fit the application domain")
    })?;
    let id = GalleryId::new(id_value)
        .map_err(|error| SourceContractError::invalid_data("gallery ID", error.to_string()))?;
    let cover = metadata.pages.first();
    let popularity = if matches!(
        sort,
        SearchSort::PopularToday
            | SearchSort::PopularWeek
            | SearchSort::PopularMonth
            | SearchSort::PopularYear
    ) {
        u32::MAX.saturating_sub(rank as u32)
    } else {
        0
    };
    Ok(GallerySummary {
        id,
        title: metadata.title.clone(),
        artist: metadata
            .artists
            .first()
            .cloned()
            .unwrap_or_else(|| "Unknown artist".to_owned()),
        group: metadata.groups.first().cloned(),
        series: metadata.series.clone(),
        characters: metadata.characters.clone(),
        pages: metadata.pages.len() as u32,
        language: domain_language(metadata.language.as_deref()),
        tags: metadata
            .tags
            .iter()
            .map(|tag| {
                let name = normalize_text(&tag.name).replace(' ', "_");
                match tag.kind {
                    HitomiTagKind::Female => format!("female:{name}"),
                    HitomiTagKind::Male => format!("male:{name}"),
                    HitomiTagKind::General => name,
                }
            })
            .collect(),
        published_rank: published_rank(metadata.published_at.as_deref()),
        popularity,
        thumbnail_key: cover.map(|page| page.source_revision.to_string()),
        thumbnail_width: cover.and_then(|page| page.width).unwrap_or(512),
        thumbnail_height: cover.and_then(|page| page.height).unwrap_or(512),
    })
}

fn domain_language(value: Option<&str>) -> Language {
    match value
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "japanese" => Language::Japanese,
        "chinese" => Language::Chinese,
        "english" => Language::English,
        _ => Language::Korean,
    }
}

fn published_rank(value: Option<&str>) -> u32 {
    let digits: String = value
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_digit)
        .take(8)
        .collect();
    digits.parse().unwrap_or_default()
}

fn stable_random_rank(id: u64) -> u64 {
    let mut value = id ^ 0x9e37_79b9_7f4a_7c15;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
