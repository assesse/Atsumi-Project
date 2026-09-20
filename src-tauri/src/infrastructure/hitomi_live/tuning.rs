//! Learning from ordinary completed downloads only; never generates traffic.
use std::time::{Duration, Instant};

use crate::application::DownloadTuningProfile;

// Reserved local profile key; not a request destination or an actual host cooldown.
pub(super) const PROFILE_HOST: &str = "download-profile.hitomi.la";
const ALGORITHM_VERSION: u32 = 1;
const PROFILE_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const WINDOW: Duration = Duration::from_secs(30);
const MAX_SAMPLE_WINDOW: Duration = Duration::from_secs(10 * 60);
// A slow connection must be able to recover without first sustaining 30 pages
// per minute. Accumulate sparse samples instead of discarding them every window.
const MIN_SUCCESSES: u64 = 8;
const MIN_BYTES: u64 = 2 * 1024 * 1024;
const RETEST_DELAY: Duration = Duration::from_secs(2 * 60);
const IDLE_RESET: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct Delay {
    started: Instant,
    duration: Duration,
}

impl Delay {
    fn remaining(&self, now: Instant) -> Option<Duration> {
        let remaining = self
            .duration
            .saturating_sub(now.saturating_duration_since(self.started));
        (!remaining.is_zero()).then_some(remaining)
    }
}

#[derive(Clone, Copy, Debug)]
struct Metrics {
    rate: f64,
    latency: f64,
    average_bytes: f64,
    stored_rate: f64,
}

#[derive(Debug)]
struct ObservationWindow {
    started: Instant,
    requests: u64,
    bytes: u64,
    latency: f64,
    demand: u64,
    pressure: u64,
    stored_start: u64,
    failed: bool,
}

#[derive(Debug)]
pub(super) struct DownloadSample {
    pub bytes: usize,
    pub service_time: Duration,
    pub had_demand: bool,
    pub processing_backlogged: bool,
    pub stored_bytes: u64,
}

#[derive(Debug)]
pub(super) struct DownloadTuner {
    pub automatic: bool,
    pub ceiling: usize,
    pub current: usize,
    stable: usize,
    baseline_rate: f64,
    trial_baseline: Option<Metrics>,
    reference_metrics: Option<Metrics>,
    regression_windows: u8,
    window: Option<ObservationWindow>,
    consecutive_failures: u32,
    cooldown: Option<Delay>,
    blocked: Option<Delay>,
    cooldown_until: u64,
    blocked_until: u64,
    last_saved: u64,
    generation: u64,
    last_sample: Option<Instant>,
}

impl DownloadTuner {
    pub fn new(
        initial: usize,
        ceiling: usize,
        profile: Option<&DownloadTuningProfile>,
        now: Instant,
        wall_ms: u64,
    ) -> Self {
        let ceiling = ceiling.clamp(1, 8);
        let initial = initial.clamp(1, ceiling);
        let mut tuner = Self {
            automatic: true,
            ceiling,
            current: initial,
            stable: initial,
            baseline_rate: 0.0,
            trial_baseline: None,
            reference_metrics: None,
            regression_windows: 0,
            window: None,
            consecutive_failures: 0,
            cooldown: None,
            blocked: None,
            cooldown_until: 0,
            blocked_until: 0,
            last_saved: 0,
            generation: 0,
            last_sample: None,
        };
        if let Some(saved) = profile.filter(|p| p.host == PROFILE_HOST) {
            // Server waits survive algorithm changes and an expired performance profile.
            tuner.cooldown_until = saved.cooldown_until_unix_ms;
            tuner.blocked_until = saved.blocked_until_unix_ms;
            tuner.cooldown = remaining_delay(saved.cooldown_until_unix_ms, wall_ms, now);
            tuner.blocked = remaining_delay(saved.blocked_until_unix_ms, wall_ms, now);
            tuner.last_saved = saved.updated_at_unix_ms;
            if saved.updated_at_unix_ms <= wall_ms
                && saved.algorithm_version == ALGORITHM_VERSION
                && wall_ms - saved.updated_at_unix_ms <= PROFILE_TTL_MS
            {
                tuner.current = saved.stable_limit.clamp(1, ceiling);
                tuner.stable = tuner.current;
                // Re-measure a baseline in the current environment before increasing.
            }
        }
        tuner
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn cooldown_remaining(&self, now: Instant) -> Option<Duration> {
        self.cooldown
            .as_ref()
            .and_then(|delay| delay.remaining(now))
    }

    pub fn observe(
        &mut self,
        sample: DownloadSample,
        request_generation: u64,
        now: Instant,
        wall_ms: u64,
    ) -> Option<DownloadTuningProfile> {
        // Responses launched before a reduction/trial must not contaminate the new window.
        if !self.automatic
            || request_generation != self.generation
            || self.cooldown_remaining(now).is_some()
        {
            return None;
        }
        self.consecutive_failures = 0;
        let idle = self.last_sample.is_some_and(|last| {
            now.saturating_duration_since(last)
                > IDLE_RESET.max(sample.service_time.saturating_add(Duration::from_secs(5)))
        });
        self.last_sample = Some(now);
        if idle {
            self.window = None;
            if self.trial_baseline.take().is_some() {
                self.current = self.stable;
                self.generation = self.generation.wrapping_add(1);
                self.block_for(RETEST_DELAY, now, wall_ms);
                return Some(self.profile(wall_ms));
            }
        }
        let window = self.window.get_or_insert(ObservationWindow {
            started: now,
            requests: 0,
            bytes: 0,
            latency: 0.0,
            demand: 0,
            pressure: 0,
            stored_start: sample.stored_bytes,
            failed: false,
        });
        window.requests += 1;
        window.bytes = window.bytes.saturating_add(sample.bytes as u64);
        window.latency += sample.service_time.as_secs_f64();
        window.demand += u64::from(sample.had_demand);
        window.pressure += u64::from(sample.processing_backlogged);
        let elapsed = now.saturating_duration_since(window.started);
        if elapsed < WINDOW
            || (elapsed < MAX_SAMPLE_WINDOW
                && (window.requests < MIN_SUCCESSES || window.bytes < MIN_BYTES))
        {
            return None;
        }
        let window = self.window.take().expect("observation window exists");
        let healthy = !window.failed
            && window.requests >= MIN_SUCCESSES
            && window.bytes >= MIN_BYTES
            && window.demand * 100 >= window.requests * 60;
        let metrics = Metrics {
            rate: window.bytes as f64 / elapsed.as_secs_f64(),
            latency: window.latency / window.requests as f64,
            average_bytes: window.bytes as f64 / window.requests as f64,
            stored_rate: sample.stored_bytes.saturating_sub(window.stored_start) as f64
                / elapsed.as_secs_f64(),
        };
        if let Some(baseline) = self.trial_baseline.take() {
            let comparable_size =
                (0.5..=2.0).contains(&(metrics.average_bytes / baseline.average_bytes));
            let improved = healthy
                && comparable_size
                && metrics.rate >= baseline.rate * 1.08
                && metrics.latency <= baseline.latency * 1.25
                && baseline.stored_rate > 0.0
                && (window.pressure * 100 > window.requests * 20
                    || metrics.stored_rate >= baseline.stored_rate * 0.95);
            if improved {
                self.stable = self.current;
                self.baseline_rate = metrics.rate;
                self.reference_metrics = Some(metrics);
                // Confirm first; gather another baseline window before trying the next step.
            } else {
                self.current = self.stable;
                self.baseline_rate = baseline.rate;
                self.block_for(RETEST_DELAY, now, wall_ms);
            }
            self.generation = self.generation.wrapping_add(1);
            return Some(self.profile(wall_ms));
        }
        let enough_demand = window.requests >= MIN_SUCCESSES
            && window.bytes >= MIN_BYTES
            && window.demand * 100 >= window.requests * 60;
        let regressed = enough_demand
            && self.reference_metrics.is_some_and(|reference| {
                (0.5..=2.0).contains(&(metrics.average_bytes / reference.average_bytes))
                    && metrics.rate < reference.rate * 0.75
                    && metrics.latency > reference.latency * 1.25
            });
        self.regression_windows = if regressed {
            self.regression_windows.saturating_add(1)
        } else {
            0
        };
        if self.regression_windows >= 2 && self.current > 1 {
            self.current -= 1;
            self.stable = self.current;
            self.reset_measurement();
            self.block_for(RETEST_DELAY, now, wall_ms);
            return Some(self.profile(wall_ms));
        }
        if regressed {
            return None;
        }
        if !healthy || metrics.stored_rate == 0.0 {
            return None;
        }
        self.baseline_rate = metrics.rate;
        self.reference_metrics = Some(metrics);
        if self.current < self.ceiling
            && self
                .blocked
                .as_ref()
                .and_then(|delay| delay.remaining(now))
                .is_none()
        {
            self.trial_baseline = Some(metrics);
            self.current += 1;
            self.generation = self.generation.wrapping_add(1);
        }
        Some(self.profile(wall_ms))
    }

    pub fn server_backpressure(
        &mut self,
        duration: Duration,
        now: Instant,
        wall_ms: u64,
    ) -> DownloadTuningProfile {
        self.stable = (self.current / 2).max(1);
        if self.automatic {
            self.current = self.stable;
        }
        self.reset_measurement();
        if self
            .cooldown_remaining(now)
            .is_none_or(|remaining| remaining < duration)
        {
            self.cooldown = Some(Delay {
                started: now,
                duration,
            });
            self.cooldown_until = deadline_ms(wall_ms, duration);
        }
        self.block_for(RETEST_DELAY.max(duration), now, wall_ms);
        self.profile(wall_ms)
    }

    pub fn failure(
        &mut self,
        congestion: bool,
        now: Instant,
        wall_ms: u64,
    ) -> Option<DownloadTuningProfile> {
        if !self.automatic {
            return None;
        }
        if let Some(window) = self.window.as_mut() {
            window.failed = true;
        }
        if congestion {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        }
        if self.consecutive_failures >= 3 {
            self.current = (self.current / 2).max(1);
            self.stable = self.current;
            self.reset_measurement();
            self.block_for(RETEST_DELAY, now, wall_ms);
            return Some(self.profile(wall_ms));
        }
        if self.trial_baseline.take().is_some() {
            self.current = self.stable;
            self.window = None;
            self.generation = self.generation.wrapping_add(1);
            self.block_for(RETEST_DELAY, now, wall_ms);
            return Some(self.profile(wall_ms));
        }
        None
    }

    fn reset_measurement(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.window = None;
        self.trial_baseline = None;
        self.reference_metrics = None;
        self.regression_windows = 0;
        self.baseline_rate = 0.0;
        self.consecutive_failures = 0;
    }

    fn block_for(&mut self, duration: Duration, now: Instant, wall_ms: u64) {
        if self
            .blocked
            .as_ref()
            .and_then(|delay| delay.remaining(now))
            .is_none_or(|remaining| remaining < duration)
        {
            self.blocked = Some(Delay {
                started: now,
                duration,
            });
            self.blocked_until = deadline_ms(wall_ms, duration);
        }
    }

    fn profile(&mut self, wall_ms: u64) -> DownloadTuningProfile {
        self.last_saved = wall_ms
            .max(self.last_saved.saturating_add(1))
            .min(i64::MAX as u64);
        DownloadTuningProfile {
            host: PROFILE_HOST.into(),
            algorithm_version: ALGORITHM_VERSION,
            stable_limit: self.stable,
            baseline_bytes_per_second: self.baseline_rate,
            updated_at_unix_ms: self.last_saved,
            cooldown_until_unix_ms: self.cooldown_until,
            blocked_until_unix_ms: self.blocked_until,
        }
    }
}

fn deadline_ms(now: u64, duration: Duration) -> u64 {
    now.saturating_add(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .min(i64::MAX as u64)
}

fn remaining_delay(deadline: u64, wall_ms: u64, now: Instant) -> Option<Delay> {
    (deadline > wall_ms).then(|| Delay {
        started: now,
        duration: Duration::from_millis(deadline - wall_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(
        tuner: &mut DownloadTuner,
        start: Instant,
        wall_ms: u64,
        bytes: usize,
        pressure: bool,
        demand: bool,
    ) -> Option<DownloadTuningProfile> {
        let generation = tuner.generation();
        let mut profile = None;
        for i in 0..=15 {
            profile = tuner
                .observe(
                    DownloadSample {
                        bytes,
                        service_time: Duration::from_millis(300),
                        had_demand: demand,
                        processing_backlogged: pressure,
                        stored_bytes: i * bytes as u64,
                    },
                    generation,
                    start + Duration::from_secs(i * 2),
                    wall_ms + i * 2000,
                )
                .or(profile);
        }
        profile
    }

    #[test]
    fn ordinary_downloads_trial_above_five_and_only_persist_confirmed_improvements() {
        let now = Instant::now();
        let mut tuner = DownloadTuner::new(5, 8, None, now, 1_000_000);
        let baseline = window(&mut tuner, now, 1_000_000, 512 * 1024, false, true).unwrap();
        assert_eq!(tuner.current, 6);
        assert_eq!(baseline.stable_limit, 5);
        let confirmed = window(
            &mut tuner,
            now + Duration::from_secs(32),
            1_032_000,
            640 * 1024,
            false,
            true,
        )
        .unwrap();
        assert_eq!(confirmed.stable_limit, 6);
        assert_eq!(tuner.current, 6);
    }

    #[test]
    fn throughput_plateau_reverts_and_does_not_immediately_trial_again() {
        let now = Instant::now();
        let mut tuner = DownloadTuner::new(5, 8, None, now, 1_000_000);
        window(&mut tuner, now, 1_000_000, 512 * 1024, false, true);
        let reverted = window(
            &mut tuner,
            now + Duration::from_secs(32),
            1_032_000,
            512 * 1024,
            false,
            true,
        )
        .unwrap();
        assert_eq!(tuner.current, 5);
        assert_eq!(reverted.stable_limit, 5);
        assert!(reverted.blocked_until_unix_ms > 1_064_000);
        window(
            &mut tuner,
            now + Duration::from_secs(64),
            1_064_000,
            512 * 1024,
            false,
            true,
        );
        assert_eq!(tuner.current, 5);
    }

    #[test]
    fn sparse_demand_and_tiny_samples_never_raise_limit() {
        for (bytes, pressure, demand) in [(512 * 1024, false, false), (1024, false, true)] {
            let now = Instant::now();
            let mut tuner = DownloadTuner::new(5, 8, None, now, 1_000_000);
            assert!(window(&mut tuner, now, 1_000_000, bytes, pressure, demand).is_none());
            assert_eq!(tuner.current, 5);
        }
    }

    #[test]
    fn confirmed_profile_restores_clamps_expires_and_keeps_server_waits() {
        let now = Instant::now();
        let saved = DownloadTuningProfile {
            host: PROFILE_HOST.into(),
            algorithm_version: 1,
            stable_limit: 7,
            baseline_bytes_per_second: 100.0,
            updated_at_unix_ms: 1_000_000,
            cooldown_until_unix_ms: 1_100_000,
            blocked_until_unix_ms: 1_200_000,
        };
        let restored = DownloadTuner::new(5, 8, Some(&saved), now, 1_050_000);
        assert_eq!(restored.current, 7);
        assert_eq!(
            restored.cooldown_remaining(now),
            Some(Duration::from_secs(50))
        );
        assert_eq!(
            DownloadTuner::new(5, 3, Some(&saved), now, 1_050_000).current,
            3
        );
        assert_eq!(
            DownloadTuner::new(5, 8, Some(&saved), now, 1_000_000 + PROFILE_TTL_MS + 1).current,
            5
        );
        assert_eq!(
            DownloadTuner::new(5, 8, Some(&saved), now, 999_999).current,
            5
        );
        assert_eq!(
            DownloadTuner::new(5, 8, Some(&saved), now, 999_999).cooldown_remaining(now),
            Some(Duration::from_millis(100_001))
        );
    }

    #[test]
    fn backpressure_and_repeated_timeouts_reduce_and_persist_without_trial_promotion() {
        let now = Instant::now();
        let mut tuner = DownloadTuner::new(5, 8, None, now, 1_000_000);
        let old_generation = tuner.generation();
        let saved = tuner.server_backpressure(Duration::from_secs(3600), now, 1_000_000);
        assert_eq!(saved.stable_limit, 2);
        assert_eq!(saved.cooldown_until_unix_ms, 4_600_000);
        assert!(tuner
            .observe(
                DownloadSample {
                    bytes: 1_000_000,
                    service_time: Duration::from_secs(1),
                    had_demand: true,
                    processing_backlogged: false,
                    stored_bytes: 0
                },
                old_generation,
                now,
                1_000_000
            )
            .is_none());
        assert!(tuner.failure(true, now, 1_000_001).is_none());
        assert!(tuner.failure(true, now, 1_000_002).is_none());
        assert_eq!(tuner.failure(true, now, 1_000_003).unwrap().stable_limit, 1);
    }

    #[test]
    fn ceiling_and_idle_trial_reset_are_enforced() {
        let now = Instant::now();
        let mut limited = DownloadTuner::new(5, 3, None, now, 1_000_000);
        window(&mut limited, now, 1_000_000, 512 * 1024, false, true);
        assert_eq!(limited.current, 3);
        let mut tuner = DownloadTuner::new(5, 8, None, now, 1_000_000);
        window(&mut tuner, now, 1_000_000, 512 * 1024, false, true);
        let generation = tuner.generation();
        tuner.observe(
            DownloadSample {
                bytes: 1024,
                service_time: Duration::from_secs(1),
                had_demand: true,
                processing_backlogged: false,
                stored_bytes: 0,
            },
            generation,
            now + Duration::from_secs(32),
            1_032_000,
        );
        tuner.observe(
            DownloadSample {
                bytes: 1024,
                service_time: Duration::from_secs(1),
                had_demand: true,
                processing_backlogged: false,
                stored_bytes: 0,
            },
            generation,
            now + Duration::from_secs(70),
            1_070_000,
        );
        assert_eq!(tuner.current, 5);
    }

    #[test]
    fn local_processing_pressure_does_not_reduce_the_network_limit() {
        let now = Instant::now();
        let mut tuner = DownloadTuner::new(5, 5, None, now, 1_000_000);
        window(&mut tuner, now, 1_000_000, 512 * 1024, true, true);
        assert_eq!(tuner.current, 5);
        let saved = window(
            &mut tuner,
            now + Duration::from_secs(32),
            1_032_000,
            512 * 1024,
            true,
            true,
        )
        .unwrap();
        assert_eq!(saved.stable_limit, 5);
        assert_eq!(tuner.current, 5);
    }

    #[test]
    fn low_rate_downloads_can_recover_one_step_after_the_server_wait() {
        let now = Instant::now();
        let mut tuner = DownloadTuner::new(1, 8, None, now, 1_000_000);
        tuner.server_backpressure(Duration::from_secs(60), now, 1_000_000);
        let generation = tuner.generation();
        // 13 successes / two minutes, with local comparison work in progress.
        for i in 0..=12 {
            tuner.observe(
                DownloadSample {
                    bytes: 256 * 1024,
                    service_time: Duration::from_secs(1),
                    had_demand: true,
                    processing_backlogged: true,
                    stored_bytes: i * 256 * 1024,
                },
                generation,
                now + Duration::from_secs(120 + i * 10),
                1_120_000 + i * 10_000,
            );
        }
        assert_eq!(tuner.current, 2);
        assert_eq!(tuner.stable, 1); // Persist only after the trial is confirmed.
    }

    #[test]
    fn disabled_learning_holds_manual_limit_but_keeps_server_wait_and_safe_profile() {
        let now = Instant::now();
        let mut tuner = DownloadTuner::new(5, 8, None, now, 1_000_000);
        tuner.automatic = false;
        assert!(window(&mut tuner, now, 1_000_000, 512 * 1024, false, true).is_none());
        let saved = tuner.server_backpressure(Duration::from_secs(60), now, 1_000_000);
        assert_eq!(tuner.current, 5);
        assert_eq!(saved.stable_limit, 2);
        assert_eq!(tuner.cooldown_remaining(now), Some(Duration::from_secs(60)));
    }

    #[test]
    fn two_slow_windows_compare_against_the_same_fast_reference() {
        let now = Instant::now();
        let mut tuner = DownloadTuner::new(8, 8, None, now, 1_000_000);
        window(&mut tuner, now, 1_000_000, 1024 * 1024, false, true);
        for round in 1..=2 {
            let generation = tuner.generation();
            for i in 0..=15 {
                tuner.observe(
                    DownloadSample {
                        bytes: 512 * 1024,
                        service_time: Duration::from_millis(600),
                        had_demand: true,
                        processing_backlogged: false,
                        stored_bytes: i * 512 * 1024,
                    },
                    generation,
                    now + Duration::from_secs(round * 32 + i * 2),
                    1_000_000 + (round * 32 + i * 2) * 1000,
                );
            }
            assert_eq!(tuner.current, if round == 1 { 8 } else { 7 });
        }
    }
}
