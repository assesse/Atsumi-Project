use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap},
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError},
        Arc, Condvar, Mutex, Weak,
    },
    thread,
    time::{Duration, Instant},
};

use thiserror::Error;

use super::{
    CancellationToken, ResolvedThumbnail, ThumbnailCacheClearDto, ThumbnailCacheStatus,
    ThumbnailCompletionEventDto, ThumbnailDeliveryDto, ThumbnailFailureCode, ThumbnailFailureDto,
    ThumbnailInvalidationDto, ThumbnailKey, ThumbnailKeyError, ThumbnailPriority,
    ThumbnailRequestDto, ThumbnailRequestTokenDto, ThumbnailResolveError, ThumbnailResolver,
    ThumbnailResult, ThumbnailRuntimeConfigDto, ThumbnailWorkerStatsDto,
};

#[derive(Debug, Clone)]
pub struct ThumbnailCoordinatorConfig {
    pub max_concurrency: usize,
    pub request_start_interval: Duration,
    pub success_cache_capacity: usize,
    pub success_cache_max_bytes: usize,
    pub success_cache_ttl: Duration,
    pub negative_cache_capacity: usize,
    pub retryable_failure_cache_ttl: Duration,
    pub permanent_failure_cache_ttl: Duration,
}

impl Default for ThumbnailCoordinatorConfig {
    fn default() -> Self {
        Self {
            max_concurrency: 5,
            request_start_interval: Duration::from_millis(25),
            success_cache_capacity: 512,
            success_cache_max_bytes: 64 * 1024 * 1024,
            success_cache_ttl: Duration::from_secs(30 * 60),
            negative_cache_capacity: 512,
            retryable_failure_cache_ttl: Duration::from_secs(3),
            permanent_failure_cache_ttl: Duration::from_secs(5 * 60),
        }
    }
}

impl ThumbnailCoordinatorConfig {
    fn validate(&self) -> Result<(), ThumbnailCoordinatorError> {
        if !(1..=30).contains(&self.max_concurrency) {
            return Err(ThumbnailCoordinatorError::InvalidConfiguration(
                "max_concurrency must be between 1 and 30".into(),
            ));
        }
        if self.request_start_interval > Duration::from_secs(5) {
            return Err(ThumbnailCoordinatorError::InvalidConfiguration(
                "request_start_interval must be at most 5 seconds".into(),
            ));
        }
        if self.success_cache_capacity == 0 {
            return Err(ThumbnailCoordinatorError::InvalidConfiguration(
                "success_cache_capacity must be positive".into(),
            ));
        }
        if self.success_cache_max_bytes == 0 {
            return Err(ThumbnailCoordinatorError::InvalidConfiguration(
                "success_cache_max_bytes must be positive".into(),
            ));
        }
        if self.success_cache_ttl.is_zero() {
            return Err(ThumbnailCoordinatorError::InvalidConfiguration(
                "success_cache_ttl must be positive".into(),
            ));
        }
        if self.negative_cache_capacity == 0 {
            return Err(ThumbnailCoordinatorError::InvalidConfiguration(
                "negative_cache_capacity must be positive".into(),
            ));
        }
        if self.retryable_failure_cache_ttl.is_zero() {
            return Err(ThumbnailCoordinatorError::InvalidConfiguration(
                "retryable_failure_cache_ttl must be positive".into(),
            ));
        }
        if self.permanent_failure_cache_ttl.is_zero() {
            return Err(ThumbnailCoordinatorError::InvalidConfiguration(
                "permanent_failure_cache_ttl must be positive".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ThumbnailCoordinatorError {
    #[error("invalid thumbnail coordinator configuration: {0}")]
    InvalidConfiguration(String),
    #[error(transparent)]
    InvalidKey(#[from] ThumbnailKeyError),
    #[error("thumbnail coordinator is closed")]
    Closed,
    #[error("could not start thumbnail worker: {0}")]
    WorkerStart(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailReceiveTimeout {
    Timeout,
    Disconnected,
}

pub struct ThumbnailRequestHandle {
    token: ThumbnailRequestTokenDto,
    receiver: Option<Receiver<ThumbnailResult>>,
    core: Weak<CoordinatorCore>,
    finished: bool,
}

impl ThumbnailRequestHandle {
    pub fn token(&self) -> ThumbnailRequestTokenDto {
        self.token.clone()
    }

    pub fn request_id(&self) -> &str {
        &self.token.request_id
    }

    pub fn recv(mut self) -> ThumbnailResult {
        let result = self
            .receiver
            .take()
            .and_then(|receiver| receiver.recv().ok())
            .unwrap_or_else(|| {
                Err(ThumbnailFailureDto::coordinator_closed(
                    self.token.key.clone(),
                ))
            });
        self.finished = true;
        result
    }

    pub fn recv_timeout(
        &mut self,
        timeout: Duration,
    ) -> Result<ThumbnailResult, ThumbnailReceiveTimeout> {
        let Some(receiver) = self.receiver.as_ref() else {
            return Err(ThumbnailReceiveTimeout::Disconnected);
        };
        match receiver.recv_timeout(timeout) {
            Ok(result) => {
                self.finished = true;
                Ok(result)
            }
            Err(RecvTimeoutError::Timeout) => Err(ThumbnailReceiveTimeout::Timeout),
            Err(RecvTimeoutError::Disconnected) => {
                self.finished = true;
                Err(ThumbnailReceiveTimeout::Disconnected)
            }
        }
    }

    pub fn try_recv(&mut self) -> Option<ThumbnailResult> {
        let receiver = self.receiver.as_ref()?;
        match receiver.try_recv() {
            Ok(result) => {
                self.finished = true;
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.finished = true;
                Some(Err(ThumbnailFailureDto::coordinator_closed(
                    self.token.key.clone(),
                )))
            }
        }
    }

    pub fn cancel(&mut self) -> bool {
        if self.finished {
            return false;
        }
        self.finished = true;
        self.core
            .upgrade()
            .is_some_and(|core| core.cancel(&self.token.request_id))
    }
}

impl Drop for ThumbnailRequestHandle {
    fn drop(&mut self) {
        if !self.finished {
            if let Some(core) = self.core.upgrade() {
                core.cancel(&self.token.request_id);
            }
        }
    }
}

pub struct ThumbnailCoordinator {
    core: Arc<CoordinatorCore>,
}

impl Clone for ThumbnailCoordinator {
    fn clone(&self) -> Self {
        self.core
            .external_coordinators
            .fetch_add(1, AtomicOrdering::Relaxed);
        Self {
            core: Arc::clone(&self.core),
        }
    }
}

impl Drop for ThumbnailCoordinator {
    fn drop(&mut self) {
        if self
            .core
            .external_coordinators
            .fetch_sub(1, AtomicOrdering::AcqRel)
            == 1
        {
            self.core.close();
        }
    }
}

impl ThumbnailCoordinator {
    pub fn new(
        resolver: Arc<dyn ThumbnailResolver>,
        config: ThumbnailCoordinatorConfig,
    ) -> Result<Self, ThumbnailCoordinatorError> {
        config.validate()?;
        let core = Arc::new(CoordinatorCore {
            resolver,
            state: Mutex::new(CoordinatorState {
                concurrency_limit: config.max_concurrency,
                request_start_interval: config.request_start_interval,
                ..CoordinatorState::default()
            }),
            wake: Condvar::new(),
            config,
            external_coordinators: AtomicUsize::new(1),
            worker_threads: AtomicUsize::new(0),
            next_worker_index: AtomicUsize::new(0),
        });

        if let Err(error) = spawn_workers_to(&core, core.config.max_concurrency) {
            core.close();
            return Err(error);
        }

        Ok(Self { core })
    }

    pub fn with_resolver<R>(
        resolver: R,
        config: ThumbnailCoordinatorConfig,
    ) -> Result<Self, ThumbnailCoordinatorError>
    where
        R: ThumbnailResolver,
    {
        Self::new(Arc::new(resolver), config)
    }

    pub fn request(
        &self,
        request: ThumbnailRequestDto,
    ) -> Result<ThumbnailRequestHandle, ThumbnailCoordinatorError> {
        let (sender, receiver) = mpsc::channel();
        let token = self.enqueue(request, SubscriberSink::Handle(sender))?;
        Ok(ThumbnailRequestHandle {
            token,
            receiver: Some(receiver),
            core: Arc::downgrade(&self.core),
            finished: false,
        })
    }

    /// Subscribes directly to a process-wide completion channel, avoiding one
    /// blocking waiter task per thumbnail in large prefetch batches.
    pub fn request_with_completion(
        &self,
        request: ThumbnailRequestDto,
        completion: Sender<ThumbnailCompletionEventDto>,
    ) -> Result<ThumbnailRequestTokenDto, ThumbnailCoordinatorError> {
        self.enqueue(request, SubscriberSink::Completion(completion))
    }

    fn enqueue(
        &self,
        request: ThumbnailRequestDto,
        subscriber: SubscriberSink,
    ) -> Result<ThumbnailRequestTokenDto, ThumbnailCoordinatorError> {
        request.key.validate()?;
        let now = Instant::now();
        let mut state = self
            .core
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return Err(ThumbnailCoordinatorError::Closed);
        }
        state.remove_expired_cache_entries(now);
        state.counters.requests_total += 1;
        state.next_request += 1;
        let request_id = format!("thumbnail-{}", state.next_request);
        let token = ThumbnailRequestTokenDto {
            request_id: request_id.clone(),
            key: request.key.clone(),
        };

        state.next_cache_sequence += 1;
        let cache_sequence = state.next_cache_sequence;
        let cached_thumbnail = state.success_cache.get_mut(&request.key).map(|entry| {
            entry.last_used = cache_sequence;
            entry.thumbnail.clone()
        });
        if let Some(thumbnail) = cached_thumbnail {
            state.counters.success_cache_hits += 1;
            drop(state);
            subscriber.deliver(
                token.clone(),
                Ok(ThumbnailDeliveryDto {
                    key: request.key,
                    thumbnail,
                    cache_status: ThumbnailCacheStatus::Memory,
                }),
            );
            return Ok(token);
        }

        state.next_cache_sequence += 1;
        let negative_sequence = state.next_cache_sequence;
        let cached_failure = state.negative_cache.get_mut(&request.key).map(|entry| {
            entry.last_used = negative_sequence;
            entry.failure.clone()
        });
        if let Some(mut failure) = cached_failure {
            state.counters.negative_cache_hits += 1;
            failure.negative_cache_hit = true;
            drop(state);
            subscriber.deliver(token.clone(), Err(failure));
            return Ok(token);
        }

        let subscription = SubscriptionLocation {
            key: request.key.clone(),
            generation: 0,
        };
        let mut queue_item = None;
        if state.work.contains_key(&request.key) {
            let generation = {
                let work = state
                    .work
                    .get_mut(&request.key)
                    .expect("work existence was checked");
                let generation = work.generation;
                work.subscribers.insert(request_id.clone(), subscriber);
                if work.status == WorkStatus::Queued
                    && request.priority.rank() > work.priority.rank()
                {
                    work.priority = request.priority;
                    work.queue_version += 1;
                    queue_item = Some(QueueItem {
                        key: request.key.clone(),
                        generation,
                        queue_version: work.queue_version,
                        priority: work.priority,
                        sequence: work.queue_sequence,
                    });
                }
                generation
            };
            state.counters.joined_in_flight += 1;
            state.subscriptions.insert(
                request_id,
                SubscriptionLocation {
                    generation,
                    ..subscription
                },
            );
        } else {
            state.next_generation += 1;
            state.next_queue_sequence += 1;
            let generation = state.next_generation;
            let sequence = state.next_queue_sequence;
            let mut subscribers = HashMap::new();
            subscribers.insert(request_id.clone(), subscriber);
            state.work.insert(
                request.key.clone(),
                WorkEntry {
                    generation,
                    queue_version: 1,
                    queue_sequence: sequence,
                    priority: request.priority,
                    status: WorkStatus::Queued,
                    cancellation: CancellationToken::new(),
                    subscribers,
                },
            );
            state.subscriptions.insert(
                request_id,
                SubscriptionLocation {
                    generation,
                    ..subscription
                },
            );
            queue_item = Some(QueueItem {
                key: request.key.clone(),
                generation,
                queue_version: 1,
                priority: request.priority,
                sequence,
            });
        }
        if let Some(item) = queue_item {
            state.queue.push(item);
        }
        drop(state);
        self.core.wake.notify_one();

        Ok(token)
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        self.core.cancel(request_id)
    }

    /// Removes cached data for a key without cancelling or reprioritizing any
    /// queued or running work. Intended for display decode failures and
    /// explicit retry actions.
    pub fn invalidate(
        &self,
        key: &ThumbnailKey,
    ) -> Result<ThumbnailInvalidationDto, ThumbnailKeyError> {
        key.validate()?;
        let mut state = self
            .core
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let success_cache_removed = if let Some(entry) = state.success_cache.remove(key) {
            state.success_cache_bytes = state
                .success_cache_bytes
                .saturating_sub(entry.thumbnail.byte_len());
            true
        } else {
            false
        };
        let negative_cache_removed = state.negative_cache.remove(key).is_some();
        Ok(ThumbnailInvalidationDto {
            key: key.clone(),
            success_cache_removed,
            negative_cache_removed,
        })
    }

    /// Clears only completed positive/negative cache entries. Queued/running
    /// work and current subscribers are intentionally left untouched.
    pub fn clear_cache(&self) -> ThumbnailCacheClearDto {
        let mut state = self
            .core
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let result = ThumbnailCacheClearDto {
            success_entries_removed: state.success_cache.len(),
            success_bytes_removed: state.success_cache_bytes,
            negative_entries_removed: state.negative_cache.len(),
        };
        state.success_cache.clear();
        state.success_cache_bytes = 0;
        state.negative_cache.clear();
        result
    }

    /// Raises the priority of queued work for an existing subscriber. Lower
    /// priorities are ignored, and already-running work reports success as a
    /// no-op because it cannot benefit from queue reordering.
    pub fn reprioritize(&self, request_id: &str, priority: ThumbnailPriority) -> bool {
        let mut state = self
            .core
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(location) = state.subscriptions.get(request_id).cloned() else {
            return false;
        };
        let mut queue_item = None;
        if let Some(work) = state.work.get_mut(&location.key) {
            if work.generation != location.generation {
                return false;
            }
            if work.status == WorkStatus::Queued && priority.rank() > work.priority.rank() {
                work.priority = priority;
                work.queue_version += 1;
                queue_item = Some(QueueItem {
                    key: location.key,
                    generation: work.generation,
                    queue_version: work.queue_version,
                    priority: work.priority,
                    sequence: work.queue_sequence,
                });
            }
        } else {
            return false;
        }
        if let Some(item) = queue_item {
            state.queue.push(item);
            drop(state);
            self.core.wake.notify_one();
        }
        true
    }

    /// Applies the image-related user settings to work which has not started.
    /// Values are clamped to the same safe bounds as SettingsSnapshot.
    pub fn reconfigure(
        &self,
        requested: ThumbnailRuntimeConfigDto,
    ) -> Result<ThumbnailRuntimeConfigDto, ThumbnailCoordinatorError> {
        let concurrent_image_requests = requested.concurrent_image_requests.clamp(1, 30);
        let request_start_interval_ms = requested.request_start_interval_ms.min(5_000);
        spawn_workers_to(&self.core, concurrent_image_requests as usize)?;

        let mut state = self
            .core
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return Err(ThumbnailCoordinatorError::Closed);
        }
        state.concurrency_limit = concurrent_image_requests as usize;
        state.request_start_interval = Duration::from_millis(request_start_interval_ms);
        drop(state);
        self.core.wake.notify_all();
        Ok(ThumbnailRuntimeConfigDto {
            concurrent_image_requests,
            request_start_interval_ms,
        })
    }

    pub fn stats(&self) -> ThumbnailWorkerStatsDto {
        let now = Instant::now();
        let mut state = self
            .core
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.remove_expired_cache_entries(now);
        let queued_keys = state
            .work
            .values()
            .filter(|work| work.status == WorkStatus::Queued)
            .count();
        let in_flight_keys = state
            .work
            .values()
            .filter(|work| work.status == WorkStatus::Running)
            .count();
        ThumbnailWorkerStatsDto {
            worker_count: self.core.worker_threads.load(AtomicOrdering::Acquire),
            concurrency_limit: state.concurrency_limit,
            request_start_interval_ms: state.request_start_interval.as_millis() as u64,
            active_workers: state.active_workers,
            queued_keys,
            in_flight_keys,
            subscriber_count: state.subscriptions.len(),
            success_cache_entries: state.success_cache.len(),
            success_cache_bytes: state.success_cache_bytes,
            negative_cache_entries: state.negative_cache.len(),
            requests_total: state.counters.requests_total,
            success_cache_hits: state.counters.success_cache_hits,
            negative_cache_hits: state.counters.negative_cache_hits,
            joined_in_flight: state.counters.joined_in_flight,
            resolved_success: state.counters.resolved_success,
            resolved_failure: state.counters.resolved_failure,
            cancelled_subscribers: state.counters.cancelled_subscribers,
            cancelled_work: state.counters.cancelled_work,
        }
    }

    pub fn wait_for_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        let mut state = self
            .core
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            if state.work.is_empty() && state.active_workers == 0 {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            let remaining = deadline - now;
            let waited = self.core.wake.wait_timeout(state, remaining);
            let (next_state, timeout_result) = waited.unwrap_or_else(|error| error.into_inner());
            state = next_state;
            if timeout_result.timed_out() && (!state.work.is_empty() || state.active_workers > 0) {
                return false;
            }
        }
    }

    pub fn shutdown(&self) {
        self.core.close();
    }
}

struct CoordinatorCore {
    resolver: Arc<dyn ThumbnailResolver>,
    state: Mutex<CoordinatorState>,
    wake: Condvar,
    config: ThumbnailCoordinatorConfig,
    external_coordinators: AtomicUsize,
    worker_threads: AtomicUsize,
    next_worker_index: AtomicUsize,
}

impl CoordinatorCore {
    fn cancel(&self, request_id: &str) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(location) = state.subscriptions.remove(request_id) else {
            return false;
        };

        let mut cancelled_subscriber = None;
        let mut cancel_entire_work = false;
        if let Some(work) = state.work.get_mut(&location.key) {
            if work.generation == location.generation {
                cancelled_subscriber = work.subscribers.remove(request_id);
                if work.subscribers.is_empty() {
                    work.cancellation.cancel();
                    cancel_entire_work = true;
                }
            }
        }
        state.counters.cancelled_subscribers += 1;
        if cancel_entire_work {
            state.work.remove(&location.key);
            state.counters.cancelled_work += 1;
        }
        drop(state);

        if let Some(subscriber) = cancelled_subscriber {
            let token = ThumbnailRequestTokenDto {
                request_id: request_id.to_owned(),
                key: location.key.clone(),
            };
            subscriber.deliver(token, Err(ThumbnailFailureDto::cancelled(location.key)));
        }
        self.wake.notify_all();
        true
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return;
        }
        state.closed = true;
        let work = std::mem::take(&mut state.work);
        state.queue.clear();
        state.subscriptions.clear();
        let mut cancelled = Vec::new();
        for (key, entry) in work {
            entry.cancellation.cancel();
            cancelled.extend(
                entry
                    .subscribers
                    .into_iter()
                    .map(|(request_id, subscriber)| {
                        (
                            subscriber,
                            ThumbnailRequestTokenDto {
                                request_id,
                                key: key.clone(),
                            },
                        )
                    }),
            );
        }
        drop(state);
        for (subscriber, token) in cancelled {
            let failure = ThumbnailFailureDto::coordinator_closed(token.key.clone());
            subscriber.deliver(token, Err(failure));
        }
        self.wake.notify_all();
    }
}

#[derive(Default)]
struct CoordinatorState {
    closed: bool,
    queue: BinaryHeap<QueueItem>,
    work: HashMap<ThumbnailKey, WorkEntry>,
    subscriptions: HashMap<String, SubscriptionLocation>,
    success_cache: HashMap<ThumbnailKey, SuccessCacheEntry>,
    success_cache_bytes: usize,
    negative_cache: HashMap<ThumbnailKey, NegativeCacheEntry>,
    active_workers: usize,
    concurrency_limit: usize,
    request_start_interval: Duration,
    last_started_at: Option<Instant>,
    next_request: u64,
    next_generation: u64,
    next_queue_sequence: u64,
    next_cache_sequence: u64,
    counters: CoordinatorCounters,
}

impl CoordinatorState {
    fn remove_expired_cache_entries(&mut self, now: Instant) {
        let expired_success: Vec<_> = self
            .success_cache
            .iter()
            .filter_map(|(key, entry)| (entry.expires_at <= now).then_some(key.clone()))
            .collect();
        for key in expired_success {
            if let Some(entry) = self.success_cache.remove(&key) {
                self.success_cache_bytes = self
                    .success_cache_bytes
                    .saturating_sub(entry.thumbnail.byte_len());
            }
        }
        self.negative_cache
            .retain(|_, entry| entry.expires_at > now);
    }
}

#[derive(Default)]
struct CoordinatorCounters {
    requests_total: u64,
    success_cache_hits: u64,
    negative_cache_hits: u64,
    joined_in_flight: u64,
    resolved_success: u64,
    resolved_failure: u64,
    cancelled_subscribers: u64,
    cancelled_work: u64,
}

struct WorkEntry {
    generation: u64,
    queue_version: u64,
    queue_sequence: u64,
    priority: ThumbnailPriority,
    status: WorkStatus,
    cancellation: CancellationToken,
    subscribers: HashMap<String, SubscriberSink>,
}

enum SubscriberSink {
    Handle(Sender<ThumbnailResult>),
    Completion(Sender<ThumbnailCompletionEventDto>),
}

impl SubscriberSink {
    fn deliver(self, token: ThumbnailRequestTokenDto, result: ThumbnailResult) {
        match self {
            Self::Handle(sender) => {
                let _ = sender.send(result);
            }
            Self::Completion(sender) => {
                let _ = sender.send(ThumbnailCompletionEventDto::from_result(token, result));
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkStatus {
    Queued,
    Running,
}

#[derive(Clone)]
struct SubscriptionLocation {
    key: ThumbnailKey,
    generation: u64,
}

struct SuccessCacheEntry {
    thumbnail: ResolvedThumbnail,
    expires_at: Instant,
    last_used: u64,
}

struct NegativeCacheEntry {
    failure: ThumbnailFailureDto,
    expires_at: Instant,
    last_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueueItem {
    key: ThumbnailKey,
    generation: u64,
    queue_version: u64,
    priority: ThumbnailPriority,
    sequence: u64,
}

impl Ord for QueueItem {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority
            .rank()
            .cmp(&other.priority.rank())
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

impl PartialOrd for QueueItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn worker_loop(core: Arc<CoordinatorCore>) {
    loop {
        let Some((item, cancellation)) = next_work(&core) else {
            return;
        };
        let resolution = catch_unwind(AssertUnwindSafe(|| {
            core.resolver
                .resolve_with_priority(&item.key, &cancellation, item.priority)
        }))
        .unwrap_or_else(|_| {
            Err(ThumbnailResolveError::new(
                ThumbnailFailureCode::Resolver,
                "thumbnail resolver panicked",
                true,
            ))
        });
        complete_work(&core, item, resolution);
    }
}

fn spawn_workers_to(
    core: &Arc<CoordinatorCore>,
    target: usize,
) -> Result<(), ThumbnailCoordinatorError> {
    if core
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .closed
    {
        return Err(ThumbnailCoordinatorError::Closed);
    }
    loop {
        let current = core.worker_threads.load(AtomicOrdering::Acquire);
        if current >= target {
            return Ok(());
        }
        if core
            .worker_threads
            .compare_exchange(
                current,
                current + 1,
                AtomicOrdering::AcqRel,
                AtomicOrdering::Acquire,
            )
            .is_err()
        {
            continue;
        }

        let worker_index = core.next_worker_index.fetch_add(1, AtomicOrdering::Relaxed);
        let worker_core = Arc::clone(core);
        let spawn = thread::Builder::new()
            .name(format!("atsumi-thumbnail-{worker_index}"))
            .spawn(move || {
                worker_loop(Arc::clone(&worker_core));
                worker_core
                    .worker_threads
                    .fetch_sub(1, AtomicOrdering::AcqRel);
            });
        if let Err(error) = spawn {
            core.worker_threads.fetch_sub(1, AtomicOrdering::AcqRel);
            return Err(ThumbnailCoordinatorError::WorkerStart(error.to_string()));
        }
    }
}

fn next_work(core: &CoordinatorCore) -> Option<(QueueItem, CancellationToken)> {
    let mut state = core.state.lock().unwrap_or_else(|error| error.into_inner());
    loop {
        if state.closed {
            return None;
        }
        while state.queue.peek().is_some_and(|item| {
            state.work.get(&item.key).is_none_or(|work| {
                work.generation != item.generation
                    || work.queue_version != item.queue_version
                    || work.status != WorkStatus::Queued
            })
        }) {
            state.queue.pop();
        }
        if state.queue.is_empty() || state.active_workers >= state.concurrency_limit {
            state = core
                .wake
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
            continue;
        }

        if let Some(last_started_at) = state.last_started_at {
            let next_start = last_started_at
                .checked_add(state.request_start_interval)
                .unwrap_or(last_started_at);
            let now = Instant::now();
            if now < next_start {
                let waited = core.wake.wait_timeout(state, next_start - now);
                state = waited.unwrap_or_else(|error| error.into_inner()).0;
                continue;
            }
        }

        let item = state.queue.pop().expect("queue was checked as non-empty");
        let cancellation = {
            let work = state
                .work
                .get_mut(&item.key)
                .expect("stale queue entries were removed");
            work.status = WorkStatus::Running;
            work.cancellation.clone()
        };
        state.active_workers += 1;
        state.last_started_at = Some(Instant::now());
        return Some((item, cancellation));
    }
}

fn complete_work(
    core: &CoordinatorCore,
    item: QueueItem,
    mut resolution: Result<ResolvedThumbnail, ThumbnailResolveError>,
) {
    if let Ok(thumbnail) = &resolution {
        if let Err(message) = thumbnail.validate() {
            resolution = Err(ThumbnailResolveError::new(
                ThumbnailFailureCode::InvalidData,
                message,
                false,
            ));
        }
    }

    let mut state = core.state.lock().unwrap_or_else(|error| error.into_inner());
    state.active_workers = state.active_workers.saturating_sub(1);
    let is_current = state
        .work
        .get(&item.key)
        .is_some_and(|work| work.generation == item.generation);
    if !is_current {
        drop(state);
        core.wake.notify_all();
        return;
    }
    let Some(work) = state.work.remove(&item.key) else {
        return;
    };
    for request_id in work.subscribers.keys() {
        state.subscriptions.remove(request_id);
    }

    let completion_key = item.key.clone();
    let result = match resolution {
        Ok(thumbnail) => {
            state.counters.resolved_success += 1;
            insert_success_cache(
                &mut state,
                &core.config,
                item.key.clone(),
                thumbnail.clone(),
            );
            Ok(ThumbnailDeliveryDto {
                key: item.key,
                thumbnail,
                cache_status: ThumbnailCacheStatus::Resolved,
            })
        }
        Err(error) => {
            state.counters.resolved_failure += 1;
            let failure = ThumbnailFailureDto {
                key: item.key.clone(),
                code: error.code,
                message: error.message,
                retryable: error.retryable,
                negative_cache_hit: false,
            };
            if !matches!(
                failure.code,
                ThumbnailFailureCode::Cancelled | ThumbnailFailureCode::CoordinatorClosed
            ) {
                let ttl = if failure.retryable {
                    core.config.retryable_failure_cache_ttl
                } else {
                    core.config.permanent_failure_cache_ttl
                };
                insert_negative_cache(&mut state, &core.config, item.key, failure.clone(), ttl);
            }
            Err(failure)
        }
    };
    let subscribers: Vec<_> = work.subscribers.into_iter().collect();
    drop(state);
    for (request_id, subscriber) in subscribers {
        subscriber.deliver(
            ThumbnailRequestTokenDto {
                request_id,
                key: completion_key.clone(),
            },
            result.clone(),
        );
    }
    core.wake.notify_all();
}

fn insert_success_cache(
    state: &mut CoordinatorState,
    config: &ThumbnailCoordinatorConfig,
    key: ThumbnailKey,
    thumbnail: ResolvedThumbnail,
) {
    let byte_len = thumbnail.byte_len();
    if byte_len > config.success_cache_max_bytes {
        return;
    }
    state.remove_expired_cache_entries(Instant::now());
    while state.success_cache.len() >= config.success_cache_capacity
        || state.success_cache_bytes.saturating_add(byte_len) > config.success_cache_max_bytes
    {
        let Some(eviction_key) = state
            .success_cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        if let Some(evicted) = state.success_cache.remove(&eviction_key) {
            state.success_cache_bytes = state
                .success_cache_bytes
                .saturating_sub(evicted.thumbnail.byte_len());
        }
    }
    state.next_cache_sequence += 1;
    let expires_at = expiry_after(config.success_cache_ttl);
    if let Some(previous) = state.success_cache.insert(
        key,
        SuccessCacheEntry {
            thumbnail,
            expires_at,
            last_used: state.next_cache_sequence,
        },
    ) {
        state.success_cache_bytes = state
            .success_cache_bytes
            .saturating_sub(previous.thumbnail.byte_len());
    }
    state.success_cache_bytes = state.success_cache_bytes.saturating_add(byte_len);
}

fn insert_negative_cache(
    state: &mut CoordinatorState,
    config: &ThumbnailCoordinatorConfig,
    key: ThumbnailKey,
    failure: ThumbnailFailureDto,
    ttl: Duration,
) {
    state.remove_expired_cache_entries(Instant::now());
    while state.negative_cache.len() >= config.negative_cache_capacity {
        let Some(eviction_key) = state
            .negative_cache
            .iter()
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        state.negative_cache.remove(&eviction_key);
    }
    state.next_cache_sequence += 1;
    state.negative_cache.insert(
        key,
        NegativeCacheEntry {
            failure,
            expires_at: expiry_after(ttl),
            last_used: state.next_cache_sequence,
        },
    );
}

fn expiry_after(ttl: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(ttl).unwrap_or(now)
}
