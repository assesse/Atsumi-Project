mod http;
mod search;

use std::{
    collections::{HashMap, VecDeque},
    io::Cursor,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{Arc, Mutex, MutexGuard, Weak},
    time::{Duration, Instant},
};

use image::{
    codecs::webp::WebPEncoder, ExtendedColorType, GenericImageView, ImageEncoder, ImageFormat,
    ImageReader, Limits,
};

use crate::{
    application::{
        DownloadGallerySnapshot, DownloadPagePayload, DownloadSourceImageFormat,
        DownloadSourcePage, DownloadSourcePort, RepositoryError, TagCatalogSource,
    },
    domain::{Gallery, GalleryId, GalleryMetadata, SourcePageNumber, TagCatalogEntry},
    source::{
        hitomi::{
            all_catalog_pages, download_full_candidates, galleryinfo_script_url, gg_script_url,
            merge_catalog, parse_catalog_page, parse_galleryinfo_script, parse_gg_routing,
            parse_nozomi_ids, webp_full_candidates, webp_thumbnail_candidates, GgRoutingTable,
            HitomiGalleryMetadata, HitomiImageCandidate, HitomiImageFormat, HitomiImageKind,
            ThumbnailSize, HITOMI_METADATA_ORIGIN,
        },
        SourceCandidateDiagnostic, SourceContractError, SourceErrorCode,
    },
    thumbnail::{
        CancellationToken, ResolvedThumbnail, ThumbnailFailureCode, ThumbnailKey,
        ThumbnailPriority, ThumbnailResolveError, ThumbnailResolver,
    },
};

use self::http::{
    stable_thumbnail_error, ExpectedContent, HttpPayload, HttpPriority, HttpRequest,
    HttpSchedulerConfig, HttpTransport, ReqwestTransport,
};
use self::search::QueryCache;

const SCRIPT_RESPONSE_LIMIT: usize = 2 * 1024 * 1024;
const NOZOMI_RESPONSE_LIMIT: usize = 32 * 1024 * 1024;
const THUMBNAIL_RESPONSE_LIMIT: usize = 12 * 1024 * 1024;
const FULL_IMAGE_RESPONSE_LIMIT: usize = 64 * 1024 * 1024;
const CATALOG_RESPONSE_LIMIT: usize = 4 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_IMAGE_DECODE_ALLOC: u64 = 256 * 1024 * 1024;
const SOURCE_PAGE_FALLBACK_MAX_EDGE: u32 = 768;

#[derive(Debug, Clone)]
pub struct HitomiLiveConfig {
    pub max_concurrent_requests: usize,
    pub max_concurrent_per_host: usize,
    pub request_start_interval: Duration,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub max_retries: u8,
    pub retry_base_delay: Duration,
    pub retry_max_delay: Duration,
    pub metadata_cache_capacity: usize,
    pub metadata_cache_ttl: Duration,
    pub gg_cache_ttl: Duration,
    pub query_cache_capacity: usize,
    pub max_candidate_ids: usize,
    pub related_gallery_limit: usize,
}

impl Default for HitomiLiveConfig {
    fn default() -> Self {
        Self {
            max_concurrent_requests: 5,
            max_concurrent_per_host: 5,
            request_start_interval: Duration::from_millis(25),
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(12),
            max_retries: 2,
            retry_base_delay: Duration::from_millis(250),
            retry_max_delay: Duration::from_secs(5),
            metadata_cache_capacity: 2_000,
            metadata_cache_ttl: Duration::from_secs(15 * 60),
            gg_cache_ttl: Duration::from_secs(60 * 60),
            query_cache_capacity: 32,
            max_candidate_ids: 1_000,
            related_gallery_limit: 8,
        }
    }
}

pub struct HitomiLiveAdapter {
    transport: Arc<dyn HttpTransport>,
    config: HitomiLiveConfig,
    metadata_cache: Mutex<TimedCache<u64, Arc<HitomiGalleryMetadata>>>,
    metadata_inflight: Mutex<HashMap<u64, Weak<Mutex<()>>>>,
    gg_cache: Mutex<Option<TimedValue<Arc<GgRoutingTable>>>>,
    queries: Mutex<QueryCache>,
    source_page_fallback_decode: Mutex<()>,
}

impl HitomiLiveAdapter {
    pub fn new(config: HitomiLiveConfig) -> Result<Self, RepositoryError> {
        validate_config(&config).map_err(RepositoryError::Source)?;
        let transport = ReqwestTransport::new(HttpSchedulerConfig {
            max_concurrent_requests: config.max_concurrent_requests,
            max_concurrent_per_host: config.max_concurrent_per_host,
            request_start_interval: config.request_start_interval,
            connect_timeout: config.connect_timeout,
            request_timeout: config.request_timeout,
            max_retries: config.max_retries,
            retry_base_delay: config.retry_base_delay,
            retry_max_delay: config.retry_max_delay,
        })
        .map_err(RepositoryError::Source)?;
        Ok(Self::with_transport(config, Arc::new(transport)))
    }

    fn with_transport(config: HitomiLiveConfig, transport: Arc<dyn HttpTransport>) -> Self {
        let metadata_capacity = config.metadata_cache_capacity;
        let query_capacity = config.query_cache_capacity;
        Self {
            transport,
            config,
            metadata_cache: Mutex::new(TimedCache::new(metadata_capacity)),
            metadata_inflight: Mutex::new(HashMap::new()),
            gg_cache: Mutex::new(None),
            queries: Mutex::new(QueryCache::new(query_capacity)),
            source_page_fallback_decode: Mutex::new(()),
        }
    }

    /// Drops only in-memory derived source state. The HTTP scheduler and its
    /// host cooldowns deliberately survive so a repair cannot bypass Retry-After.
    pub fn clear_derived_caches(&self) {
        unpoison(self.metadata_cache.lock()).values.clear();
        unpoison(self.metadata_inflight.lock()).clear();
        *unpoison(self.gg_cache.lock()) = None;
        unpoison(self.queries.lock()).clear();
    }

    fn fetch_metadata(
        &self,
        gallery_id: u64,
    ) -> Result<Arc<HitomiGalleryMetadata>, SourceContractError> {
        self.fetch_metadata_with_cancellation(gallery_id, None)
    }

    fn fetch_metadata_with_cancellation(
        &self,
        gallery_id: u64,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Arc<HitomiGalleryMetadata>, SourceContractError> {
        if let Some(cancellation) = cancellation {
            check_cancelled(cancellation)?;
        }
        if let Some(metadata) = unpoison(self.metadata_cache.lock())
            .get_fresh(&gallery_id, self.config.metadata_cache_ttl)
        {
            return Ok(metadata);
        }

        let request_lock = {
            let mut locks = unpoison(self.metadata_inflight.lock());
            if locks.len() > self.config.metadata_cache_capacity.saturating_mul(2) {
                locks.retain(|_, lock| lock.strong_count() > 0);
            }
            locks
                .get(&gallery_id)
                .and_then(Weak::upgrade)
                .unwrap_or_else(|| {
                    let lock = Arc::new(Mutex::new(()));
                    locks.insert(gallery_id, Arc::downgrade(&lock));
                    lock
                })
        };
        let _request_guard = unpoison(request_lock.lock());
        // A shared single-flight wait cannot be interrupted while blocked on
        // std::sync::Mutex, but cancellation is checked immediately after it.
        if let Some(cancellation) = cancellation {
            check_cancelled(cancellation)?;
        }
        if let Some(metadata) = unpoison(self.metadata_cache.lock())
            .get_fresh(&gallery_id, self.config.metadata_cache_ttl)
        {
            return Ok(metadata);
        }

        let url = galleryinfo_script_url(gallery_id)?;
        let payload = self.transport.execute(HttpRequest {
            url,
            expected: ExpectedContent::Script,
            max_bytes: SCRIPT_RESPONSE_LIMIT,
            range: None,
            priority: HttpPriority::Critical,
            cancellation: cancellation.cloned(),
        })?;
        if let Some(cancellation) = cancellation {
            check_cancelled(cancellation)?;
        }
        let script = std::str::from_utf8(&payload.bytes).map_err(|error| {
            SourceContractError::invalid_data("galleryinfo script", error.to_string())
        })?;
        let metadata = Arc::new(parse_galleryinfo_script(script)?);
        if metadata.id != gallery_id {
            return Err(SourceContractError::invalid_data(
                "galleryinfo.id",
                format!("requested {gallery_id}, received {}", metadata.id),
            ));
        }
        unpoison(self.metadata_cache.lock()).insert(gallery_id, Arc::clone(&metadata));
        Ok(metadata)
    }

    fn fetch_gg_routing(&self) -> Result<Arc<GgRoutingTable>, SourceContractError> {
        let mut cache = unpoison(self.gg_cache.lock());
        if let Some(cached) = cache.as_ref() {
            if cached.inserted.elapsed() <= self.config.gg_cache_ttl {
                return Ok(Arc::clone(&cached.value));
            }
        }

        let payload = self.transport.execute(HttpRequest {
            url: gg_script_url(),
            expected: ExpectedContent::Script,
            max_bytes: SCRIPT_RESPONSE_LIMIT,
            range: None,
            priority: HttpPriority::Critical,
            cancellation: None,
        })?;
        let script = std::str::from_utf8(&payload.bytes).map_err(|error| {
            SourceContractError::invalid_data("gg.js script", error.to_string())
        })?;
        let routing = Arc::new(parse_gg_routing(script)?);
        *cache = Some(TimedValue {
            inserted: Instant::now(),
            value: Arc::clone(&routing),
        });
        Ok(routing)
    }

    fn fetch_nozomi_path(&self, path: &str) -> Result<Vec<u64>, SourceContractError> {
        self.fetch_nozomi_path_with_cancellation(path, None)
    }

    fn fetch_nozomi_path_with_cancellation(
        &self,
        path: &str,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Vec<u64>, SourceContractError> {
        if let Some(cancellation) = cancellation {
            check_cancelled(cancellation)?;
        }
        if path.starts_with('/') || path.contains("..") || !path.ends_with(".nozomi") {
            return Err(SourceContractError::validation(
                "nozomiPath",
                "is not a safe relative Nozomi path",
            ));
        }
        let payload = self.transport.execute(HttpRequest {
            url: format!("{HITOMI_METADATA_ORIGIN}/{path}"),
            expected: ExpectedContent::Nozomi,
            max_bytes: NOZOMI_RESPONSE_LIMIT,
            range: None,
            priority: HttpPriority::Critical,
            cancellation: cancellation.cloned(),
        })?;
        if let Some(cancellation) = cancellation {
            check_cancelled(cancellation)?;
        }
        parse_nozomi_ids(&payload.bytes)
    }

    fn fetch_optional_nozomi_path(&self, path: &str) -> Result<Vec<u64>, SourceContractError> {
        match self.fetch_nozomi_path(path) {
            Err(error) if error.code == SourceErrorCode::NotFound => Ok(Vec::new()),
            result => result,
        }
    }

    fn fetch_optional_nozomi_path_with_cancellation(
        &self,
        path: &str,
        cancellation: &CancellationToken,
    ) -> Result<Vec<u64>, SourceContractError> {
        match self.fetch_nozomi_path_with_cancellation(path, Some(cancellation)) {
            Err(error) if error.code == SourceErrorCode::NotFound => Ok(Vec::new()),
            result => result,
        }
    }

    fn fetch_image_candidate(
        &self,
        candidate: &HitomiImageCandidate,
        priority: ThumbnailPriority,
        cancellation: &CancellationToken,
        normalize_source_page_fallback: bool,
    ) -> Result<ResolvedThumbnail, SourceContractError> {
        let payload = self.transport.execute(HttpRequest {
            url: candidate.url.clone(),
            expected: ExpectedContent::Image,
            max_bytes: THUMBNAIL_RESPONSE_LIMIT,
            range: None,
            priority: priority.into(),
            cancellation: Some(cancellation.clone()),
        })?;
        if normalize_source_page_fallback {
            check_cancelled(cancellation)?;
            // Full-size mirror fallbacks are decoded one at a time and reduced
            // before entering the thumbnail cache/IPC. This keeps a missing
            // derivative from recreating the WebView memory spike that the
            // bounded Detail preview window is intended to prevent.
            let _decode = unpoison(self.source_page_fallback_decode.lock());
            check_cancelled(cancellation)?;
            decode_thumbnail(
                payload,
                candidate.source_revision.clone().into_string(),
                Some(SOURCE_PAGE_FALLBACK_MAX_EDGE),
            )
        } else {
            decode_thumbnail(
                payload,
                candidate.source_revision.clone().into_string(),
                None,
            )
        }
    }

    fn resolve_thumbnail(
        &self,
        key: &ThumbnailKey,
        cancellation: &CancellationToken,
        priority: ThumbnailPriority,
    ) -> Result<ResolvedThumbnail, SourceContractError> {
        check_cancelled(cancellation)?;
        let gallery_id = key.gallery_id().ok_or_else(|| {
            SourceContractError::validation("thumbnailKey", "is not a live gallery key")
        })?;
        let gallery_id = u64::try_from(gallery_id).map_err(|_| {
            SourceContractError::validation("galleryId", "must be a positive integer")
        })?;
        let metadata = self.fetch_metadata(gallery_id)?;
        check_cancelled(cancellation)?;
        let source_page = key.source_page().unwrap_or(1);
        let page = metadata.page(source_page)?;
        let routing = self.fetch_gg_routing()?;
        check_cancelled(cancellation)?;
        let mut candidates = webp_thumbnail_candidates(page, &routing, ThumbnailSize::Large)?;
        // Floating Detail source-page previews need the alternate w1/w2 WebP
        // routes when Hitomi's derivative and primary routes have not caught
        // up yet. Keep ordinary gallery covers on the established low-cost
        // path so GalleryCard traffic and payload sizes do not change.
        if matches!(key, ThumbnailKey::GalleryPage { .. }) {
            for candidate in webp_full_candidates(page, &routing)? {
                if let Some(existing) = candidates
                    .iter_mut()
                    .find(|existing| existing.url == candidate.url)
                {
                    // The primary full-path URL also appears in the legacy
                    // thumbnail list. Preserve its position but mark its real
                    // payload class so it is normalized before IPC.
                    existing.kind = HitomiImageKind::Full;
                } else {
                    candidates.push(candidate);
                }
            }
        }
        // AVIF remains the final display-format fallback after every WebP
        // route. Original JPEG/PNG candidates are intentionally not used by
        // the thumbnail IPC path.
        if page.has_avif {
            candidates.extend(
                download_full_candidates(page, &routing)?
                    .into_iter()
                    .filter(|candidate| candidate.format == HitomiImageFormat::Avif),
            );
        }
        if candidates.is_empty() {
            return Err(SourceContractError::image_candidates_exhausted());
        }

        let mut last_error = None;
        for candidate in candidates {
            check_cancelled(cancellation)?;
            let normalize_source_page_fallback = matches!(key, ThumbnailKey::GalleryPage { .. })
                && candidate.kind == HitomiImageKind::Full
                && candidate.format == HitomiImageFormat::Webp;
            match self.fetch_image_candidate(
                &candidate,
                priority,
                cancellation,
                normalize_source_page_fallback,
            ) {
                Ok(thumbnail) => {
                    check_cancelled(cancellation)?;
                    return Ok(thumbnail);
                }
                Err(error)
                    if matches!(
                        error.code,
                        SourceErrorCode::NotFound
                            | SourceErrorCode::ImageResponseInvalid
                            | SourceErrorCode::ImageDecodeFailed
                    ) =>
                {
                    last_error = Some(error);
                }
                Err(error) => return Err(error),
            }
        }

        Err(last_error.unwrap_or_else(SourceContractError::image_candidates_exhausted))
    }
}

impl ThumbnailResolver for HitomiLiveAdapter {
    fn resolve(
        &self,
        key: &ThumbnailKey,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedThumbnail, ThumbnailResolveError> {
        self.resolve_thumbnail(key, cancellation, ThumbnailPriority::Visible)
            .map_err(source_thumbnail_error)
    }

    fn resolve_with_priority(
        &self,
        key: &ThumbnailKey,
        cancellation: &CancellationToken,
        priority: ThumbnailPriority,
    ) -> Result<ResolvedThumbnail, ThumbnailResolveError> {
        self.resolve_thumbnail(key, cancellation, priority)
            .map_err(source_thumbnail_error)
    }
}

impl TagCatalogSource for HitomiLiveAdapter {
    fn tag_catalog_fetch_all(&self) -> Result<Vec<TagCatalogEntry>, RepositoryError> {
        let mut entries = Vec::new();
        for (namespace, url) in all_catalog_pages() {
            let payload = self.transport.execute(HttpRequest {
                url,
                expected: ExpectedContent::Html,
                max_bytes: CATALOG_RESPONSE_LIMIT,
                range: None,
                priority: HttpPriority::Visible,
                cancellation: None,
            })?;
            let html = String::from_utf8(payload.bytes).map_err(|_| {
                SourceContractError::invalid_data("metadata catalog", "page is not UTF-8")
            })?;
            entries.extend(parse_catalog_page(&html, namespace)?);
        }
        Ok(merge_catalog(entries)?)
    }
}

impl DownloadSourcePort for HitomiLiveAdapter {
    fn gallery_snapshot(
        &self,
        gallery_id: GalleryId,
        cancellation: &CancellationToken,
    ) -> Result<DownloadGallerySnapshot, SourceContractError> {
        check_cancelled(cancellation)?;
        let source_id = u64::try_from(gallery_id.get()).map_err(|_| {
            SourceContractError::validation("galleryId", "must be a positive integer")
        })?;
        let metadata = self.fetch_metadata(source_id)?;
        check_cancelled(cancellation)?;
        let source_page_count = u32::try_from(metadata.pages.len()).map_err(|_| {
            SourceContractError::invalid_data(
                "gallery page count",
                "exceeds the supported 32-bit range",
            )
        })?;
        let gallery_metadata = GalleryMetadata::new(
            metadata.title.clone(),
            metadata.artists.first().cloned(),
            metadata.groups.first().cloned(),
            source_page_count,
        )
        .map(|gallery| gallery.with_artists(metadata.artists.clone()))
        .map_err(|error| {
            SourceContractError::invalid_data("gallery metadata", error.to_string())
        })?;
        let gallery_revision = metadata
            .source_revision
            .as_str()
            .rsplit_once(':')
            .and_then(|(_, fingerprint)| u64::from_str_radix(fingerprint, 16).ok())
            .unwrap_or(1);
        let pages = metadata
            .pages
            .iter()
            .map(|page| {
                Ok(DownloadSourcePage {
                    source_page_number: SourcePageNumber::new(page.source_page).map_err(
                        |error| {
                            SourceContractError::invalid_data(
                                "source page number",
                                error.to_string(),
                            )
                        },
                    )?,
                    source_revision: page.source_revision.to_string(),
                })
            })
            .collect::<Result<Vec<_>, SourceContractError>>()?;
        Ok(DownloadGallerySnapshot {
            gallery: Gallery::new(gallery_id, gallery_revision, gallery_metadata),
            source_revision: metadata.source_revision.to_string(),
            pages,
        })
    }

    fn download_page(
        &self,
        gallery_id: GalleryId,
        source_page_number: SourcePageNumber,
        cancellation: &CancellationToken,
    ) -> Result<DownloadPagePayload, SourceContractError> {
        check_cancelled(cancellation)?;
        let source_id = u64::try_from(gallery_id.get()).map_err(|_| {
            SourceContractError::validation("galleryId", "must be a positive integer")
        })?;
        let metadata = self.fetch_metadata(source_id)?;
        let page = metadata.page(source_page_number.get())?;
        let routing = self.fetch_gg_routing()?;
        let candidates = download_full_candidates(page, &routing)?;
        if candidates.is_empty() {
            return Err(SourceContractError::image_candidates_exhausted());
        }

        let mut diagnostics = Vec::new();
        for (candidate_index, candidate) in candidates.iter().enumerate() {
            check_cancelled(cancellation)?;
            let candidate_index = u32::try_from(candidate_index).unwrap_or(u32::MAX);
            if candidate.format == HitomiImageFormat::Jxl {
                diagnostics.push(SourceCandidateDiagnostic {
                    candidate_index,
                    format: candidate.format.as_str().to_owned(),
                    http_status: None,
                    content_type: None,
                    bytes_received: None,
                    error_code: Some(SourceErrorCode::ImageFormatUnsupported),
                    retryable: false,
                });
                continue;
            }
            let payload = self.transport.execute(HttpRequest {
                url: candidate.url.clone(),
                expected: ExpectedContent::Image,
                max_bytes: FULL_IMAGE_RESPONSE_LIMIT,
                range: None,
                priority: HttpPriority::Download,
                cancellation: Some(cancellation.clone()),
            });
            let payload = match payload {
                Ok(payload) => payload,
                Err(mut error) => {
                    diagnostics.push(candidate_error_diagnostic(
                        candidate_index,
                        candidate.format,
                        &error,
                    ));
                    if candidate_fallback_allowed(&error) {
                        continue;
                    }
                    error.candidate_diagnostics = diagnostics;
                    return Err(error);
                }
            };
            let http_status = payload.status;
            let content_type = sanitized_content_type(&payload.content_type);
            let bytes_received = u64::try_from(payload.bytes.len()).ok();
            match decode_download_payload(
                payload,
                source_page_number,
                candidate.source_revision.to_string(),
                candidate_index,
                candidate.format,
            ) {
                Ok(mut page) => {
                    diagnostics.push(SourceCandidateDiagnostic {
                        candidate_index,
                        format: candidate.format.as_str().to_owned(),
                        http_status: Some(http_status),
                        content_type,
                        bytes_received,
                        error_code: None,
                        retryable: false,
                    });
                    page.candidate_diagnostics = diagnostics;
                    return Ok(page);
                }
                Err(mut error) => {
                    diagnostics.push(SourceCandidateDiagnostic {
                        candidate_index,
                        format: candidate.format.as_str().to_owned(),
                        http_status: Some(http_status),
                        content_type,
                        bytes_received,
                        error_code: Some(error.code),
                        retryable: error.retryable,
                    });
                    if candidate_fallback_allowed(&error) {
                        continue;
                    }
                    error.candidate_diagnostics = diagnostics;
                    return Err(error);
                }
            }
        }
        let retryable = diagnostics.iter().any(|diagnostic| diagnostic.retryable);
        Err(SourceContractError::image_candidates_exhausted_with(
            retryable,
            diagnostics,
        ))
    }
}

fn candidate_fallback_allowed(error: &SourceContractError) -> bool {
    error.retryable
        || matches!(
            error.code,
            SourceErrorCode::NotFound
                | SourceErrorCode::ImageResponseInvalid
                | SourceErrorCode::ImageDecodeFailed
                | SourceErrorCode::ImageFormatUnsupported
        )
}

fn candidate_error_diagnostic(
    candidate_index: u32,
    format: HitomiImageFormat,
    error: &SourceContractError,
) -> SourceCandidateDiagnostic {
    SourceCandidateDiagnostic {
        candidate_index,
        format: format.as_str().to_owned(),
        http_status: error.http_status,
        content_type: error.diagnostic_content_type.clone(),
        bytes_received: error.diagnostic_bytes_received,
        error_code: Some(error.code),
        retryable: error.retryable,
    }
}

fn sanitized_content_type(content_type: &str) -> Option<String> {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if mime.is_empty() {
        return None;
    }
    if mime.len() <= 127
        && mime
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'+' | b'-'))
    {
        Some(mime)
    } else {
        Some("invalid".into())
    }
}

fn source_thumbnail_error(error: SourceContractError) -> ThumbnailResolveError {
    if error.code == SourceErrorCode::Cancelled {
        return ThumbnailResolveError::cancelled();
    }
    let code = match error.code {
        SourceErrorCode::Cancelled => ThumbnailFailureCode::Cancelled,
        SourceErrorCode::NotFound => ThumbnailFailureCode::NotFound,
        SourceErrorCode::ImageCandidatesExhausted => ThumbnailFailureCode::CandidatesExhausted,
        SourceErrorCode::ImageResponseInvalid => ThumbnailFailureCode::ResponseInvalid,
        SourceErrorCode::ImageDecodeFailed => ThumbnailFailureCode::DecodeFailed,
        SourceErrorCode::ImageFormatUnsupported => ThumbnailFailureCode::InvalidData,
        SourceErrorCode::Unauthorized => ThumbnailFailureCode::Unauthorized,
        SourceErrorCode::RateLimited
        | SourceErrorCode::TemporarilyUnavailable
        | SourceErrorCode::Timeout
        | SourceErrorCode::Transport => ThumbnailFailureCode::TemporarilyUnavailable,
        SourceErrorCode::Validation | SourceErrorCode::Protocol | SourceErrorCode::InvalidData => {
            ThumbnailFailureCode::InvalidData
        }
    };
    let (message, retryable) = stable_thumbnail_error(&error);
    ThumbnailResolveError::new(code, message, retryable)
}

fn check_cancelled(cancellation: &CancellationToken) -> Result<(), SourceContractError> {
    if cancellation.is_cancelled() {
        Err(SourceContractError::cancelled())
    } else {
        Ok(())
    }
}

fn decode_thumbnail(
    payload: HttpPayload,
    source_revision: String,
    normalize_max_edge: Option<u32>,
) -> Result<ResolvedThumbnail, SourceContractError> {
    let format = image::guess_format(&payload.bytes).map_err(|_| {
        SourceContractError::image_response_invalid(
            "thumbnail bytes do not contain a supported image signature",
        )
    })?;
    if !matches!(
        format,
        ImageFormat::WebP | ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::Avif
    ) {
        return Err(SourceContractError::image_response_invalid(format!(
            "thumbnail image format {format:?} is unsupported"
        )));
    }
    let content_type = canonical_image_content_type(format);
    let declared_type = payload
        .content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if !mime_matches_format(declared_type, format) {
        return Err(SourceContractError::image_response_invalid(format!(
            "thumbnail Content-Type {declared_type:?} does not match decoded {format:?} data"
        )));
    }

    let bytes = payload.bytes;
    let decode_result = catch_unwind(AssertUnwindSafe(|| {
        if format == ImageFormat::Avif {
            return super::avif_decode::decode_avif_rgba(&bytes).map_err(|_| {
                image::ImageError::Unsupported(
                    image::error::UnsupportedError::from_format_and_kind(
                        image::error::ImageFormatHint::Exact(ImageFormat::Avif),
                        image::error::UnsupportedErrorKind::GenericFeature(
                            "bounded AVIF decode".into(),
                        ),
                    ),
                )
            });
        }
        let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
        limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
        limits.max_alloc = Some(MAX_IMAGE_DECODE_ALLOC);
        reader.limits(limits);
        reader.decode()
    }));
    let image = match decode_result {
        Ok(Ok(image)) => image,
        Ok(Err(error)) => {
            return Err(SourceContractError::image_decode_failed(format!(
                "image decoder rejected the payload: {error}"
            )))
        }
        Err(_) => {
            return Err(SourceContractError::image_decode_failed(
                "image decoder rejected malformed input",
            ))
        }
    };
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(SourceContractError::image_decode_failed(
            "decoded thumbnail dimensions must be positive",
        ));
    }

    if let Some(max_edge) = normalize_max_edge {
        let image = image.thumbnail(max_edge, max_edge);
        let (width, height) = image.dimensions();
        let rgba = image.to_rgba8();
        let mut bytes = Vec::new();
        WebPEncoder::new_lossless(&mut bytes)
            .write_image(&rgba, width, height, ExtendedColorType::Rgba8)
            .map_err(|_| {
                SourceContractError::image_decode_failed(
                    "full source-page fallback could not be encoded as a bounded preview",
                )
            })?;
        return Ok(ResolvedThumbnail {
            content_type: "image/webp".to_owned(),
            bytes,
            width,
            height,
            source_revision: Some(source_revision),
        });
    }

    Ok(ResolvedThumbnail {
        content_type: content_type.to_owned(),
        bytes,
        width,
        height,
        source_revision: Some(source_revision),
    })
}

fn decode_download_payload(
    payload: HttpPayload,
    source_page_number: SourcePageNumber,
    source_revision: String,
    candidate_index: u32,
    expected_format: HitomiImageFormat,
) -> Result<DownloadPagePayload, SourceContractError> {
    let format = image::guess_format(&payload.bytes).map_err(|_| {
        SourceContractError::image_response_invalid(
            "download bytes do not contain a supported image signature",
        )
    })?;
    if !candidate_magic_matches(expected_format, format) {
        return Err(SourceContractError::image_response_invalid(
            "download image signature does not match the selected candidate format",
        ));
    }
    let source_format = match format {
        ImageFormat::WebP => DownloadSourceImageFormat::Webp,
        ImageFormat::Jpeg => DownloadSourceImageFormat::Jpeg,
        ImageFormat::Png => DownloadSourceImageFormat::Png,
        ImageFormat::Avif => DownloadSourceImageFormat::Avif,
        _ => {
            return Err(SourceContractError::image_format_unsupported(
                "download image format",
            ))
        }
    };
    let declared_type = payload
        .content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim();
    if !mime_matches_format(declared_type, format) {
        return Err(SourceContractError::image_response_invalid(
            "download Content-Type does not match the decoded image signature",
        ));
    }

    let bytes = payload.bytes;
    let decode_result = catch_unwind(AssertUnwindSafe(|| {
        if format == ImageFormat::Avif {
            return super::avif_decode::decode_avif_rgba(&bytes).map_err(|_| {
                image::ImageError::Unsupported(
                    image::error::UnsupportedError::from_format_and_kind(
                        image::error::ImageFormatHint::Exact(ImageFormat::Avif),
                        image::error::UnsupportedErrorKind::GenericFeature(
                            "bounded AVIF decode".into(),
                        ),
                    ),
                )
            });
        }
        let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
        limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
        limits.max_alloc = Some(MAX_IMAGE_DECODE_ALLOC);
        reader.limits(limits);
        reader.decode()
    }));
    let image = match decode_result {
        Ok(Ok(image)) => image,
        Ok(Err(_)) => {
            return Err(SourceContractError::image_decode_failed(
                "image decoder rejected the download payload",
            ))
        }
        Err(_) => {
            return Err(SourceContractError::image_decode_failed(
                "image decoder rejected malformed download input",
            ))
        }
    };
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(SourceContractError::image_decode_failed(
            "decoded download dimensions must be positive",
        ));
    }
    Ok(DownloadPagePayload {
        source_page_number,
        bytes,
        source_revision,
        source_format,
        width,
        height,
        candidate_index,
        candidate_diagnostics: Vec::new(),
    })
}

fn candidate_magic_matches(expected: HitomiImageFormat, actual: ImageFormat) -> bool {
    matches!(
        (expected, actual),
        (HitomiImageFormat::Webp, ImageFormat::WebP)
            | (HitomiImageFormat::Jpeg, ImageFormat::Jpeg)
            | (HitomiImageFormat::Png, ImageFormat::Png)
            | (HitomiImageFormat::Avif, ImageFormat::Avif)
    )
}

fn canonical_image_content_type(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::WebP => "image/webp",
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::Png => "image/png",
        ImageFormat::Avif => "image/avif",
        _ => "application/octet-stream",
    }
}

fn mime_matches_format(mime: &str, format: ImageFormat) -> bool {
    if mime.is_empty() || mime.eq_ignore_ascii_case("application/octet-stream") {
        return true;
    }
    match format {
        ImageFormat::WebP => mime.eq_ignore_ascii_case("image/webp"),
        ImageFormat::Jpeg => {
            mime.eq_ignore_ascii_case("image/jpeg") || mime.eq_ignore_ascii_case("image/jpg")
        }
        ImageFormat::Png => mime.eq_ignore_ascii_case("image/png"),
        ImageFormat::Avif => mime.eq_ignore_ascii_case("image/avif"),
        _ => false,
    }
}

fn validate_config(config: &HitomiLiveConfig) -> Result<(), SourceContractError> {
    if !(1..=30).contains(&config.max_concurrent_requests) {
        return Err(SourceContractError::validation(
            "maxConcurrentRequests",
            "must be between 1 and 30",
        ));
    }
    if !(1..=config.max_concurrent_requests).contains(&config.max_concurrent_per_host) {
        return Err(SourceContractError::validation(
            "maxConcurrentPerHost",
            "must be positive and no greater than maxConcurrentRequests",
        ));
    }
    if config.connect_timeout.is_zero() || config.request_timeout.is_zero() {
        return Err(SourceContractError::validation(
            "requestTimeout",
            "timeouts must be greater than zero",
        ));
    }
    if config.connect_timeout > config.request_timeout {
        return Err(SourceContractError::validation(
            "connectTimeout",
            "must not exceed the whole-request timeout",
        ));
    }
    if config.max_retries > 5
        || config.retry_base_delay.is_zero()
        || config.retry_max_delay < config.retry_base_delay
        || config.retry_max_delay > Duration::from_secs(60)
    {
        return Err(SourceContractError::validation(
            "retryPolicy",
            "must use at most 5 retries and a positive bounded delay",
        ));
    }
    if config.metadata_cache_capacity == 0
        || config.query_cache_capacity == 0
        || config.max_candidate_ids == 0
    {
        return Err(SourceContractError::validation(
            "cacheCapacity",
            "cache and candidate capacities must be greater than zero",
        ));
    }
    Ok(())
}

struct TimedValue<T> {
    inserted: Instant,
    value: T,
}

struct TimedCache<K, V> {
    capacity: usize,
    values: HashMap<K, TimedValue<V>>,
    order: VecDeque<K>,
}

impl<K, V> TimedCache<K, V>
where
    K: Clone + Eq + std::hash::Hash,
    V: Clone,
{
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            values: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get_fresh(&mut self, key: &K, ttl: Duration) -> Option<V> {
        let fresh = self
            .values
            .get(key)
            .is_some_and(|value| value.inserted.elapsed() <= ttl);
        if !fresh {
            self.values.remove(key);
            return None;
        }
        let value = self.values.get(key)?.value.clone();
        self.order.retain(|candidate| candidate != key);
        self.order.push_back(key.clone());
        Some(value)
    }

    fn insert(&mut self, key: K, value: V) {
        self.values.insert(
            key.clone(),
            TimedValue {
                inserted: Instant::now(),
                value,
            },
        );
        self.order.retain(|candidate| candidate != &key);
        self.order.push_back(key);
        while self.values.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.values.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

fn unpoison<T>(result: std::sync::LockResult<MutexGuard<'_, T>>) -> MutexGuard<'_, T> {
    result.unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests;
