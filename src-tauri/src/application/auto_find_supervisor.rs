use std::{
    collections::BTreeMap,
    sync::{mpsc::Sender, Arc, Mutex},
    thread::{self, JoinHandle},
};

use crate::{
    domain::{
        AutoFindCandidateRecord, AutoFindCutoffEvidence, AutoFindHistoryMode, AutoFindRun,
        AutoFindRunState, FavoriteNamespace, FavoriteRecord, Language,
    },
    thumbnail::CancellationToken,
};

use super::{
    ApplicationError, AutoFindSource, AutoFindSourceRequest, AutomationRepository, RepositoryError,
    StateRepository,
};

const AUTO_FIND_CANDIDATE_LIMIT: u32 = 50_000;

#[derive(Clone)]
pub struct AutoFindSupervisor {
    inner: Arc<AutoFindSupervisorInner>,
}

struct AutoFindSupervisorInner {
    repository: Arc<dyn AutomationRepository>,
    settings: Arc<dyn StateRepository>,
    source: Arc<dyn AutoFindSource>,
    events: Sender<AutoFindRun>,
    control: Mutex<()>,
    active: Mutex<Option<ActiveRun>>,
}

struct ActiveRun {
    run_id: String,
    cancellation: CancellationToken,
    worker: Option<JoinHandle<()>>,
}

pub(crate) struct PreparedAutoFindRefresh {
    favorites: Vec<FavoriteRecord>,
    total_favorites: u32,
    history_mode: AutoFindHistoryMode,
    cutoff_evidence: Vec<AutoFindCutoffEvidence>,
}

impl AutoFindSupervisor {
    pub fn new(
        repository: Arc<dyn AutomationRepository>,
        settings: Arc<dyn StateRepository>,
        source: Arc<dyn AutoFindSource>,
        events: Sender<AutoFindRun>,
    ) -> Self {
        Self {
            inner: Arc::new(AutoFindSupervisorInner {
                repository,
                settings,
                source,
                events,
                control: Mutex::new(()),
                active: Mutex::new(None),
            }),
        }
    }

    pub fn recover_interrupted(&self) -> Result<usize, ApplicationError> {
        self.inner
            .repository
            .auto_find_recover_interrupted()
            .map_err(Into::into)
    }

    pub fn refresh(&self) -> Result<AutoFindRun, ApplicationError> {
        if let Some(active_run) = self.active_run_snapshot()? {
            return Ok(active_run);
        }
        let prepared = self.prepare_refresh()?;
        self.commit_refresh(prepared)
    }

    pub(crate) fn prepare_refresh(&self) -> Result<PreparedAutoFindRefresh, ApplicationError> {
        let favorites = self
            .inner
            .repository
            .favorites_list()?
            .into_iter()
            .filter(|favorite| favorite.namespace == FavoriteNamespace::Artist)
            .collect::<Vec<_>>();
        let total_favorites = u32::try_from(favorites.len()).unwrap_or(u32::MAX);
        let history_mode = self.inner.settings.settings_get()?.auto_find_history_mode;
        let artists = favorites
            .iter()
            .map(|favorite| favorite.value.clone())
            .collect::<Vec<_>>();
        let cutoff_evidence = self.inner.repository.auto_find_owned_cutoffs(&artists)?;
        Ok(PreparedAutoFindRefresh {
            favorites,
            total_favorites,
            history_mode,
            cutoff_evidence,
        })
    }

    pub(crate) fn commit_refresh(
        &self,
        prepared: PreparedAutoFindRefresh,
    ) -> Result<AutoFindRun, ApplicationError> {
        let _control = self.control_lock()?;
        self.reap_finished_worker();
        if let Some(active_run) = self.active_run()? {
            return Ok(active_run);
        }
        let PreparedAutoFindRefresh {
            favorites,
            total_favorites,
            history_mode,
            cutoff_evidence,
        } = prepared;
        let run = self.inner.repository.auto_find_start(
            total_favorites,
            history_mode,
            &cutoff_evidence,
        )?;
        if run.state != AutoFindRunState::Running {
            return Ok(run);
        }
        if total_favorites == 0 {
            let completed = self
                .inner
                .repository
                .auto_find_finish(&run.run_id, AutoFindRunState::Completed, None, None)?
                .ok_or_else(|| {
                    RepositoryError::Corrupt(
                        "empty Auto Find run disappeared before completion".into(),
                    )
                })?;
            let _ = self.inner.events.send(completed.clone());
            return Ok(completed);
        }

        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let inner = Arc::clone(&self.inner);
        let run_id = run.run_id.clone();
        let worker_run_id = run_id.clone();
        let worker = thread::Builder::new()
            .name(format!("atsumi-auto-find-{}", short_id(&run_id)))
            .spawn(move || {
                run_refresh(
                    inner,
                    worker_run_id,
                    favorites,
                    history_mode,
                    cutoff_evidence,
                    worker_cancellation,
                );
            })
            .map_err(|error| {
                let _ = self.inner.repository.auto_find_finish(
                    &run_id,
                    AutoFindRunState::Failed,
                    Some("AUTO_FIND_WORKER_UNAVAILABLE"),
                    Some("The Auto Find worker could not be started"),
                );
                RepositoryError::Other(format!("could not start Auto Find worker: {error}"))
            })?;
        *self.active_lock()? = Some(ActiveRun {
            run_id,
            cancellation,
            worker: Some(worker),
        });
        let _ = self.inner.events.send(run.clone());
        Ok(run)
    }

    pub fn cancel(&self) -> Result<AutoFindRun, ApplicationError> {
        let _control = self.control_lock()?;
        self.reap_finished_worker();
        let run_id = {
            let guard = self.active_lock()?;
            let active = guard.as_ref().ok_or(ApplicationError::AutoFindNotRunning)?;
            active.cancellation.cancel();
            active.run_id.clone()
        };
        let run = self
            .inner
            .repository
            .auto_find_finish(
                &run_id,
                AutoFindRunState::Cancelled,
                Some("AUTO_FIND_CANCELLED"),
                Some("The Auto Find refresh was cancelled"),
            )?
            .ok_or(ApplicationError::AutoFindNotRunning)?;
        let _ = self.inner.events.send(run.clone());
        Ok(run)
    }

    /// Returns only a currently running identity/projection. Finished workers
    /// are reaped first so stale in-memory slots never keep the app in an
    /// artificial "active work" state.
    pub fn active_run_snapshot(&self) -> Result<Option<AutoFindRun>, ApplicationError> {
        let _control = self.control_lock()?;
        self.reap_finished_worker();
        Ok(self
            .active_run()?
            .filter(|run| run.state == AutoFindRunState::Running))
    }

    pub fn shutdown_and_wait(&self) {
        let _control = self.control_lock().ok();
        let active = self.active_lock().ok().and_then(|mut active| active.take());
        if let Some(mut active) = active {
            active.cancellation.cancel();
            let _ = self.inner.repository.auto_find_finish(
                &active.run_id,
                AutoFindRunState::Cancelled,
                Some("AUTO_FIND_APP_EXIT"),
                Some("The application closed during Auto Find refresh"),
            );
            if let Some(worker) = active.worker.take() {
                let _ = worker.join();
            }
        }
    }

    fn control_lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, ApplicationError> {
        self.inner.control.lock().map_err(|_| {
            RepositoryError::Other("Auto Find supervisor control mutex was poisoned".into()).into()
        })
    }

    fn active_run(&self) -> Result<Option<AutoFindRun>, ApplicationError> {
        let run_id = self
            .active_lock()?
            .as_ref()
            .map(|active| active.run_id.clone());
        let Some(run_id) = run_id else {
            return Ok(None);
        };
        let snapshot = self.inner.repository.auto_find_snapshot()?;
        Ok(snapshot.run.filter(|run| run.run_id == run_id))
    }

    fn reap_finished_worker(&self) {
        let worker = self.active_lock().ok().and_then(|mut active| {
            let finished = active
                .as_ref()
                .and_then(|run| run.worker.as_ref())
                .is_some_and(JoinHandle::is_finished);
            finished.then(|| active.take()).flatten()
        });
        if let Some(mut worker) = worker {
            if let Some(handle) = worker.worker.take() {
                let _ = handle.join();
            }
        }
    }

    fn active_lock(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<ActiveRun>>, ApplicationError> {
        self.inner.active.lock().map_err(|_| {
            RepositoryError::Other("Auto Find supervisor mutex was poisoned".into()).into()
        })
    }
}

fn run_refresh(
    inner: Arc<AutoFindSupervisorInner>,
    run_id: String,
    favorites: Vec<crate::domain::FavoriteRecord>,
    history_mode: AutoFindHistoryMode,
    cutoff_evidence: Vec<AutoFindCutoffEvidence>,
    cancellation: CancellationToken,
) {
    let result = (|| -> Result<(), RepositoryError> {
        let cutoffs = cutoff_evidence
            .into_iter()
            .map(|evidence| (evidence.artist.clone(), evidence))
            .collect::<BTreeMap<_, _>>();
        for (favorite_index, favorite) in favorites.iter().enumerate() {
            if cancelled(&inner, &run_id, &cancellation)? {
                return Ok(());
            }
            let cutoff = cutoffs.get(&favorite.value);
            let request = AutoFindSourceRequest {
                artist: favorite.value.clone(),
                languages: vec![
                    Language::Korean,
                    Language::Japanese,
                    Language::Chinese,
                    Language::English,
                ],
                newer_than_gallery_id: (history_mode
                    == AutoFindHistoryMode::NewerThanOldestDownloaded)
                    .then(|| cutoff.and_then(|evidence| evidence.oldest_owned_gallery_id))
                    .flatten(),
                candidate_limit: AUTO_FIND_CANDIDATE_LIMIT,
            };
            let source = inner
                .source
                .auto_find_artist_plan(&request, &cancellation)?;
            if let Some(reason) = source.truncated_reason {
                inner.repository.auto_find_truncation_add(
                    &run_id,
                    &crate::domain::AutoFindTruncation {
                        artist: favorite.value.clone(),
                        reason,
                        eligible_count: source.eligible_count,
                        limit: source.limit,
                    },
                )?;
            }
            for gallery_id in source.candidate_ids {
                if cancelled(&inner, &run_id, &cancellation)? {
                    return Ok(());
                }
                let Some(gallery) = inner
                    .source
                    .auto_find_gallery_summary(gallery_id, &cancellation)?
                else {
                    continue;
                };
                if cancelled(&inner, &run_id, &cancellation)? {
                    return Ok(());
                }
                record_candidate(&inner, &run_id, favorite, gallery)?;
            }
            if let Some(run) = inner.repository.auto_find_progress(
                &run_id,
                u32::try_from(favorite_index + 1).unwrap_or(u32::MAX),
            )? {
                let _ = inner.events.send(run);
            }
        }
        if let Some(run) =
            inner
                .repository
                .auto_find_finish(&run_id, AutoFindRunState::Completed, None, None)?
        {
            let _ = inner.events.send(run);
        }
        Ok(())
    })();

    if cancellation.is_cancelled() {
        if let Ok(true) = inner.repository.auto_find_is_running(&run_id) {
            if let Ok(Some(run)) = inner.repository.auto_find_finish(
                &run_id,
                AutoFindRunState::Cancelled,
                Some("AUTO_FIND_CANCELLED"),
                Some("The Auto Find refresh was cancelled"),
            ) {
                let _ = inner.events.send(run);
            }
        }
    } else if result.is_err() {
        if let Ok(Some(run)) = inner.repository.auto_find_finish(
            &run_id,
            AutoFindRunState::Failed,
            Some("AUTO_FIND_SOURCE_FAILED"),
            Some("The source refresh failed; retry the explicit Auto Find refresh"),
        ) {
            let _ = inner.events.send(run);
        }
    }
}

fn record_candidate(
    inner: &AutoFindSupervisorInner,
    run_id: &str,
    favorite: &crate::domain::FavoriteRecord,
    gallery: crate::domain::GallerySummary,
) -> Result<(), RepositoryError> {
    let _ = inner
        .repository
        .auto_find_candidate_add(&AutoFindCandidateRecord {
            run_id: run_id.to_owned(),
            gallery,
            matched_favorite: favorite.key(),
        })?;
    Ok(())
}

fn cancelled(
    inner: &AutoFindSupervisorInner,
    run_id: &str,
    cancellation: &CancellationToken,
) -> Result<bool, RepositoryError> {
    Ok(cancellation.is_cancelled() || !inner.repository.auto_find_is_running(run_id)?)
}

fn short_id(value: &str) -> &str {
    value.rsplit('-').next().unwrap_or("run")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc, Arc, Barrier, Condvar, Mutex,
        },
        thread,
        time::{Duration, Instant},
    };

    use crate::{
        application::{
            ApplicationService, AutoFindSource, AutoFindSourceRequest, AutoFindSourceResult,
            AutomationRepository, DownloadRepository, SearchRepository,
        },
        domain::{
            AutoFindCandidateRecord, AutoFindHistoryMode, AutoFindRunState, FavoriteKey,
            FavoriteNamespace, GalleryDetail, GalleryId, GalleryPage, GallerySummary, Language,
            SearchRequest, SearchSort, SearchSubmission, SettingsPatch,
        },
        infrastructure::{FixtureSearchRepository, SqliteRepository},
    };

    use super::AutoFindSupervisor;

    struct DeterministicSearchRepository {
        items: Vec<GallerySummary>,
        submissions: AtomicUsize,
        requests: Mutex<Vec<SearchRequest>>,
        auto_find_requests: Mutex<Vec<AutoFindSourceRequest>>,
        gate: Option<Arc<SearchGate>>,
        cancel_after_summary: Option<usize>,
        summary_calls: AtomicUsize,
    }

    impl DeterministicSearchRepository {
        fn immediate(items: Vec<GallerySummary>) -> Self {
            Self {
                items,
                submissions: AtomicUsize::new(0),
                requests: Mutex::new(Vec::new()),
                auto_find_requests: Mutex::new(Vec::new()),
                gate: None,
                cancel_after_summary: None,
                summary_calls: AtomicUsize::new(0),
            }
        }

        fn blocked(items: Vec<GallerySummary>, gate: Arc<SearchGate>) -> Self {
            Self {
                items,
                submissions: AtomicUsize::new(0),
                requests: Mutex::new(Vec::new()),
                auto_find_requests: Mutex::new(Vec::new()),
                gate: Some(gate),
                cancel_after_summary: None,
                summary_calls: AtomicUsize::new(0),
            }
        }

        fn submission_count(&self) -> usize {
            self.submissions.load(Ordering::SeqCst)
        }

        fn cancels_after_summaries(items: Vec<GallerySummary>, summaries: usize) -> Self {
            Self {
                items,
                submissions: AtomicUsize::new(0),
                requests: Mutex::new(Vec::new()),
                auto_find_requests: Mutex::new(Vec::new()),
                gate: None,
                cancel_after_summary: Some(summaries),
                summary_calls: AtomicUsize::new(0),
            }
        }
    }

    impl SearchRepository for DeterministicSearchRepository {
        fn search_submit(
            &self,
            request: &SearchRequest,
        ) -> Result<SearchSubmission, super::RepositoryError> {
            self.submissions.fetch_add(1, Ordering::SeqCst);
            self.requests
                .lock()
                .expect("test request mutex")
                .push(request.clone());
            if let Some(gate) = &self.gate {
                gate.block();
            }
            Ok(SearchSubmission {
                query_id: "auto-find-test-query".into(),
                first_page: GalleryPage {
                    page: 1,
                    total_pages: 1,
                    items: self.items.clone(),
                },
            })
        }

        fn search_page_get(
            &self,
            _query_id: &str,
            _page: u32,
        ) -> Result<Option<GalleryPage>, super::RepositoryError> {
            Ok(None)
        }

        fn gallery_detail_get(
            &self,
            _gallery_id: GalleryId,
        ) -> Result<Option<GalleryDetail>, super::RepositoryError> {
            Ok(None)
        }
    }

    impl AutoFindSource for DeterministicSearchRepository {
        fn auto_find_artist_plan(
            &self,
            request: &AutoFindSourceRequest,
            _cancellation: &crate::thumbnail::CancellationToken,
        ) -> Result<AutoFindSourceResult, super::RepositoryError> {
            self.submissions.fetch_add(1, Ordering::SeqCst);
            self.auto_find_requests
                .lock()
                .expect("test Auto Find request mutex")
                .push(request.clone());
            if let Some(gate) = &self.gate {
                gate.block();
            }
            Ok(AutoFindSourceResult {
                candidate_ids: self.items.iter().map(|item| item.id).collect(),
                eligible_count: u32::try_from(self.items.len()).unwrap_or(u32::MAX),
                limit: 50_000,
                truncated_reason: None,
            })
        }

        fn auto_find_gallery_summary(
            &self,
            gallery_id: GalleryId,
            cancellation: &crate::thumbnail::CancellationToken,
        ) -> Result<Option<GallerySummary>, super::RepositoryError> {
            let call = self.summary_calls.fetch_add(1, Ordering::SeqCst) + 1;
            let result = self
                .items
                .iter()
                .find(|item| item.id == gallery_id)
                .cloned();
            if self.cancel_after_summary == Some(call) {
                cancellation.cancel();
            }
            Ok(result)
        }
    }

    #[derive(Default)]
    struct SearchGate {
        state: Mutex<SearchGateState>,
        changed: Condvar,
    }

    #[derive(Default)]
    struct SearchGateState {
        entered: bool,
        released: bool,
    }

    impl SearchGate {
        fn block(&self) {
            let mut state = self.state.lock().expect("test search gate mutex");
            state.entered = true;
            self.changed.notify_all();
            while !state.released {
                state = self.changed.wait(state).expect("wait for search release");
            }
        }

        fn wait_until_entered(&self) {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut state = self.state.lock().expect("test search gate mutex");
            while !state.entered {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .expect("Auto Find search did not reach the deterministic gate");
                let (next, timeout) = self
                    .changed
                    .wait_timeout(state, remaining)
                    .expect("wait for Auto Find search");
                state = next;
                assert!(!timeout.timed_out(), "Auto Find search did not start");
            }
        }

        fn release(&self) {
            let mut state = self.state.lock().expect("test search gate mutex");
            state.released = true;
            self.changed.notify_all();
        }
    }

    #[test]
    fn favorite_namespaces_are_normalized_and_persisted() {
        let temporary = tempfile::tempdir().expect("create favorite database directory");
        let database_path = temporary.path().join("favorites.sqlite3");
        let expected = [
            (FavoriteNamespace::Artist, "serein"),
            (FavoriteNamespace::Group, "paper studio"),
            (FavoriteNamespace::Series, "archive of rain"),
            (FavoriteNamespace::Character, "mira lane"),
            (FavoriteNamespace::Tag, "female:glasses"),
        ];
        assert_eq!(
            FavoriteKey {
                namespace: FavoriteNamespace::Character,
                value: " Mira   Lane ".into(),
            }
            .normalized()
            .expect("normalize favorite search key")
            .search_token(),
            "character:mira_lane"
        );

        {
            let repository =
                Arc::new(SqliteRepository::open(&database_path).expect("open favorite repository"));
            let service =
                ApplicationService::new(repository.clone()).with_automation_repository(repository);
            let inputs = [
                (FavoriteNamespace::Artist, "  SEREIN "),
                (FavoriteNamespace::Group, " Paper   Studio "),
                (FavoriteNamespace::Series, " Archive Of Rain "),
                (FavoriteNamespace::Character, " Mira Lane "),
                (FavoriteNamespace::Tag, " FEMALE:GLASSES "),
            ];
            for (namespace, value) in inputs {
                let result = service
                    .favorite_set(
                        FavoriteKey {
                            namespace,
                            value: value.into(),
                        },
                        true,
                    )
                    .expect("enable normalized favorite");
                assert!(result.enabled);
            }
            let repeated = service
                .favorite_set(
                    FavoriteKey {
                        namespace: FavoriteNamespace::Artist,
                        value: " Serein ".into(),
                    },
                    true,
                )
                .expect("repeat equivalent artist favorite");
            assert_eq!(repeated.favorite.expect("enabled favorite").revision, 1);
        }

        {
            let repository = Arc::new(
                SqliteRepository::open(&database_path).expect("reopen favorite repository"),
            );
            let service =
                ApplicationService::new(repository.clone()).with_automation_repository(repository);
            let persisted = service
                .favorites_list()
                .expect("list persisted favorites")
                .into_iter()
                .map(|favorite| (favorite.namespace, favorite.value))
                .collect::<BTreeSet<_>>();
            assert_eq!(
                persisted,
                expected
                    .iter()
                    .map(|(namespace, value)| (*namespace, (*value).to_owned()))
                    .collect()
            );

            service
                .favorite_set(
                    FavoriteKey {
                        namespace: FavoriteNamespace::Artist,
                        value: " SEREIN ".into(),
                    },
                    false,
                )
                .expect("remove favorite through normalized key");
            assert!(service
                .favorites_list()
                .expect("list favorites after removal")
                .iter()
                .all(|favorite| favorite.namespace != FavoriteNamespace::Artist));
        }
    }

    #[test]
    fn only_submitted_nonempty_searches_are_recorded_and_persisted() {
        let temporary = tempfile::tempdir().expect("create history database directory");
        let database_path = temporary.path().join("history.sqlite3");

        {
            let repository =
                Arc::new(SqliteRepository::open(&database_path).expect("open history repository"));
            let search = Arc::new(
                FixtureSearchRepository::new().expect("create deterministic fixture search"),
            );
            let service = ApplicationService::new(repository.clone())
                .with_search_repository(search)
                .with_automation_repository(repository);
            service
                .search_submit(SearchRequest {
                    text: "   ".into(),
                    include_tags: Vec::new(),
                    exclude_tags: Vec::new(),
                    languages: Vec::new(),
                    sort: SearchSort::Recent,
                    page_size: 40,
                })
                .expect("load the empty Recent view");
            assert!(service
                .search_history_list(10)
                .expect("list empty search history")
                .is_empty());

            for request in [
                SearchRequest {
                    text: " ARTIST:SEREIN ".into(),
                    include_tags: vec![" FEMALE:GLASSES ".into()],
                    exclude_tags: Vec::new(),
                    languages: vec![Language::English, Language::Korean, Language::Korean],
                    sort: SearchSort::Recent,
                    page_size: 40,
                },
                SearchRequest {
                    text: "artist:serein".into(),
                    include_tags: vec!["female:glasses".into()],
                    exclude_tags: Vec::new(),
                    languages: vec![Language::Korean, Language::English],
                    sort: SearchSort::Recent,
                    page_size: 40,
                },
            ] {
                service
                    .search_submit(request)
                    .expect("submit a user search");
            }
        }

        let repository =
            Arc::new(SqliteRepository::open(&database_path).expect("reopen history repository"));
        let service =
            ApplicationService::new(repository.clone()).with_automation_repository(repository);
        let history = service
            .search_history_list(10)
            .expect("list persisted search history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].text, "artist:serein");
        assert_eq!(history[0].include_tags, vec!["female:glasses"]);
        assert_eq!(
            history[0].languages,
            vec![Language::Korean, Language::English]
        );
        assert_eq!(history[0].use_count, 2);
    }

    #[test]
    fn auto_find_filters_downloaded_and_excluded_results_and_survives_restart() {
        let temporary = tempfile::tempdir().expect("create Auto Find database directory");
        let database_path = temporary.path().join("auto-find.sqlite3");
        let downloaded = GalleryId::new(501).expect("downloaded gallery id");
        let excluded = GalleryId::new(502).expect("excluded gallery id");
        let candidate = GalleryId::new(503).expect("candidate gallery id");
        let completed_run_id = {
            let repository = Arc::new(
                SqliteRepository::open(&database_path).expect("open Auto Find repository"),
            );
            enable_artist(&repository, " SEREIN ");
            repository
                .download_queue_add("auto-find-downloaded-filter", &[downloaded])
                .expect("seed an existing download entry");
            repository
                .auto_find_exclude(&[excluded], "hidden by user")
                .expect("seed an explicit exclusion");
            let search = Arc::new(DeterministicSearchRepository::immediate(vec![
                gallery(downloaded.get(), "serein"),
                gallery(excluded.get(), "serein"),
                gallery(candidate.get(), "serein"),
            ]));
            let (events, event_receiver) = mpsc::channel();
            let supervisor = AutoFindSupervisor::new(
                repository.clone(),
                repository.clone(),
                search.clone(),
                events,
            );

            let started = supervisor.refresh().expect("start Auto Find refresh");
            assert_eq!(started.state, AutoFindRunState::Running);
            let snapshot = wait_for_terminal(&repository);
            assert!(supervisor
                .active_run_snapshot()
                .expect("completed Auto Find projection should be readable")
                .is_none());
            supervisor.shutdown_and_wait();
            assert_eq!(
                event_receiver.try_iter().count(),
                3,
                "one favorite emits only start, per-favorite progress, and final events"
            );

            let run = snapshot.run.expect("completed Auto Find run");
            assert_eq!(run.state, AutoFindRunState::Completed);
            assert_eq!(run.completed_favorites, 1);
            assert_eq!(run.candidates_found, 1);
            assert_eq!(search.submission_count(), 1);
            assert_eq!(
                search.auto_find_requests.lock().expect("test requests")[0].artist,
                "serein"
            );
            assert_eq!(
                snapshot
                    .candidates
                    .iter()
                    .map(|item| item.gallery.id)
                    .collect::<Vec<_>>(),
                vec![candidate]
            );
            assert_eq!(
                snapshot.candidates[0].matched_favorite,
                FavoriteKey {
                    namespace: FavoriteNamespace::Artist,
                    value: "serein".into(),
                }
            );
            run.run_id
        };

        let reopened =
            SqliteRepository::open(&database_path).expect("reopen completed Auto Find repository");
        let restored = reopened
            .auto_find_snapshot()
            .expect("restore completed Auto Find results");
        assert_eq!(restored.run.expect("restored run").run_id, completed_run_id);
        assert_eq!(restored.candidates.len(), 1);
        assert_eq!(restored.candidates[0].gallery.id, candidate);
        assert_eq!(restored.candidates[0].gallery.series, vec!["test series"]);
        assert_eq!(
            restored.candidates[0].gallery.characters,
            vec!["test character"]
        );
    }

    #[test]
    fn cancellation_prevents_a_late_search_response_from_adding_candidates() {
        let repository =
            Arc::new(SqliteRepository::open_in_memory().expect("open Auto Find repository"));
        enable_artist(&repository, "serein");
        let gate = Arc::new(SearchGate::default());
        let search = Arc::new(DeterministicSearchRepository::blocked(
            vec![gallery(601, "serein")],
            gate.clone(),
        ));
        let (events, _event_receiver) = mpsc::channel();
        let supervisor =
            AutoFindSupervisor::new(repository.clone(), repository.clone(), search, events);

        supervisor
            .refresh()
            .expect("start blocked Auto Find refresh");
        gate.wait_until_entered();
        let active = supervisor
            .active_run_snapshot()
            .expect("read running Auto Find projection")
            .expect("blocked run should be active");
        assert_eq!(active.state, AutoFindRunState::Running);
        let cancelled = supervisor.cancel().expect("cancel Auto Find refresh");
        assert_eq!(cancelled.state, AutoFindRunState::Cancelled);
        assert!(supervisor
            .active_run_snapshot()
            .expect("terminal Auto Find run should be readable")
            .is_none());
        gate.release();
        supervisor.shutdown_and_wait();

        let snapshot = repository
            .auto_find_snapshot()
            .expect("load cancelled Auto Find snapshot");
        let run = snapshot.run.expect("cancelled Auto Find run");
        assert_eq!(run.state, AutoFindRunState::Cancelled);
        assert_eq!(run.error_code.as_deref(), Some("AUTO_FIND_CANCELLED"));
        assert_eq!(run.candidates_found, 0);
        assert!(snapshot.candidates.is_empty());
    }

    #[test]
    fn cancellation_preserves_already_persisted_candidates_and_stops_later_metadata() {
        let repository =
            Arc::new(SqliteRepository::open_in_memory().expect("open Auto Find repository"));
        enable_artist(&repository, "serein");
        let source = Arc::new(DeterministicSearchRepository::cancels_after_summaries(
            vec![
                gallery(901, "serein"),
                gallery(902, "serein"),
                gallery(903, "serein"),
            ],
            2,
        ));
        let (events, _event_receiver) = mpsc::channel();
        let supervisor = AutoFindSupervisor::new(
            repository.clone(),
            repository.clone(),
            source.clone(),
            events,
        );
        supervisor
            .refresh()
            .expect("start cancellable Auto Find refresh");
        let snapshot = wait_for_terminal(&repository);
        supervisor.shutdown_and_wait();

        assert_eq!(source.summary_calls.load(Ordering::SeqCst), 2);
        assert_eq!(snapshot.candidates.len(), 1);
        assert_eq!(snapshot.candidates[0].gallery.id.get(), 901);
        assert_eq!(
            snapshot.run.expect("terminal run").state,
            AutoFindRunState::Cancelled
        );
    }

    #[test]
    fn history_mode_is_snapshotted_and_a_settings_change_applies_to_the_next_run() {
        let repository =
            Arc::new(SqliteRepository::open_in_memory().expect("open Auto Find repository"));
        enable_artist(&repository, "serein");
        let gate = Arc::new(SearchGate::default());
        let source = Arc::new(DeterministicSearchRepository::blocked(
            Vec::new(),
            gate.clone(),
        ));
        let (events, _event_receiver) = mpsc::channel();
        let supervisor =
            AutoFindSupervisor::new(repository.clone(), repository.clone(), source, events);
        supervisor.refresh().expect("start first run");
        gate.wait_until_entered();
        let service = ApplicationService::new(repository.clone());
        let settings = service.settings_get().expect("load settings");
        service
            .settings_update(
                SettingsPatch {
                    auto_find_history_mode: Some(AutoFindHistoryMode::NewerThanOldestDownloaded),
                    ..SettingsPatch::default()
                },
                settings.revision,
            )
            .expect("save setting while first run is active");
        assert_eq!(
            repository
                .auto_find_snapshot()
                .unwrap()
                .run
                .unwrap()
                .history_mode,
            AutoFindHistoryMode::IncludeAllHistory
        );
        gate.release();
        wait_for_terminal(&repository);

        supervisor.refresh().expect("start second run");
        let second = wait_for_terminal(&repository);
        supervisor.shutdown_and_wait();
        assert_eq!(
            second.run.expect("second run").history_mode,
            AutoFindHistoryMode::NewerThanOldestDownloaded
        );
    }

    #[test]
    fn concurrent_refresh_calls_share_one_run_and_one_worker() {
        let repository =
            Arc::new(SqliteRepository::open_in_memory().expect("open Auto Find repository"));
        enable_artist(&repository, "serein");
        let gate = Arc::new(SearchGate::default());
        let search = Arc::new(DeterministicSearchRepository::blocked(
            vec![gallery(701, "serein")],
            gate.clone(),
        ));
        let (events, _event_receiver) = mpsc::channel();
        let supervisor =
            AutoFindSupervisor::new(repository.clone(), repository, search.clone(), events);
        let barrier = Arc::new(Barrier::new(3));
        let callers = (0..2)
            .map(|_| {
                let supervisor = supervisor.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    supervisor.refresh().expect("start shared Auto Find run")
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let runs = callers
            .into_iter()
            .map(|caller| caller.join().expect("join refresh caller"))
            .collect::<Vec<_>>();
        gate.wait_until_entered();

        assert_eq!(runs[0].run_id, runs[1].run_id);
        assert_eq!(search.submission_count(), 1);
        supervisor.cancel().expect("cancel shared Auto Find run");
        gate.release();
        supervisor.shutdown_and_wait();
    }

    #[test]
    fn startup_recovery_fails_running_run_and_preserves_partial_candidates() {
        let temporary = tempfile::tempdir().expect("create recovery database directory");
        let database_path = temporary.path().join("recovery.sqlite3");
        let interrupted_run_id = {
            let repository =
                SqliteRepository::open(&database_path).expect("open recovery repository");
            let run = repository
                .auto_find_start(2, AutoFindHistoryMode::IncludeAllHistory, &[])
                .expect("seed running run");
            repository
                .auto_find_candidate_add(&AutoFindCandidateRecord {
                    run_id: run.run_id.clone(),
                    gallery: gallery(801, "serein"),
                    matched_favorite: FavoriteKey {
                        namespace: FavoriteNamespace::Artist,
                        value: "serein".into(),
                    },
                })
                .expect("seed partial candidate");
            run.run_id
        };

        let repository = Arc::new(
            SqliteRepository::open(&database_path).expect("reopen interrupted repository"),
        );
        let search = Arc::new(DeterministicSearchRepository::immediate(Vec::new()));
        let (events, _event_receiver) = mpsc::channel();
        let supervisor =
            AutoFindSupervisor::new(repository.clone(), repository.clone(), search, events);
        assert_eq!(
            supervisor
                .recover_interrupted()
                .expect("recover interrupted Auto Find run"),
            1
        );
        assert_eq!(
            supervisor
                .recover_interrupted()
                .expect("repeat idempotent recovery"),
            0
        );

        let snapshot = repository
            .auto_find_snapshot()
            .expect("load recovered Auto Find snapshot");
        let run = snapshot.run.expect("recovered Auto Find run");
        assert_eq!(run.run_id, interrupted_run_id);
        assert_eq!(run.state, AutoFindRunState::Failed);
        assert_eq!(run.error_code.as_deref(), Some("AUTO_FIND_INTERRUPTED"));
        assert_eq!(run.completed_favorites, 0);
        assert_eq!(run.candidates_found, 1);
        assert_eq!(snapshot.candidates.len(), 1);
        assert_eq!(snapshot.candidates[0].gallery.id.get(), 801);
    }

    fn enable_artist(repository: &Arc<SqliteRepository>, value: &str) {
        let service = ApplicationService::new(repository.clone())
            .with_automation_repository(repository.clone());
        service
            .favorite_set(
                FavoriteKey {
                    namespace: FavoriteNamespace::Artist,
                    value: value.into(),
                },
                true,
            )
            .expect("enable artist favorite");
    }

    fn wait_for_terminal(repository: &SqliteRepository) -> crate::domain::AutoFindSnapshot {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let snapshot = repository
                .auto_find_snapshot()
                .expect("read Auto Find snapshot while waiting");
            if snapshot
                .run
                .as_ref()
                .is_some_and(|run| run.state != AutoFindRunState::Running)
            {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "Auto Find run did not reach a terminal state"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn gallery(id: i64, artist: &str) -> GallerySummary {
        GallerySummary {
            id: GalleryId::new(id).expect("valid test gallery id"),
            title: format!("Gallery {id}"),
            artist: artist.into(),
            group: Some("test group".into()),
            series: vec!["test series".into()],
            characters: vec!["test character".into()],
            pages: 12,
            language: Language::Korean,
            tags: vec!["test".into()],
            published_rank: u32::try_from(id).expect("test rank fits u32"),
            popularity: 10,
            thumbnail_key: Some(format!("test-{id}")),
            thumbnail_width: 512,
            thumbnail_height: 512,
        }
    }
}
