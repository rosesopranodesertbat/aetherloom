//! Absolute-deadline scheduler for the fixed 128 Hz authoritative simulation.

use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::thread;
use std::time::{Duration, Instant};

pub use aetherloom_protocol::AUTHORITATIVE_HZ;
pub const TICK_PERIOD_NS: u64 = aetherloom_protocol::AUTHORITATIVE_TICK_NANOS;
pub const TICK_PERIOD: Duration = Duration::from_nanos(TICK_PERIOD_NS);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TickId(pub u64);

/// Monotonic time source separated from the scheduler for deterministic tests.
pub trait MonotonicClock {
    fn now_ns(&self) -> u64;
    fn sleep_until_ns(&mut self, deadline_ns: u64);
}

/// Production monotonic clock. Its nanoseconds are relative to construction and
/// are never interpreted as wall-clock time.
#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonotonicClock for SystemClock {
    fn now_ns(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    fn sleep_until_ns(&mut self, deadline_ns: u64) {
        loop {
            let now_ns = self.now_ns();
            if now_ns >= deadline_ns {
                return;
            }
            thread::sleep(Duration::from_nanos(deadline_ns - now_ns));
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickPermit {
    tick: TickId,
    scheduled_start_ns: u64,
    actual_start_ns: u64,
    start_jitter_ns: i64,
}

impl TickPermit {
    pub const fn tick(&self) -> TickId {
        self.tick
    }

    pub const fn scheduled_start_ns(&self) -> u64 {
        self.scheduled_start_ns
    }

    pub const fn actual_start_ns(&self) -> u64 {
        self.actual_start_ns
    }

    pub const fn start_jitter_ns(&self) -> i64 {
        self.start_jitter_ns
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickSample {
    pub tick: TickId,
    pub scheduled_start_ns: u64,
    pub actual_start_ns: u64,
    pub start_jitter_ns: i64,
    pub duration_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TickMetricsSnapshot {
    pub retained_samples: usize,
    pub total_ticks: u64,
    pub latest_duration_ns: u64,
    pub latest_start_jitter_ns: i64,
    pub maximum_duration_ns: u64,
    pub maximum_start_jitter_ns: i64,
    pub p50_duration_ns: u64,
    pub p95_duration_ns: u64,
    pub p99_duration_ns: u64,
    pub p50_start_jitter_ns: i64,
    pub p95_start_jitter_ns: i64,
    pub p99_start_jitter_ns: i64,
}

/// A bounded rolling metrics window suitable for exporting to a telemetry
/// adapter without retaining unbounded per-tick history.
#[derive(Debug)]
pub struct TickMetrics {
    capacity: usize,
    total_ticks: u64,
    samples: VecDeque<TickSample>,
}

impl TickMetrics {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "metrics capacity must be greater than zero");
        Self {
            capacity,
            total_ticks: 0,
            samples: VecDeque::with_capacity(capacity),
        }
    }

    pub fn record(&mut self, sample: TickSample) {
        if self.samples.len() == self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
        self.total_ticks = self.total_ticks.saturating_add(1);
    }

    pub fn snapshot(&self) -> Option<TickMetricsSnapshot> {
        let latest = self.samples.back()?;
        let mut durations: Vec<u64> =
            self.samples.iter().map(|sample| sample.duration_ns).collect();
        let mut jitters: Vec<i64> = self
            .samples
            .iter()
            .map(|sample| sample.start_jitter_ns)
            .collect();
        durations.sort_unstable();
        jitters.sort_unstable();

        Some(TickMetricsSnapshot {
            retained_samples: self.samples.len(),
            total_ticks: self.total_ticks,
            latest_duration_ns: latest.duration_ns,
            latest_start_jitter_ns: latest.start_jitter_ns,
            maximum_duration_ns: durations[durations.len() - 1],
            maximum_start_jitter_ns: jitters[jitters.len() - 1],
            p50_duration_ns: percentile(&durations, 50),
            p95_duration_ns: percentile(&durations, 95),
            p99_duration_ns: percentile(&durations, 99),
            p50_start_jitter_ns: percentile(&jitters, 50),
            p95_start_jitter_ns: percentile(&jitters, 95),
            p99_start_jitter_ns: percentile(&jitters, 99),
        })
    }

    pub fn retained_samples(&self) -> impl ExactSizeIterator<Item = &TickSample> {
        self.samples.iter()
    }
}

fn percentile<T: Copy>(sorted: &[T], percentile: usize) -> T {
    let numerator = percentile.saturating_mul(sorted.len().saturating_sub(1));
    let index = numerator.div_ceil(100);
    sorted[index]
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    TickAlreadyInFlight(TickId),
    UnexpectedCompletion {
        expected: Option<TickId>,
        actual: TickId,
    },
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TickAlreadyInFlight(tick) => {
                write!(formatter, "tick {} has not been completed", tick.0)
            }
            Self::UnexpectedCompletion { expected, actual } => write!(
                formatter,
                "completed tick {}; expected {:?}",
                actual.0,
                expected.map(|tick| tick.0)
            ),
        }
    }
}

impl Error for SchedulerError {}

/// Drift-resistant 128 Hz scheduler.
///
/// Every deadline is derived from the fixed epoch (`epoch + tick * period`),
/// never from the previous tick's completion time. A late server catches up by
/// returning subsequent permits immediately; it does not skip simulation ticks
/// or stretch simulation time.
#[derive(Debug)]
pub struct TickScheduler<C> {
    clock: C,
    epoch_ns: u64,
    next_tick: TickId,
    in_flight: Option<TickId>,
    metrics: TickMetrics,
}

impl TickScheduler<SystemClock> {
    pub fn system(metrics_capacity: usize) -> Self {
        Self::new(SystemClock::new(), metrics_capacity)
    }
}

impl<C: MonotonicClock> TickScheduler<C> {
    pub fn new(clock: C, metrics_capacity: usize) -> Self {
        let epoch_ns = clock.now_ns();
        Self {
            clock,
            epoch_ns,
            next_tick: TickId(0),
            in_flight: None,
            metrics: TickMetrics::new(metrics_capacity),
        }
    }

    pub fn wait_for_tick(&mut self) -> Result<TickPermit, SchedulerError> {
        if let Some(tick) = self.in_flight {
            return Err(SchedulerError::TickAlreadyInFlight(tick));
        }

        let tick = self.next_tick;
        let scheduled_start_ns = absolute_deadline(self.epoch_ns, tick);
        while self.clock.now_ns() < scheduled_start_ns {
            self.clock.sleep_until_ns(scheduled_start_ns);
        }
        let actual_start_ns = self.clock.now_ns();
        let start_jitter_ns = signed_delta(actual_start_ns, scheduled_start_ns);

        self.next_tick = TickId(self.next_tick.0.saturating_add(1));
        self.in_flight = Some(tick);
        Ok(TickPermit {
            tick,
            scheduled_start_ns,
            actual_start_ns,
            start_jitter_ns,
        })
    }

    pub fn complete_tick(
        &mut self,
        permit: TickPermit,
    ) -> Result<TickSample, SchedulerError> {
        if self.in_flight != Some(permit.tick) {
            return Err(SchedulerError::UnexpectedCompletion {
                expected: self.in_flight,
                actual: permit.tick,
            });
        }

        let sample = TickSample {
            tick: permit.tick,
            scheduled_start_ns: permit.scheduled_start_ns,
            actual_start_ns: permit.actual_start_ns,
            start_jitter_ns: permit.start_jitter_ns,
            duration_ns: self.clock.now_ns().saturating_sub(permit.actual_start_ns),
        };
        self.in_flight = None;
        self.metrics.record(sample);
        Ok(sample)
    }

    /// Cancels an uncommitted permit so the same authoritative tick can be
    /// retried without advancing simulation time or recording a sample.
    ///
    /// Callers must only use this when authoritative state is unchanged.
    pub fn cancel_tick(&mut self, permit: TickPermit) -> Result<(), SchedulerError> {
        if self.in_flight != Some(permit.tick) {
            return Err(SchedulerError::UnexpectedCompletion {
                expected: self.in_flight,
                actual: permit.tick,
            });
        }
        self.in_flight = None;
        self.next_tick = permit.tick;
        Ok(())
    }

    pub const fn epoch_ns(&self) -> u64 {
        self.epoch_ns
    }

    pub const fn next_tick(&self) -> TickId {
        self.next_tick
    }

    pub fn clock(&self) -> &C {
        &self.clock
    }

    pub fn clock_mut(&mut self) -> &mut C {
        &mut self.clock
    }

    pub fn metrics(&self) -> &TickMetrics {
        &self.metrics
    }
}

fn absolute_deadline(epoch_ns: u64, tick: TickId) -> u64 {
    epoch_ns.saturating_add(tick.0.saturating_mul(TICK_PERIOD_NS))
}

fn signed_delta(actual: u64, scheduled: u64) -> i64 {
    if actual >= scheduled {
        i64::try_from(actual - scheduled).unwrap_or(i64::MAX)
    } else {
        -i64::try_from(scheduled - actual).unwrap_or(i64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FakeClock {
        now_ns: u64,
        sleeps: Vec<u64>,
    }

    impl FakeClock {
        fn new(now_ns: u64) -> Self {
            Self {
                now_ns,
                sleeps: Vec::new(),
            }
        }

        fn advance(&mut self, duration_ns: u64) {
            self.now_ns = self.now_ns.saturating_add(duration_ns);
        }
    }

    impl MonotonicClock for FakeClock {
        fn now_ns(&self) -> u64 {
            self.now_ns
        }

        fn sleep_until_ns(&mut self, deadline_ns: u64) {
            self.sleeps.push(deadline_ns);
            self.now_ns = self.now_ns.max(deadline_ns);
        }
    }

    #[test]
    fn tick_rate_is_exactly_representable_in_nanoseconds() {
        assert_eq!(AUTHORITATIVE_HZ, 128);
        assert_eq!(TICK_PERIOD_NS, 7_812_500);
        assert_eq!(TICK_PERIOD, Duration::from_nanos(7_812_500));
    }

    #[test]
    fn deadlines_are_anchored_to_epoch_not_previous_completion() {
        let mut scheduler = TickScheduler::new(FakeClock::new(1_000), 16);

        let first = scheduler.wait_for_tick().expect("first permit");
        assert_eq!(first.tick(), TickId(0));
        assert_eq!(first.scheduled_start_ns(), 1_000);
        scheduler.clock_mut().advance(2_000_000);
        scheduler.complete_tick(first).expect("complete first");

        let second = scheduler.wait_for_tick().expect("second permit");
        assert_eq!(second.tick(), TickId(1));
        assert_eq!(second.scheduled_start_ns(), 1_000 + TICK_PERIOD_NS);
        assert_eq!(
            scheduler.clock().sleeps,
            vec![1_000 + TICK_PERIOD_NS],
            "completion duration must not shift the next deadline"
        );
    }

    #[test]
    fn late_runtime_never_skips_ticks_or_time_dilates() {
        let mut scheduler = TickScheduler::new(FakeClock::new(0), 16);
        let first = scheduler.wait_for_tick().expect("tick zero");
        scheduler.clock_mut().advance(5 * TICK_PERIOD_NS);
        scheduler.complete_tick(first).expect("complete tick zero");

        for expected in 1..=5 {
            let permit = scheduler.wait_for_tick().expect("catch-up permit");
            assert_eq!(permit.tick(), TickId(expected));
            assert_eq!(
                permit.scheduled_start_ns(),
                expected * TICK_PERIOD_NS
            );
            assert_eq!(
                permit.actual_start_ns(),
                5 * TICK_PERIOD_NS,
                "overdue ticks start immediately"
            );
            scheduler.complete_tick(permit).expect("complete catch-up tick");
        }
    }

    #[test]
    fn scheduler_rejects_overlapping_ticks() {
        let mut scheduler = TickScheduler::new(FakeClock::new(0), 4);
        let permit = scheduler.wait_for_tick().expect("first permit");
        assert_eq!(
            scheduler.wait_for_tick(),
            Err(SchedulerError::TickAlreadyInFlight(TickId(0)))
        );
        scheduler.complete_tick(permit).expect("complete");
    }

    #[test]
    fn cancelled_permit_retries_the_same_tick_without_metrics() {
        let mut scheduler = TickScheduler::new(FakeClock::new(0), 4);
        let permit = scheduler.wait_for_tick().expect("first permit");
        scheduler.cancel_tick(permit).expect("cancel");
        assert_eq!(scheduler.next_tick(), TickId(0));
        assert!(scheduler.metrics().snapshot().is_none());

        let retry = scheduler.wait_for_tick().expect("retry permit");
        assert_eq!(retry.tick(), TickId(0));
        scheduler.complete_tick(retry).expect("complete retry");
        assert_eq!(
            scheduler.metrics().snapshot().expect("metrics").total_ticks,
            1
        );
    }

    #[test]
    fn metrics_window_is_bounded_and_reports_tail_percentiles() {
        let mut metrics = TickMetrics::new(3);
        for tick in 0..4 {
            metrics.record(TickSample {
                tick: TickId(tick),
                scheduled_start_ns: 0,
                actual_start_ns: tick * 10,
                start_jitter_ns: (tick * 10) as i64,
                duration_ns: (tick + 1) * 1_000,
            });
        }

        let snapshot = metrics.snapshot().expect("metrics");
        assert_eq!(snapshot.retained_samples, 3);
        assert_eq!(snapshot.total_ticks, 4);
        assert_eq!(snapshot.latest_duration_ns, 4_000);
        assert_eq!(snapshot.maximum_duration_ns, 4_000);
        assert_eq!(snapshot.p50_duration_ns, 3_000);
        assert_eq!(snapshot.p95_duration_ns, 4_000);
        assert_eq!(
            metrics
                .retained_samples()
                .map(|sample| sample.tick)
                .collect::<Vec<_>>(),
            vec![TickId(1), TickId(2), TickId(3)]
        );
    }
}
