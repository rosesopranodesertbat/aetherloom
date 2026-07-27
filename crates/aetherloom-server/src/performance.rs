//! Reusable 128-player scheduling and simulation performance gate.

use crate::scheduler::{
    MonotonicClock, SchedulerError, TickId, TickScheduler, AUTHORITATIVE_HZ,
    TICK_PERIOD_NS,
};
use std::error::Error;
use std::fmt;
use std::time::Duration;

pub const PERFORMANCE_GATE_PLAYERS: usize = 128;
pub const PRODUCTION_SOAK_DURATION: Duration = Duration::from_secs(60 * 60);
pub const MAX_SOAK_DURATION: Duration = PRODUCTION_SOAK_DURATION;
pub const MAX_SIMULATION_P99: Duration = Duration::from_millis(4);
pub const MAX_START_JITTER_P99: Duration = Duration::from_micros(500);
pub const MAX_MEDIAN_EGRESS_BYTES_PER_SECOND: u64 = 96 * 1024;
pub const MAX_P95_EGRESS_BYTES_PER_SECOND: u64 = 192 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PerformanceGateConfig {
    pub duration: Duration,
    pub ingress_queue_capacity: usize,
    pub egress_queue_capacity: usize,
    pub maximum_simulation_p99: Duration,
    pub maximum_start_jitter_p99: Duration,
    pub maximum_median_egress_bytes_per_second: u64,
    pub maximum_p95_egress_bytes_per_second: u64,
}

impl PerformanceGateConfig {
    /// Production acceptance profile: 128 players for 60 minutes.
    pub const fn production_60_minutes() -> Self {
        Self {
            duration: PRODUCTION_SOAK_DURATION,
            ingress_queue_capacity: 4_096,
            egress_queue_capacity: 4_096,
            maximum_simulation_p99: MAX_SIMULATION_P99,
            maximum_start_jitter_p99: MAX_START_JITTER_P99,
            maximum_median_egress_bytes_per_second:
                MAX_MEDIAN_EGRESS_BYTES_PER_SECOND,
            maximum_p95_egress_bytes_per_second: MAX_P95_EGRESS_BYTES_PER_SECOND,
        }
    }

    /// Creates the same gate with a shorter, exact-tick duration for CI.
    pub const fn ci(duration: Duration) -> Self {
        Self {
            duration,
            ..Self::production_60_minutes()
        }
    }

    pub fn expected_ticks(&self) -> Result<u64, PerformanceGateConfigError> {
        self.validate()?;
        let duration_ns = self.duration.as_nanos();
        u64::try_from(duration_ns / TICK_PERIOD_NS as u128)
            .map_err(|_| PerformanceGateConfigError::DurationTooLong)
    }

    pub fn validate(&self) -> Result<(), PerformanceGateConfigError> {
        if self.duration.is_zero() {
            return Err(PerformanceGateConfigError::ZeroDuration);
        }
        if self.duration > MAX_SOAK_DURATION {
            return Err(PerformanceGateConfigError::DurationTooLong);
        }
        if self.duration.as_nanos() % TICK_PERIOD_NS as u128 != 0 {
            return Err(PerformanceGateConfigError::PartialTickDuration);
        }
        if self.ingress_queue_capacity == 0 || self.egress_queue_capacity == 0 {
            return Err(PerformanceGateConfigError::ZeroQueueCapacity);
        }
        if self.maximum_simulation_p99.is_zero()
            || self.maximum_start_jitter_p99.is_zero()
            || self.maximum_median_egress_bytes_per_second == 0
            || self.maximum_p95_egress_bytes_per_second == 0
        {
            return Err(PerformanceGateConfigError::ZeroThreshold);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PerformanceGateConfigError {
    ZeroDuration,
    DurationTooLong,
    PartialTickDuration,
    ZeroQueueCapacity,
    ZeroThreshold,
}

impl fmt::Display for PerformanceGateConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDuration => formatter.write_str("soak duration must be non-zero"),
            Self::DurationTooLong => write!(
                formatter,
                "soak duration exceeds the {} minute bounded production window",
                MAX_SOAK_DURATION.as_secs() / 60
            ),
            Self::PartialTickDuration => {
                formatter.write_str("soak duration must contain an exact number of 128 Hz ticks")
            }
            Self::ZeroQueueCapacity => {
                formatter.write_str("performance-gate queue capacities must be non-zero")
            }
            Self::ZeroThreshold => {
                formatter.write_str("performance thresholds must be non-zero")
            }
        }
    }
}

impl Error for PerformanceGateConfigError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueueLoadSample {
    pub ingress_depth: usize,
    pub egress_depth: usize,
    /// New rejected ingress submissions since the preceding tick.
    pub ingress_overflow_events: u64,
    /// New rejected egress submissions since the preceding tick.
    pub egress_overflow_events: u64,
    /// Bytes submitted for each stable player slot during this tick. A real
    /// soak adapter fills this after prioritization, before socket overhead.
    pub egress_bytes_per_client: [u32; PERFORMANCE_GATE_PLAYERS],
}

impl Default for QueueLoadSample {
    fn default() -> Self {
        Self {
            ingress_depth: 0,
            egress_depth: 0,
            ingress_overflow_events: 0,
            egress_overflow_events: 0,
            egress_bytes_per_client: [0; PERFORMANCE_GATE_PLAYERS],
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SoakTickContext {
    pub tick: TickId,
    pub player_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GateViolation {
    TickCount {
        expected: u64,
        actual: u64,
    },
    DroppedOrRepeatedTicks {
        count: u64,
    },
    TimeDilation {
        count: u64,
    },
    SimulationP99 {
        actual_ns: u64,
        required_below_ns: u64,
    },
    StartJitterP99 {
        actual_ns: u64,
        required_below_ns: u64,
    },
    IngressDepth {
        maximum: usize,
        capacity: usize,
    },
    EgressDepth {
        maximum: usize,
        capacity: usize,
    },
    QueueOverflow {
        ingress_events: u64,
        egress_events: u64,
    },
    MedianEgress {
        actual_bytes_per_second: u64,
        required_below_bytes_per_second: u64,
    },
    P95Egress {
        actual_bytes_per_second: u64,
        required_below_bytes_per_second: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerformanceGateReport {
    pub player_count: usize,
    pub configured_duration: Duration,
    pub expected_ticks: u64,
    pub executed_ticks: u64,
    pub authoritative_hz: u32,
    pub simulation_p99_ns: u64,
    pub start_jitter_p99_ns: u64,
    pub dropped_or_repeated_ticks: u64,
    pub time_dilation_events: u64,
    pub maximum_ingress_depth: usize,
    pub maximum_egress_depth: usize,
    pub ingress_overflow_events: u64,
    pub egress_overflow_events: u64,
    pub median_egress_bytes_per_second: u64,
    pub p95_egress_bytes_per_second: u64,
    pub violations: Vec<GateViolation>,
}

impl PerformanceGateReport {
    pub fn passed(&self) -> bool {
        self.violations.is_empty()
    }
}

#[derive(Debug)]
pub enum PerformanceGateRunError<E> {
    Configuration(PerformanceGateConfigError),
    Scheduler(SchedulerError),
    Workload { tick: TickId, source: E },
}

impl<E: fmt::Display> fmt::Display for PerformanceGateRunError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(error) => error.fmt(formatter),
            Self::Scheduler(error) => error.fmt(formatter),
            Self::Workload { tick, source } => {
                write!(formatter, "workload failed at tick {}: {source}", tick.0)
            }
        }
    }
}

impl<E: Error + 'static> Error for PerformanceGateRunError<E> {}

/// Runs the 128-player gate against a caller-supplied clock and workload.
///
/// The workload callback advances or consumes the supplied clock while it runs
/// one authoritative tick and returns current bounded-queue telemetry. A native
/// soak passes `SystemClock`; deterministic CI passes a fake clock.
pub fn run_performance_gate<C, F, E>(
    config: PerformanceGateConfig,
    clock: C,
    mut workload: F,
) -> Result<PerformanceGateReport, PerformanceGateRunError<E>>
where
    C: MonotonicClock,
    F: FnMut(SoakTickContext, &mut C) -> Result<QueueLoadSample, E>,
{
    let expected_ticks = config
        .expected_ticks()
        .map_err(PerformanceGateRunError::Configuration)?;
    let metrics_capacity = usize::try_from(expected_ticks)
        .map_err(|_| {
            PerformanceGateRunError::Configuration(
                PerformanceGateConfigError::DurationTooLong,
            )
        })?;
    let mut scheduler = TickScheduler::new(clock, metrics_capacity);
    let epoch_ns = scheduler.epoch_ns();
    let mut dropped_or_repeated_ticks = 0_u64;
    let mut time_dilation_events = 0_u64;
    let mut maximum_ingress_depth = 0;
    let mut maximum_egress_depth = 0;
    let mut ingress_overflow_events = 0_u64;
    let mut egress_overflow_events = 0_u64;
    let mut egress_bytes_per_client = [0_u64; PERFORMANCE_GATE_PLAYERS];

    for expected_tick in 0..expected_ticks {
        let permit = scheduler
            .wait_for_tick()
            .map_err(PerformanceGateRunError::Scheduler)?;
        if permit.tick().0 != expected_tick {
            dropped_or_repeated_ticks =
                dropped_or_repeated_ticks.saturating_add(1);
        }
        let expected_deadline =
            epoch_ns.saturating_add(expected_tick.saturating_mul(TICK_PERIOD_NS));
        if permit.scheduled_start_ns() != expected_deadline {
            time_dilation_events = time_dilation_events.saturating_add(1);
        }

        let load = workload(
            SoakTickContext {
                tick: permit.tick(),
                player_count: PERFORMANCE_GATE_PLAYERS,
            },
            scheduler.clock_mut(),
        )
        .map_err(|source| PerformanceGateRunError::Workload {
            tick: permit.tick(),
            source,
        })?;
        maximum_ingress_depth = maximum_ingress_depth.max(load.ingress_depth);
        maximum_egress_depth = maximum_egress_depth.max(load.egress_depth);
        ingress_overflow_events = ingress_overflow_events
            .saturating_add(load.ingress_overflow_events);
        egress_overflow_events = egress_overflow_events
            .saturating_add(load.egress_overflow_events);
        for (total, bytes) in egress_bytes_per_client
            .iter_mut()
            .zip(load.egress_bytes_per_client)
        {
            *total = total.saturating_add(bytes as u64);
        }
        scheduler
            .complete_tick(permit)
            .map_err(PerformanceGateRunError::Scheduler)?;
    }

    let metrics = scheduler
        .metrics()
        .snapshot()
        .expect("validated duration always executes at least one tick");
    let simulation_p99_ns = metrics.p99_duration_ns;
    let start_jitter_p99_ns = metrics.p99_start_jitter_ns.max(0) as u64;
    let executed_ticks = metrics.total_ticks;
    let maximum_simulation_ns = duration_ns(config.maximum_simulation_p99);
    let maximum_jitter_ns = duration_ns(config.maximum_start_jitter_p99);
    let duration_ns = duration_ns(config.duration);
    let mut egress_rates = egress_bytes_per_client
        .map(|bytes| bytes_per_second(bytes, duration_ns));
    egress_rates.sort_unstable();
    let median_egress_bytes_per_second =
        percentile_value(&egress_rates, 50);
    let p95_egress_bytes_per_second = percentile_value(&egress_rates, 95);
    let mut violations = Vec::new();

    if executed_ticks != expected_ticks {
        violations.push(GateViolation::TickCount {
            expected: expected_ticks,
            actual: executed_ticks,
        });
    }
    if dropped_or_repeated_ticks != 0 {
        violations.push(GateViolation::DroppedOrRepeatedTicks {
            count: dropped_or_repeated_ticks,
        });
    }
    if time_dilation_events != 0 {
        violations.push(GateViolation::TimeDilation {
            count: time_dilation_events,
        });
    }
    if simulation_p99_ns >= maximum_simulation_ns {
        violations.push(GateViolation::SimulationP99 {
            actual_ns: simulation_p99_ns,
            required_below_ns: maximum_simulation_ns,
        });
    }
    if start_jitter_p99_ns >= maximum_jitter_ns {
        violations.push(GateViolation::StartJitterP99 {
            actual_ns: start_jitter_p99_ns,
            required_below_ns: maximum_jitter_ns,
        });
    }
    if maximum_ingress_depth > config.ingress_queue_capacity {
        violations.push(GateViolation::IngressDepth {
            maximum: maximum_ingress_depth,
            capacity: config.ingress_queue_capacity,
        });
    }
    if maximum_egress_depth > config.egress_queue_capacity {
        violations.push(GateViolation::EgressDepth {
            maximum: maximum_egress_depth,
            capacity: config.egress_queue_capacity,
        });
    }
    if ingress_overflow_events != 0 || egress_overflow_events != 0 {
        violations.push(GateViolation::QueueOverflow {
            ingress_events: ingress_overflow_events,
            egress_events: egress_overflow_events,
        });
    }
    if median_egress_bytes_per_second
        >= config.maximum_median_egress_bytes_per_second
    {
        violations.push(GateViolation::MedianEgress {
            actual_bytes_per_second: median_egress_bytes_per_second,
            required_below_bytes_per_second:
                config.maximum_median_egress_bytes_per_second,
        });
    }
    if p95_egress_bytes_per_second >= config.maximum_p95_egress_bytes_per_second
    {
        violations.push(GateViolation::P95Egress {
            actual_bytes_per_second: p95_egress_bytes_per_second,
            required_below_bytes_per_second:
                config.maximum_p95_egress_bytes_per_second,
        });
    }

    Ok(PerformanceGateReport {
        player_count: PERFORMANCE_GATE_PLAYERS,
        configured_duration: config.duration,
        expected_ticks,
        executed_ticks,
        authoritative_hz: AUTHORITATIVE_HZ,
        simulation_p99_ns,
        start_jitter_p99_ns,
        dropped_or_repeated_ticks,
        time_dilation_events,
        maximum_ingress_depth,
        maximum_egress_depth,
        ingress_overflow_events,
        egress_overflow_events,
        median_egress_bytes_per_second,
        p95_egress_bytes_per_second,
        violations,
    })
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn bytes_per_second(bytes: u64, duration_ns: u64) -> u64 {
    let rate = (bytes as u128)
        .saturating_mul(1_000_000_000)
        .checked_div(duration_ns as u128)
        .unwrap_or(u128::MAX);
    u64::try_from(rate).unwrap_or(u64::MAX)
}

fn percentile_value(sorted: &[u64], percentile: usize) -> u64 {
    let rank = percentile
        .saturating_mul(sorted.len())
        .saturating_add(99)
        / 100;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::convert::Infallible;

    #[derive(Debug)]
    struct FakeClock {
        now_ns: u64,
        sleep_overshoot_ns: u64,
    }

    impl FakeClock {
        fn new(sleep_overshoot_ns: u64) -> Self {
            Self {
                now_ns: 0,
                sleep_overshoot_ns,
            }
        }

        fn advance(&mut self, nanoseconds: u64) {
            self.now_ns = self.now_ns.saturating_add(nanoseconds);
        }
    }

    impl MonotonicClock for FakeClock {
        fn now_ns(&self) -> u64 {
            self.now_ns
        }

        fn sleep_until_ns(&mut self, deadline_ns: u64) {
            self.now_ns = self
                .now_ns
                .max(deadline_ns.saturating_add(self.sleep_overshoot_ns));
        }
    }

    #[test]
    fn production_preset_is_a_bounded_128_player_sixty_minute_gate() {
        let config = PerformanceGateConfig::production_60_minutes();
        assert_eq!(config.duration, Duration::from_secs(3_600));
        assert_eq!(
            config.expected_ticks().expect("production config"),
            460_800
        );
        assert_eq!(PERFORMANCE_GATE_PLAYERS, 128);
        assert_eq!(MAX_SIMULATION_P99, Duration::from_millis(4));
        assert_eq!(MAX_START_JITTER_P99, Duration::from_micros(500));
    }

    #[test]
    fn ci_gate_passes_exact_128_hz_workload_with_headroom() {
        let config = PerformanceGateConfig::ci(Duration::from_millis(125));
        let report = run_performance_gate(
            config,
            FakeClock::new(100_000),
            |context, clock| -> Result<_, Infallible> {
                assert_eq!(context.player_count, 128);
                clock.advance(2_000_000);
                Ok(QueueLoadSample {
                    ingress_depth: 64,
                    egress_depth: 128,
                    egress_bytes_per_client: [500; PERFORMANCE_GATE_PLAYERS],
                    ..QueueLoadSample::default()
                })
            },
        )
        .expect("gate runs");

        assert!(report.passed(), "{:?}", report.violations);
        assert_eq!(report.expected_ticks, 16);
        assert_eq!(report.executed_ticks, 16);
        assert_eq!(report.authoritative_hz, 128);
        assert_eq!(report.simulation_p99_ns, 2_000_000);
        assert_eq!(report.start_jitter_p99_ns, 100_000);
        assert_eq!(report.dropped_or_repeated_ticks, 0);
        assert_eq!(report.time_dilation_events, 0);
        assert_eq!(report.median_egress_bytes_per_second, 64_000);
        assert_eq!(report.p95_egress_bytes_per_second, 64_000);
    }

    #[test]
    fn strict_latency_thresholds_and_queue_bounds_fail_the_gate() {
        let mut config = PerformanceGateConfig::ci(Duration::from_millis(62) + Duration::from_micros(500));
        config.ingress_queue_capacity = 32;
        config.egress_queue_capacity = 32;
        let report = run_performance_gate(
            config,
            FakeClock::new(600_000),
            |_context, clock| -> Result<_, Infallible> {
                clock.advance(4_000_000);
                Ok(QueueLoadSample {
                    ingress_depth: 33,
                    egress_depth: 34,
                    ingress_overflow_events: 1,
                    egress_overflow_events: 2,
                    egress_bytes_per_client: [2_000; PERFORMANCE_GATE_PLAYERS],
                })
            },
        )
        .expect("gate runs");

        assert!(!report.passed());
        assert!(report
            .violations
            .iter()
            .any(|violation| matches!(violation, GateViolation::SimulationP99 { .. })));
        assert!(report
            .violations
            .iter()
            .any(|violation| matches!(violation, GateViolation::StartJitterP99 { .. })));
        assert!(report
            .violations
            .iter()
            .any(|violation| matches!(violation, GateViolation::QueueOverflow { .. })));
        assert!(report
            .violations
            .iter()
            .any(|violation| matches!(violation, GateViolation::IngressDepth { .. })));
        assert!(report
            .violations
            .iter()
            .any(|violation| matches!(violation, GateViolation::MedianEgress { .. })));
        assert!(report
            .violations
            .iter()
            .any(|violation| matches!(violation, GateViolation::P95Egress { .. })));
    }

    #[test]
    fn configuration_rejects_partial_ticks_and_unbounded_durations() {
        assert_eq!(
            PerformanceGateConfig::ci(Duration::from_millis(1)).validate(),
            Err(PerformanceGateConfigError::PartialTickDuration)
        );
        assert_eq!(
            PerformanceGateConfig::ci(Duration::from_secs(3_601)).validate(),
            Err(PerformanceGateConfigError::DurationTooLong)
        );
    }

    #[test]
    fn egress_rate_conversion_does_not_saturate_before_division() {
        let mut config =
            PerformanceGateConfig::ci(Duration::from_millis(62) + Duration::from_micros(500));
        config.maximum_median_egress_bytes_per_second = u64::MAX;
        config.maximum_p95_egress_bytes_per_second = u64::MAX;
        let report = run_performance_gate(
            config,
            FakeClock::new(0),
            |_context, clock| -> Result<_, Infallible> {
                clock.advance(1);
                Ok(QueueLoadSample {
                    egress_bytes_per_client: [u32::MAX; PERFORMANCE_GATE_PLAYERS],
                    ..QueueLoadSample::default()
                })
            },
        )
        .expect("gate runs");
        let expected = u64::from(u32::MAX) * u64::from(AUTHORITATIVE_HZ);

        assert_eq!(report.median_egress_bytes_per_second, expected);
        assert_eq!(report.p95_egress_bytes_per_second, expected);
    }
}
