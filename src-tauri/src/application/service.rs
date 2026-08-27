use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

use crate::domain::{
    download_root_for_display, plan_artifact_relative_directory, AutoFindExclusionResult,
    AutoFindSnapshot, DownloadEntry, DownloadEntryId, DownloadJobDescriptor, DownloadJobProjection,
    DownloadListRequest, DownloadPage, ExplorationDataResetRequest, ExplorationDataResetResult,
    ExplorationExclusion, ExplorationExclusionRestoreResult, FavoriteKey, FavoriteMutationResult,
    FavoriteRecord, FixtureDownloadJobStep, Gallery, GalleryDetail, GalleryId, GalleryMetadata,
    GalleryPage, JobRef, SearchHistoryEntry, SearchRequest, SearchSubmission, SettingsPatch,
    SettingsSnapshot, TagCatalogStatus, TagSuggestion, TagSuggestionRequest, ValidationError,
    WindowPlacement, WindowPlacementSnapshot,
};

use super::{
    ApplicationError, AutomationRepository, DownloadMutationOutcome, DownloadQueueAddOutcome,
    DownloadRepository, SearchRepository, StateRepository, TagCatalogRepository, TagCatalogSource,
};

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadQueueLaunch {
    pub entries: Vec<DownloadEntry>,
    pub jobs: Vec<DownloadJobDescriptor>,
}

pub struct ApplicationService {
    repository: Arc<dyn StateRepository>,
    search_repository: Option<Arc<dyn SearchRepository>>,
    download_repository: Option<Arc<dyn DownloadRepository>>,
    automation_repository: Option<Arc<dyn AutomationRepository>>,
    tag_catalog_repository: Option<Arc<dyn TagCatalogRepository>>,
    tag_catalog_source: Option<Arc<dyn TagCatalogSource>>,
    tag_catalog_refresh_lock: Arc<Mutex<()>>,
}

impl Clone for ApplicationService {
    fn clone(&self) -> Self {
        Self {
            repository: Arc::clone(&self.repository),
            search_repository: self.search_repository.as_ref().map(Arc::clone),
            download_repository: self.download_repository.as_ref().map(Arc::clone),
            automation_repository: self.automation_repository.as_ref().map(Arc::clone),
            tag_catalog_repository: self.tag_catalog_repository.as_ref().map(Arc::clone),
            tag_catalog_source: self.tag_catalog_source.as_ref().map(Arc::clone),
            tag_catalog_refresh_lock: Arc::clone(&self.tag_catalog_refresh_lock),
        }
    }
}

impl ApplicationService {
    pub fn new(repository: Arc<dyn StateRepository>) -> Self {
        Self {
            repository,
            search_repository: None,
            download_repository: None,
            automation_repository: None,
            tag_catalog_repository: None,
            tag_catalog_source: None,
            tag_catalog_refresh_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn with_search_repository(mut self, search_repository: Arc<dyn SearchRepository>) -> Self {
        self.search_repository = Some(search_repository);
        self
    }

    pub fn with_download_repository(
        mut self,
        download_repository: Arc<dyn DownloadRepository>,
    ) -> Self {
        self.download_repository = Some(download_repository);
        self
    }

    pub fn with_automation_repository(
        mut self,
        automation_repository: Arc<dyn AutomationRepository>,
    ) -> Self {
        self.automation_repository = Some(automation_repository);
        self
    }

    pub fn with_tag_catalog(
        mut self,
        repository: Arc<dyn TagCatalogRepository>,
        source: Arc<dyn TagCatalogSource>,
    ) -> Self {
        self.tag_catalog_repository = Some(repository);
        self.tag_catalog_source = Some(source);
        self
    }

    pub fn settings_get(&self) -> Result<SettingsSnapshot, ApplicationError> {
        let mut snapshot = self.repository.settings_get()?;
        snapshot.download_root = download_root_for_display(&snapshot.download_root);
        Ok(snapshot)
    }

    pub fn settings_update(
        &self,
        patch: SettingsPatch,
        expected_revision: u64,
    ) -> Result<SettingsSnapshot, ApplicationError> {
        let current = self.repository.settings_get()?;
        ensure_revision("settings", expected_revision, current.revision)?;
        let next = current.apply_patch(patch)?;

        if self
            .repository
            .settings_compare_and_set(&next, expected_revision)?
        {
            let mut displayed = next;
            displayed.download_root = download_root_for_display(&displayed.download_root);
            return Ok(displayed);
        }

        let actual = self.repository.settings_get()?.revision;
        Err(revision_conflict("settings", expected_revision, actual))
    }

    pub fn folder_name_template_preview(&self, template: &str) -> Result<String, ApplicationError> {
        let gallery = Gallery::new(
            GalleryId::new(4_113_714)?,
            0,
            GalleryMetadata::new("작품 제목", Some("작가".into()), Some("그룹".into()), 1)?,
        );
        Ok(plan_artifact_relative_directory(template, &gallery)?
            .as_str()
            .to_owned())
    }

    pub fn window_placement_get(&self) -> Result<WindowPlacementSnapshot, ApplicationError> {
        self.repository.window_placement_get().map_err(Into::into)
    }

    pub fn window_placement_update(
        &self,
        placement: WindowPlacement,
        expected_revision: u64,
    ) -> Result<WindowPlacementSnapshot, ApplicationError> {
        let current = self.repository.window_placement_get()?;
        ensure_revision("windowPlacement", expected_revision, current.revision)?;
        let next = current.updated(placement)?;

        if self
            .repository
            .window_placement_compare_and_set(&next, expected_revision)?
        {
            return Ok(next);
        }

        let actual = self.repository.window_placement_get()?.revision;
        Err(revision_conflict(
            "windowPlacement",
            expected_revision,
            actual,
        ))
    }

    pub fn fixture_download_job_advance(
        &self,
        job_id: &str,
        worker_attempt: u64,
        step: FixtureDownloadJobStep,
    ) -> Result<DownloadJobProjection, ApplicationError> {
        self.download_repository()?
            .fixture_download_job_advance(job_id, worker_attempt, step)
            .map_err(Into::into)
    }

    pub fn download_recover_interrupted(&self) -> Result<usize, ApplicationError> {
        self.download_repository()?
            .download_recover_interrupted()
            .map_err(Into::into)
    }

    pub fn download_queue_add(
        &self,
        galleries: Vec<i64>,
        request_id: String,
    ) -> Result<DownloadQueueLaunch, ApplicationError> {
        validate_request_id(&request_id)?;
        if galleries.is_empty() {
            return Err(ValidationError::new("galleries", "must not be empty").into());
        }
        if galleries.len() > 200 {
            return Err(ValidationError::new("galleries", "must contain at most 200 IDs").into());
        }

        if galleries.iter().any(|gallery_id| *gallery_id <= 0) {
            return Err(
                ValidationError::new("galleries", "gallery IDs must be positive integers").into(),
            );
        }
        let mut galleries = galleries
            .into_iter()
            .map(GalleryId::new)
            .collect::<Result<Vec<_>, _>>()?;
        galleries.sort_unstable();
        galleries.dedup();

        match self
            .download_repository()?
            .download_queue_add(request_id.trim(), &galleries)?
        {
            DownloadQueueAddOutcome::Added(record) => Ok(DownloadQueueLaunch {
                entries: record.entries,
                jobs: record.jobs,
            }),
            DownloadQueueAddOutcome::IdempotencyConflict => {
                Err(ApplicationError::IdempotencyConflict {
                    request_id: request_id.trim().to_owned(),
                })
            }
        }
    }

    pub fn download_entries_list(
        &self,
        request: DownloadListRequest,
    ) -> Result<DownloadPage, ApplicationError> {
        let request = request.normalized()?;
        self.download_repository()?
            .download_entries_list(&request)
            .map_err(Into::into)
    }

    pub fn download_active_count(&self) -> Result<u64, ApplicationError> {
        self.download_repository()?
            .download_active_count()
            .map_err(Into::into)
    }

    pub fn download_active_entry_ids(&self) -> Result<Vec<DownloadEntryId>, ApplicationError> {
        self.download_repository()?
            .download_active_entry_ids()
            .map_err(Into::into)
    }

    pub fn download_retry(&self, entry_ids: Vec<String>) -> Result<Vec<JobRef>, ApplicationError> {
        let entry_ids = normalize_entry_ids(entry_ids)?;
        match self.download_repository()?.download_retry(&entry_ids)? {
            DownloadMutationOutcome::Applied(job_refs) => Ok(job_refs),
            DownloadMutationOutcome::EntryNotFound(entry_id) => {
                Err(ApplicationError::DownloadEntryNotFound(entry_id))
            }
            DownloadMutationOutcome::InvalidState { entry_id, state } => {
                Err(ApplicationError::InvalidDownloadState {
                    entry_id,
                    state,
                    operation: "retry",
                })
            }
        }
    }

    pub fn download_cancel(
        &self,
        entry_ids: Vec<String>,
    ) -> Result<Vec<DownloadEntry>, ApplicationError> {
        let entry_ids = normalize_entry_ids(entry_ids)?;
        match self.download_repository()?.download_cancel(&entry_ids)? {
            DownloadMutationOutcome::Applied(entries) => Ok(entries),
            DownloadMutationOutcome::EntryNotFound(entry_id) => {
                Err(ApplicationError::DownloadEntryNotFound(entry_id))
            }
            DownloadMutationOutcome::InvalidState { entry_id, state } => {
                Err(ApplicationError::InvalidDownloadState {
                    entry_id,
                    state,
                    operation: "cancel",
                })
            }
        }
    }

    pub fn search_submit(
        &self,
        request: SearchRequest,
    ) -> Result<SearchSubmission, ApplicationError> {
        let history_request = request.normalized()?;
        let settings = self.repository.settings_get()?;
        let request = history_request
            .clone()
            .with_global_tags(&settings.search_include_tags, &settings.search_exclude_tags)?;
        tracing::info!(
            operation_id = "search_submit",
            has_text = !request.text.is_empty(),
            include_tag_count = request.include_tags.len(),
            exclude_tag_count = request.exclude_tags.len(),
            language_count = request.languages.len(),
            sort = ?request.sort,
            page_size = request.page_size,
            "submitting source search query"
        );
        let submission = self
            .search_repository()?
            .search_submit(&request)
            .map_err(ApplicationError::from)?;
        if !history_request.text.is_empty()
            || !history_request.include_tags.is_empty()
            || !history_request.exclude_tags.is_empty()
        {
            if let Some(repository) = self.automation_repository.as_deref() {
                repository.search_history_record(&history_request)?;
            }
        }
        Ok(submission)
    }

    pub fn search_page_get(
        &self,
        query_id: String,
        page: u32,
    ) -> Result<GalleryPage, ApplicationError> {
        let query_id = query_id.trim();
        if query_id.is_empty() {
            return Err(ValidationError::new("queryId", "must not be empty").into());
        }
        if query_id.len() > 200 {
            return Err(ValidationError::new("queryId", "must be at most 200 bytes").into());
        }
        if page == 0 {
            return Err(ValidationError::new("page", "must be one-based").into());
        }

        let result = self
            .search_repository()?
            .search_page_get(query_id, page)?
            .ok_or_else(|| ApplicationError::QueryNotFound(query_id.to_owned()))?;

        let is_out_of_range = if result.total_pages == 0 {
            page != 1
        } else {
            page > result.total_pages
        };
        if is_out_of_range {
            return Err(
                ValidationError::new("page", "must not exceed the search result range").into(),
            );
        }

        Ok(result)
    }

    pub fn search_page_get_cancellable(
        &self,
        query_id: String,
        page: u32,
        cancellation: &crate::thumbnail::CancellationToken,
    ) -> Result<GalleryPage, ApplicationError> {
        let query_id = query_id.trim();
        if query_id.is_empty() {
            return Err(ValidationError::new("queryId", "must not be empty").into());
        }
        if query_id.len() > 200 {
            return Err(ValidationError::new("queryId", "must be at most 200 bytes").into());
        }
        if page == 0 {
            return Err(ValidationError::new("page", "must be one-based").into());
        }

        let result = self
            .search_repository()?
            .search_page_get_cancellable(query_id, page, cancellation)?
            .ok_or_else(|| ApplicationError::QueryNotFound(query_id.to_owned()))?;
        let is_out_of_range = if result.total_pages == 0 {
            page != 1
        } else {
            page > result.total_pages
        };
        if is_out_of_range {
            return Err(
                ValidationError::new("page", "must not exceed the search result range").into(),
            );
        }
        Ok(result)
    }

    pub fn gallery_detail_get(&self, gallery_id: i64) -> Result<GalleryDetail, ApplicationError> {
        let gallery_id = GalleryId::new(gallery_id)?;
        self.search_repository()?
            .gallery_detail_get(gallery_id)?
            .ok_or(ApplicationError::GalleryNotFound(gallery_id))
    }

    pub fn favorites_list(&self) -> Result<Vec<FavoriteRecord>, ApplicationError> {
        self.automation_repository()?
            .favorites_list()
            .map_err(Into::into)
    }

    pub fn favorite_set(
        &self,
        key: FavoriteKey,
        enabled: bool,
    ) -> Result<FavoriteMutationResult, ApplicationError> {
        let key = key.normalized()?;
        self.automation_repository()?
            .favorite_set(&key, enabled)
            .map_err(Into::into)
    }

    pub fn search_history_list(
        &self,
        limit: u32,
    ) -> Result<Vec<SearchHistoryEntry>, ApplicationError> {
        if !(1..=100).contains(&limit) {
            return Err(ValidationError::new("limit", "must be between 1 and 100").into());
        }
        self.automation_repository()?
            .search_history_list(limit)
            .map_err(Into::into)
    }

    pub fn tag_catalog_status(&self) -> Result<TagCatalogStatus, ApplicationError> {
        Ok(self.tag_catalog_repository()?.tag_catalog_status()?)
    }

    pub fn tag_suggestions_search(
        &self,
        request: TagSuggestionRequest,
    ) -> Result<Vec<TagSuggestion>, ApplicationError> {
        let request = request.normalized()?;
        if request.query.chars().count() < 2 || request.limit == 0 {
            return Ok(Vec::new());
        }
        Ok(self
            .tag_catalog_repository()?
            .tag_suggestions_search(&request)?)
    }

    pub fn tag_catalog_refresh(&self) -> Result<TagCatalogStatus, ApplicationError> {
        let _guard = self
            .tag_catalog_refresh_lock
            .try_lock()
            .map_err(|_| super::RepositoryError::OperationActive("tag catalog refresh".into()))?;
        let repository = self.tag_catalog_repository()?;
        repository.tag_catalog_record_attempt()?;
        match self.tag_catalog_source()?.tag_catalog_fetch_all() {
            Ok(entries) => repository.tag_catalog_replace(&entries).map_err(Into::into),
            Err(error) => {
                let code = error.stable_code();
                repository.tag_catalog_record_failure(code, "The existing tag catalog was kept")?;
                Err(error.into())
            }
        }
    }

    pub fn auto_find_snapshot(&self) -> Result<AutoFindSnapshot, ApplicationError> {
        self.automation_repository()?
            .auto_find_snapshot()
            .map_err(Into::into)
    }

    pub fn auto_find_exclude(
        &self,
        gallery_ids: Vec<i64>,
        reason: String,
    ) -> Result<AutoFindExclusionResult, ApplicationError> {
        if gallery_ids.is_empty() {
            return Err(ValidationError::new("galleryIds", "must not be empty").into());
        }
        if gallery_ids.len() > 200 {
            return Err(ValidationError::new("galleryIds", "must contain at most 200 IDs").into());
        }
        let reason = reason.trim();
        if reason.is_empty() || reason.len() > 500 {
            return Err(
                ValidationError::new("reason", "must contain between 1 and 500 bytes").into(),
            );
        }
        let mut gallery_ids = gallery_ids
            .into_iter()
            .map(GalleryId::new)
            .collect::<Result<Vec<_>, _>>()?;
        gallery_ids.sort_unstable();
        gallery_ids.dedup();
        self.automation_repository()?
            .auto_find_exclude(&gallery_ids, reason)
            .map_err(Into::into)
    }

    pub fn exploration_exclusions_list(
        &self,
    ) -> Result<Vec<ExplorationExclusion>, ApplicationError> {
        self.automation_repository()?
            .exploration_exclusions_list()
            .map_err(Into::into)
    }

    pub fn exploration_exclusions_restore(
        &self,
        gallery_ids: Vec<i64>,
    ) -> Result<ExplorationExclusionRestoreResult, ApplicationError> {
        if gallery_ids.is_empty() {
            return Err(ValidationError::new("galleryIds", "must not be empty").into());
        }
        if gallery_ids.len() > 200 {
            return Err(ValidationError::new("galleryIds", "must contain at most 200 IDs").into());
        }
        let mut gallery_ids = gallery_ids
            .into_iter()
            .map(GalleryId::new)
            .collect::<Result<Vec<_>, _>>()?;
        gallery_ids.sort_unstable();
        gallery_ids.dedup();
        self.automation_repository()?
            .exploration_exclusions_restore(&gallery_ids)
            .map_err(Into::into)
    }

    pub fn exploration_data_reset(
        &self,
        request: ExplorationDataResetRequest,
    ) -> Result<ExplorationDataResetResult, ApplicationError> {
        request.validate()?;
        self.automation_repository()?
            .exploration_data_reset()
            .map_err(Into::into)
    }

    fn search_repository(&self) -> Result<&dyn SearchRepository, ApplicationError> {
        self.search_repository.as_deref().ok_or_else(|| {
            super::RepositoryError::Other("search repository is not configured".into()).into()
        })
    }

    fn download_repository(&self) -> Result<&dyn DownloadRepository, ApplicationError> {
        self.download_repository.as_deref().ok_or_else(|| {
            super::RepositoryError::Other("download repository is not configured".into()).into()
        })
    }

    fn automation_repository(&self) -> Result<&dyn AutomationRepository, ApplicationError> {
        self.automation_repository.as_deref().ok_or_else(|| {
            super::RepositoryError::Other("automation repository is not configured".into()).into()
        })
    }

    fn tag_catalog_repository(&self) -> Result<&dyn TagCatalogRepository, ApplicationError> {
        self.tag_catalog_repository.as_deref().ok_or_else(|| {
            super::RepositoryError::Other("tag catalog repository is not configured".into()).into()
        })
    }

    fn tag_catalog_source(&self) -> Result<&dyn TagCatalogSource, ApplicationError> {
        self.tag_catalog_source.as_deref().ok_or_else(|| {
            super::RepositoryError::Other("tag catalog source is not configured".into()).into()
        })
    }
}

fn ensure_revision(
    resource: &'static str,
    expected: u64,
    actual: u64,
) -> Result<(), ApplicationError> {
    if expected == actual {
        Ok(())
    } else {
        Err(revision_conflict(resource, expected, actual))
    }
}

fn revision_conflict(resource: &'static str, expected: u64, actual: u64) -> ApplicationError {
    ApplicationError::RevisionConflict {
        resource,
        expected,
        actual,
    }
}

fn validate_request_id(request_id: &str) -> Result<(), ValidationError> {
    let request_id = request_id.trim();
    if request_id.is_empty() {
        return Err(ValidationError::new("requestId", "must not be empty"));
    }
    if request_id.len() > 200 {
        return Err(ValidationError::new(
            "requestId",
            "must be at most 200 bytes",
        ));
    }
    Ok(())
}

fn normalize_entry_ids(values: Vec<String>) -> Result<Vec<DownloadEntryId>, ValidationError> {
    if values.is_empty() {
        return Err(ValidationError::new("entryIds", "must not be empty"));
    }
    if values.len() > 200 {
        return Err(ValidationError::new(
            "entryIds",
            "must contain at most 200 IDs",
        ));
    }

    let mut seen = BTreeSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let entry_id = DownloadEntryId::new(value)?;
        if seen.insert(entry_id.clone()) {
            normalized.push(entry_id);
        }
    }
    Ok(normalized)
}
