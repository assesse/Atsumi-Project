use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use super::*;

fn request(key: ThumbnailKey, priority: ThumbnailPriority) -> ThumbnailRequestDto {
    ThumbnailRequestDto {
        key,
        consumer: ThumbnailConsumer::Explore,
        priority,
    }
}

fn config(concurrency: usize) -> ThumbnailCoordinatorConfig {
    ThumbnailCoordinatorConfig {
        max_concurrency: concurrency,
        request_start_interval: Duration::ZERO,
        success_cache_ttl: Duration::from_secs(60),
        retryable_failure_cache_ttl: Duration::from_millis(30),
        permanent_failure_cache_ttl: Duration::from_millis(80),
        ..ThumbnailCoordinatorConfig::default()
    }
}

#[test]
fn key_and_event_dtos_use_the_frontend_contract() {
    let key = ThumbnailKey::gallery_page(73, 4).unwrap();
    assert_eq!(
        serde_json::to_value(&key).unwrap(),
        serde_json::json!({
            "kind": "galleryPage",
            "galleryId": 73,
            "sourcePage": 4
        })
    );
    assert!(ThumbnailKey::gallery_cover(0).is_err());
    assert!(ThumbnailKey::gallery_page(1, 0).is_err());
    let artifact = ThumbnailKey::artifact_page("entry-73", 4).unwrap();
    assert_eq!(
        serde_json::to_value(&artifact).unwrap(),
        serde_json::json!({
            "kind": "artifactPage",
            "entryId": "entry-73",
            "sourcePage": 4
        })
    );
    assert!(ThumbnailKey::artifact_page("", 1).is_err());
    assert!(ThumbnailKey::artifact_page("entry-73", 0).is_err());

    let request = ThumbnailRequestDto {
        key,
        consumer: ThumbnailConsumer::Downloads,
        priority: ThumbnailPriority::Critical,
    };
    let serialized = serde_json::to_value(request).unwrap();
    assert_eq!(serialized["consumer"], "downloads");
    assert_eq!(serialized["priority"], "critical");

    let token = ThumbnailRequestTokenDto {
        request_id: "thumbnail-9".into(),
        key: ThumbnailKey::gallery_cover(73).unwrap(),
    };
    let event = ThumbnailCompletionEventDto::from_result(
        token,
        Err(ThumbnailFailureDto {
            key: ThumbnailKey::gallery_cover(73).unwrap(),
            code: ThumbnailFailureCode::NotFound,
            message: "missing".into(),
            retryable: false,
            negative_cache_hit: false,
        }),
    );
    let event = serde_json::to_value(event).unwrap();
    assert_eq!(event["requestId"], "thumbnail-9");
    assert_eq!(event["outcome"]["status"], "failed");
}

#[test]
fn fixture_resolution_is_deterministic_and_page_specific() {
    let resolver = FixtureThumbnailResolver::new();
    let cancellation = CancellationToken::new();
    let cover_key = ThumbnailKey::gallery_cover(10).unwrap();
    let page_key = ThumbnailKey::gallery_page(10, 1).unwrap();
    let first = resolver.resolve(&cover_key, &cancellation).unwrap();
    let second = resolver.resolve(&cover_key, &cancellation).unwrap();
    let page = resolver.resolve(&page_key, &cancellation).unwrap();

    assert_eq!(first, second);
    assert_ne!(first.bytes, page.bytes);
    assert_eq!(first.content_type, "image/svg+xml");
    first.validate().unwrap();
}

struct CountingResolver {
    calls: AtomicUsize,
    active: AtomicUsize,
    max_active: AtomicUsize,
    latency: Duration,
}

impl CountingResolver {
    fn new(latency: Duration) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            max_active: AtomicUsize::new(0),
            latency,
        }
    }
}

impl ThumbnailResolver for CountingResolver {
    fn resolve(
        &self,
        key: &ThumbnailKey,
        cancellation: &CancellationToken,
    ) -> Result<ResolvedThumbnail, ThumbnailResolveError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(active, Ordering::SeqCst);
        let deadline = Instant::now() + self.latency;
        while Instant::now() < deadline {
            if cancellation.is_cancelled() {
                self.active.fetch_sub(1, Ordering::SeqCst);
                return Err(ThumbnailResolveError::cancelled());
            }
            thread::sleep(Duration::from_millis(2));
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(ResolvedThumbnail {
            content_type: "image/png".into(),
            bytes: key.cache_id().into_bytes(),
            width: 32,
            height: 32,
            source_revision: Some("counting-v1".into()),
        })
    }
}

#[test]
fn identical_in_flight_requests_share_one_resolution() {
    let resolver = Arc::new(CountingResolver::new(Duration::from_millis(35)));
    let coordinator = ThumbnailCoordinator::new(resolver.clone(), config(2)).unwrap();
    let key = ThumbnailKey::gallery_cover(1).unwrap();
    let first = coordinator
        .request(request(key.clone(), ThumbnailPriority::Visible))
        .unwrap();
    let second = coordinator
        .request(request(key, ThumbnailPriority::Critical))
        .unwrap();

    assert!(first.recv().is_ok());
    assert!(second.recv().is_ok());
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(coordinator.stats().joined_in_flight, 1);
    coordinator.shutdown();
}

#[test]
fn process_wide_completion_channel_needs_no_per_request_waiter() {
    let resolver = Arc::new(CountingResolver::new(Duration::from_millis(30)));
    let coordinator = ThumbnailCoordinator::new(resolver.clone(), config(2)).unwrap();
    let key = ThumbnailKey::gallery_cover(2).unwrap();
    let (completion_sender, completion_receiver) = mpsc::channel();
    let first = coordinator
        .request_with_completion(
            request(key.clone(), ThumbnailPriority::Prefetch),
            completion_sender.clone(),
        )
        .unwrap();
    let second = coordinator
        .request_with_completion(request(key, ThumbnailPriority::Visible), completion_sender)
        .unwrap();

    assert!(coordinator.cancel(&first.request_id));
    let cancelled = completion_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(cancelled.request_id, first.request_id);
    assert!(matches!(
        cancelled.outcome,
        ThumbnailCompletionOutcomeDto::Failed {
            failure: ThumbnailFailureDto {
                code: ThumbnailFailureCode::Cancelled,
                ..
            }
        }
    ));

    let ready = completion_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    assert_eq!(ready.request_id, second.request_id);
    assert!(matches!(
        ready.outcome,
        ThumbnailCompletionOutcomeDto::Ready { .. }
    ));
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(coordinator.stats().joined_in_flight, 1);
    coordinator.shutdown();
}

#[test]
fn worker_concurrency_is_bounded() {
    let resolver = Arc::new(CountingResolver::new(Duration::from_millis(40)));
    let coordinator = ThumbnailCoordinator::new(resolver.clone(), config(2)).unwrap();
    let handles: Vec<_> = (1..=6)
        .map(|gallery_id| {
            coordinator
                .request(request(
                    ThumbnailKey::gallery_cover(gallery_id).unwrap(),
                    ThumbnailPriority::Visible,
                ))
                .unwrap()
        })
        .collect();
    for handle in handles {
        assert!(handle.recv().is_ok());
    }

    assert_eq!(resolver.calls.load(Ordering::SeqCst), 6);
    assert_eq!(resolver.max_active.load(Ordering::SeqCst), 2);
    coordinator.shutdown();
}

#[test]
fn cancelling_one_subscriber_keeps_shared_work_alive() {
    let resolver = Arc::new(CountingResolver::new(Duration::from_millis(35)));
    let coordinator = ThumbnailCoordinator::new(resolver.clone(), config(1)).unwrap();
    let key = ThumbnailKey::gallery_cover(3).unwrap();
    let mut first = coordinator
        .request(request(key.clone(), ThumbnailPriority::Visible))
        .unwrap();
    let second = coordinator
        .request(request(key, ThumbnailPriority::Visible))
        .unwrap();

    assert!(first.cancel());
    assert!(second.recv().is_ok());
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    let stats = coordinator.stats();
    assert_eq!(stats.cancelled_subscribers, 1);
    assert_eq!(stats.cancelled_work, 0);
    coordinator.shutdown();
}

#[test]
fn last_subscriber_cancels_queued_work() {
    let resolver = Arc::new(CountingResolver::new(Duration::from_millis(50)));
    let coordinator = ThumbnailCoordinator::new(resolver.clone(), config(1)).unwrap();
    let blocker = coordinator
        .request(request(
            ThumbnailKey::gallery_cover(4).unwrap(),
            ThumbnailPriority::Visible,
        ))
        .unwrap();
    while coordinator.stats().active_workers == 0 {
        thread::sleep(Duration::from_millis(1));
    }
    let mut queued = coordinator
        .request(request(
            ThumbnailKey::gallery_cover(5).unwrap(),
            ThumbnailPriority::Prefetch,
        ))
        .unwrap();
    assert!(queued.cancel());
    assert!(blocker.recv().is_ok());
    assert!(coordinator.wait_for_idle(Duration::from_secs(1)));

    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    assert_eq!(coordinator.stats().cancelled_work, 1);
    coordinator.shutdown();
}

#[test]
fn success_and_retryable_failure_caches_suppress_duplicate_work() {
    let success_resolver = Arc::new(CountingResolver::new(Duration::ZERO));
    let coordinator = ThumbnailCoordinator::new(success_resolver.clone(), config(1)).unwrap();
    let key = ThumbnailKey::gallery_cover(6).unwrap();
    assert!(coordinator
        .request(request(key.clone(), ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .is_ok());
    let cached = coordinator
        .request(request(key, ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .unwrap();
    assert_eq!(cached.cache_status, ThumbnailCacheStatus::Memory);
    assert_eq!(success_resolver.calls.load(Ordering::SeqCst), 1);
    coordinator.shutdown();

    let failure_key = ThumbnailKey::gallery_cover(7).unwrap();
    let failure_resolver = FixtureThumbnailResolver::new().with_failure(
        failure_key.clone(),
        ThumbnailResolveError::temporarily_unavailable("fixture outage"),
    );
    let failure_coordinator =
        ThumbnailCoordinator::with_resolver(failure_resolver, config(1)).unwrap();
    let first = failure_coordinator
        .request(request(failure_key.clone(), ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .unwrap_err();
    let second = failure_coordinator
        .request(request(failure_key, ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .unwrap_err();
    assert!(!first.negative_cache_hit);
    assert!(second.negative_cache_hit);
    assert_eq!(failure_coordinator.stats().resolved_failure, 1);
    thread::sleep(Duration::from_millis(40));
    let third = failure_coordinator
        .request(request(
            ThumbnailKey::gallery_cover(7).unwrap(),
            ThumbnailPriority::Visible,
        ))
        .unwrap()
        .recv()
        .unwrap_err();
    assert!(!third.negative_cache_hit);
    assert_eq!(failure_coordinator.stats().resolved_failure, 2);
    failure_coordinator.shutdown();
}

#[test]
fn permanent_failures_use_the_longer_negative_cache_ttl() {
    let key = ThumbnailKey::gallery_cover(8).unwrap();
    let resolver = FixtureThumbnailResolver::new().with_failure(
        key.clone(),
        ThumbnailResolveError::new(ThumbnailFailureCode::NotFound, "gone", false),
    );
    let coordinator = ThumbnailCoordinator::with_resolver(resolver, config(1)).unwrap();

    let first = coordinator
        .request(request(key.clone(), ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .unwrap_err();
    let second = coordinator
        .request(request(key.clone(), ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .unwrap_err();
    assert_eq!(first.code, ThumbnailFailureCode::NotFound);
    assert!(!first.negative_cache_hit);
    assert!(second.negative_cache_hit);
    assert_eq!(coordinator.stats().resolved_failure, 1);

    thread::sleep(Duration::from_millis(90));
    let after_ttl = coordinator
        .request(request(key, ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .unwrap_err();
    assert!(!after_ttl.negative_cache_hit);
    assert_eq!(coordinator.stats().resolved_failure, 2);
    coordinator.shutdown();
}

#[test]
fn cancellation_and_coordinator_closed_failures_are_never_cached() {
    let cancelled_key = ThumbnailKey::gallery_cover(9).unwrap();
    let closed_key = ThumbnailKey::gallery_cover(10).unwrap();
    let resolver = FixtureThumbnailResolver::new()
        .with_failure(cancelled_key.clone(), ThumbnailResolveError::cancelled())
        .with_failure(
            closed_key.clone(),
            ThumbnailResolveError::new(
                ThumbnailFailureCode::CoordinatorClosed,
                "fixture closed",
                false,
            ),
        );
    let coordinator = ThumbnailCoordinator::with_resolver(resolver, config(1)).unwrap();

    for key in [cancelled_key, closed_key] {
        for _ in 0..2 {
            let failure = coordinator
                .request(request(key.clone(), ThumbnailPriority::Visible))
                .unwrap()
                .recv()
                .unwrap_err();
            assert!(!failure.negative_cache_hit);
        }
    }
    let stats = coordinator.stats();
    assert_eq!(stats.negative_cache_entries, 0);
    assert_eq!(stats.resolved_failure, 4);
    coordinator.shutdown();
}

#[test]
fn invalidation_evicts_cached_data_and_leaves_in_flight_work_alone() {
    let resolver = Arc::new(CountingResolver::new(Duration::from_millis(35)));
    let coordinator = ThumbnailCoordinator::new(resolver.clone(), config(1)).unwrap();
    let key = ThumbnailKey::gallery_cover(15).unwrap();

    assert!(coordinator
        .request(request(key.clone(), ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .is_ok());
    assert_eq!(coordinator.stats().success_cache_entries, 1);
    let invalidation = coordinator.invalidate(&key).unwrap();
    assert!(invalidation.success_cache_removed);
    assert!(!invalidation.negative_cache_removed);
    assert_eq!(coordinator.stats().success_cache_entries, 0);

    let in_flight = coordinator
        .request(request(key.clone(), ThumbnailPriority::Visible))
        .unwrap();
    while coordinator.stats().active_workers == 0 {
        thread::sleep(Duration::from_millis(1));
    }
    let while_running = coordinator.invalidate(&key).unwrap();
    assert!(!while_running.success_cache_removed);
    assert!(!while_running.negative_cache_removed);
    assert!(in_flight.recv().is_ok());
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    assert_eq!(coordinator.stats().cancelled_work, 0);
    assert_eq!(coordinator.stats().success_cache_entries, 1);
    coordinator.shutdown();
}

#[test]
fn invalidation_also_allows_an_explicit_retry_of_negative_cache() {
    let key = ThumbnailKey::gallery_cover(16).unwrap();
    let resolver = FixtureThumbnailResolver::new().with_failure(
        key.clone(),
        ThumbnailResolveError::new(ThumbnailFailureCode::InvalidData, "bad image", false),
    );
    let coordinator = ThumbnailCoordinator::with_resolver(resolver, config(1)).unwrap();
    assert!(coordinator
        .request(request(key.clone(), ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .is_err());
    assert_eq!(coordinator.stats().negative_cache_entries, 1);
    let invalidation = coordinator.invalidate(&key).unwrap();
    assert!(!invalidation.success_cache_removed);
    assert!(invalidation.negative_cache_removed);
    assert_eq!(coordinator.stats().negative_cache_entries, 0);
    let next = coordinator
        .request(request(key, ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .unwrap_err();
    assert!(!next.negative_cache_hit);
    assert_eq!(coordinator.stats().resolved_failure, 2);
    coordinator.shutdown();
}

#[test]
fn explicit_cache_clear_removes_completed_caches_without_touching_active_work() {
    let success_key = ThumbnailKey::gallery_cover(17).unwrap();
    let failure_key = ThumbnailKey::gallery_cover(18).unwrap();
    let resolver = FixtureThumbnailResolver::new().with_failure(
        failure_key.clone(),
        ThumbnailResolveError::new(ThumbnailFailureCode::NotFound, "gone", false),
    );
    let coordinator = ThumbnailCoordinator::with_resolver(resolver, config(1)).unwrap();
    assert!(coordinator
        .request(request(success_key, ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .is_ok());
    assert!(coordinator
        .request(request(failure_key, ThumbnailPriority::Visible))
        .unwrap()
        .recv()
        .is_err());
    let before = coordinator.stats();
    assert_eq!(before.success_cache_entries, 1);
    assert_eq!(before.negative_cache_entries, 1);

    let cleared = coordinator.clear_cache();
    assert_eq!(cleared.success_entries_removed, 1);
    assert!(cleared.success_bytes_removed > 0);
    assert_eq!(cleared.negative_entries_removed, 1);
    let after = coordinator.stats();
    assert_eq!(after.success_cache_entries, 0);
    assert_eq!(after.success_cache_bytes, 0);
    assert_eq!(after.negative_cache_entries, 0);
    coordinator.shutdown();
}

struct RecordingResolver {
    order: Mutex<Vec<i64>>,
    latency: Duration,
}

impl ThumbnailResolver for RecordingResolver {
    fn resolve(
        &self,
        key: &ThumbnailKey,
        _cancellation: &CancellationToken,
    ) -> Result<ResolvedThumbnail, ThumbnailResolveError> {
        let gallery_id = key
            .gallery_id()
            .expect("recording resolver uses gallery keys");
        self.order.lock().unwrap().push(gallery_id);
        thread::sleep(self.latency);
        Ok(ResolvedThumbnail {
            content_type: "image/png".into(),
            bytes: vec![gallery_id as u8],
            width: 1,
            height: 1,
            source_revision: None,
        })
    }
}

#[test]
fn queued_work_is_dispatched_by_priority_then_fifo() {
    let resolver = Arc::new(RecordingResolver {
        order: Mutex::new(Vec::new()),
        latency: Duration::from_millis(30),
    });
    let coordinator = ThumbnailCoordinator::new(resolver.clone(), config(1)).unwrap();
    let first = coordinator
        .request(request(
            ThumbnailKey::gallery_cover(11).unwrap(),
            ThumbnailPriority::Visible,
        ))
        .unwrap();
    while coordinator.stats().active_workers == 0 {
        thread::sleep(Duration::from_millis(1));
    }
    let prefetch = coordinator
        .request(request(
            ThumbnailKey::gallery_cover(12).unwrap(),
            ThumbnailPriority::Prefetch,
        ))
        .unwrap();
    let visible = coordinator
        .request(request(
            ThumbnailKey::gallery_cover(13).unwrap(),
            ThumbnailPriority::Visible,
        ))
        .unwrap();
    let critical = coordinator
        .request(request(
            ThumbnailKey::gallery_cover(14).unwrap(),
            ThumbnailPriority::Critical,
        ))
        .unwrap();
    for handle in [first, prefetch, visible, critical] {
        assert!(handle.recv().is_ok());
    }

    assert_eq!(*resolver.order.lock().unwrap(), vec![11, 14, 13, 12]);
    coordinator.shutdown();
}

#[test]
fn queued_subscription_can_be_reprioritized() {
    let resolver = Arc::new(RecordingResolver {
        order: Mutex::new(Vec::new()),
        latency: Duration::from_millis(25),
    });
    let coordinator = ThumbnailCoordinator::new(resolver.clone(), config(1)).unwrap();
    let blocker = coordinator
        .request(request(
            ThumbnailKey::gallery_cover(21).unwrap(),
            ThumbnailPriority::Visible,
        ))
        .unwrap();
    while coordinator.stats().active_workers == 0 {
        thread::sleep(Duration::from_millis(1));
    }
    let promoted = coordinator
        .request(request(
            ThumbnailKey::gallery_cover(22).unwrap(),
            ThumbnailPriority::Prefetch,
        ))
        .unwrap();
    let visible = coordinator
        .request(request(
            ThumbnailKey::gallery_cover(23).unwrap(),
            ThumbnailPriority::Visible,
        ))
        .unwrap();
    assert!(coordinator.reprioritize(promoted.request_id(), ThumbnailPriority::Critical));
    for handle in [blocker, promoted, visible] {
        assert!(handle.recv().is_ok());
    }
    assert_eq!(*resolver.order.lock().unwrap(), vec![21, 22, 23]);
    coordinator.shutdown();
}

#[test]
fn runtime_settings_are_applied_to_new_work() {
    let coordinator =
        ThumbnailCoordinator::with_resolver(FixtureThumbnailResolver::new(), config(1)).unwrap();
    let applied = coordinator
        .reconfigure(ThumbnailRuntimeConfigDto {
            concurrent_image_requests: 3,
            request_start_interval_ms: 40,
        })
        .unwrap();
    assert_eq!(applied.concurrent_image_requests, 3);
    assert_eq!(applied.request_start_interval_ms, 40);
    let stats = coordinator.stats();
    assert_eq!(stats.concurrency_limit, 3);
    assert_eq!(stats.request_start_interval_ms, 40);
    assert!(stats.worker_count >= 3);
    coordinator.shutdown();
}
