//! Deterministic, bounded network impairment model for protocol and convergence tests.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::time::Duration;

/// Probability rates are expressed as integer parts per 10,000.
pub const RATE_DENOMINATOR: u16 = 10_000;
pub const MAX_ONE_WAY_LATENCY: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OverflowPolicy {
    /// Preserve already scheduled traffic and reject the newly submitted copy.
    DropNewest,
    /// Replace the oldest lower sequence in the same flow when possible.
    ///
    /// This is useful for bounded snapshot-style test links, where retaining a
    /// stale delta is less useful than retaining the newest state.
    KeepLatestPerFlow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImpairmentConfig {
    pub seed: u64,
    pub loss_rate: u16,
    pub duplication_rate: u16,
    pub reordering_rate: u16,
    pub base_latency: Duration,
    pub jitter: Duration,
    pub reordering_window: Duration,
    pub capacity: usize,
    pub overflow_policy: OverflowPolicy,
}

impl Default for ImpairmentConfig {
    fn default() -> Self {
        Self {
            seed: 1,
            loss_rate: 0,
            duplication_rate: 0,
            reordering_rate: 0,
            base_latency: Duration::ZERO,
            jitter: Duration::ZERO,
            reordering_window: Duration::ZERO,
            capacity: 1_024,
            overflow_policy: OverflowPolicy::DropNewest,
        }
    }
}

impl ImpairmentConfig {
    pub fn validate(&self) -> Result<(), ImpairmentConfigError> {
        for (name, rate) in [
            ("loss", self.loss_rate),
            ("duplication", self.duplication_rate),
            ("reordering", self.reordering_rate),
        ] {
            if rate > RATE_DENOMINATOR {
                return Err(ImpairmentConfigError::RateOutOfRange { name, rate });
            }
        }
        if self.capacity == 0 {
            return Err(ImpairmentConfigError::ZeroCapacity);
        }

        let maximum_delay = self
            .base_latency
            .checked_add(self.jitter)
            .and_then(|delay| delay.checked_add(self.reordering_window))
            .ok_or(ImpairmentConfigError::LatencyOutOfRange)?;
        if maximum_delay > MAX_ONE_WAY_LATENCY {
            return Err(ImpairmentConfigError::LatencyOutOfRange);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImpairmentConfigError {
    RateOutOfRange { name: &'static str, rate: u16 },
    ZeroCapacity,
    LatencyOutOfRange,
    ZeroFlowCapacity,
}

impl fmt::Display for ImpairmentConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateOutOfRange { name, rate } => write!(
                formatter,
                "{name} rate {rate} exceeds {RATE_DENOMINATOR}"
            ),
            Self::ZeroCapacity => {
                formatter.write_str("impairment storage capacity must be non-zero")
            }
            Self::LatencyOutOfRange => write!(
                formatter,
                "maximum modeled one-way latency must not exceed {} ms",
                MAX_ONE_WAY_LATENCY.as_millis()
            ),
            Self::ZeroFlowCapacity => {
                formatter.write_str("sequence-filter flow capacity must be non-zero")
            }
        }
    }
}

impl Error for ImpairmentConfigError {}

/// A monotonically sequenced datagram within an independently ordered flow.
///
/// Tests should use an unwrapped sequence value even if the wire protocol uses
/// a narrower wrapping representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SequencedDatagram<T> {
    pub flow_id: u64,
    pub sequence: u64,
    pub payload: T,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ImpairmentStats {
    pub submitted: u64,
    pub lost: u64,
    pub duplicates_created: u64,
    pub capacity_dropped: u64,
    pub obsolete_evicted: u64,
    pub delivered: u64,
    pub maximum_buffered: usize,
}

#[derive(Debug)]
struct ScheduledDatagram<T> {
    deliver_at_ns: u64,
    insertion_order: u64,
    datagram: SequencedDatagram<T>,
}

/// A deterministic network link with strictly bounded in-flight storage.
#[derive(Debug)]
pub struct NetworkImpairment<T> {
    config: ImpairmentConfig,
    random: DeterministicRandom,
    pending: Vec<ScheduledDatagram<T>>,
    next_insertion_order: u64,
    stats: ImpairmentStats,
}

impl<T: Clone> NetworkImpairment<T> {
    pub fn new(config: ImpairmentConfig) -> Result<Self, ImpairmentConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            random: DeterministicRandom::new(config.seed),
            pending: Vec::with_capacity(config.capacity),
            next_insertion_order: 0,
            stats: ImpairmentStats::default(),
        })
    }

    /// Submits one datagram at a caller-controlled monotonic timestamp.
    ///
    /// Loss is evaluated once for the original. If it survives, duplication is
    /// evaluated once and each copy receives its own deterministic delay.
    pub fn send(&mut self, sent_at_ns: u64, datagram: SequencedDatagram<T>) {
        self.stats.submitted = self.stats.submitted.saturating_add(1);
        if self.random.chance(self.config.loss_rate) {
            self.stats.lost = self.stats.lost.saturating_add(1);
            return;
        }

        self.schedule_copy(sent_at_ns, datagram.clone());
        if self.random.chance(self.config.duplication_rate) {
            self.stats.duplicates_created =
                self.stats.duplicates_created.saturating_add(1);
            self.schedule_copy(sent_at_ns, datagram);
        }
    }

    /// Removes and returns every datagram whose modeled delivery time has arrived.
    ///
    /// Results are ordered by delivery timestamp and then stable insertion order.
    /// The returned vector can never contain more than `capacity` items.
    pub fn receive_ready(&mut self, now_ns: u64) -> Vec<SequencedDatagram<T>> {
        let mut ready = Vec::new();
        let mut index = 0;
        while index < self.pending.len() {
            if self.pending[index].deliver_at_ns <= now_ns {
                ready.push(self.pending.swap_remove(index));
            } else {
                index += 1;
            }
        }
        ready.sort_unstable_by_key(|scheduled| {
            (scheduled.deliver_at_ns, scheduled.insertion_order)
        });
        self.stats.delivered = self
            .stats
            .delivered
            .saturating_add(ready.len() as u64);
        ready
            .into_iter()
            .map(|scheduled| scheduled.datagram)
            .collect()
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub const fn capacity(&self) -> usize {
        self.config.capacity
    }

    pub const fn stats(&self) -> ImpairmentStats {
        self.stats
    }

    fn schedule_copy(&mut self, sent_at_ns: u64, datagram: SequencedDatagram<T>) {
        if self.pending.len() == self.config.capacity
            && !self.make_capacity_for(&datagram)
        {
            self.stats.capacity_dropped =
                self.stats.capacity_dropped.saturating_add(1);
            return;
        }

        let delay_ns = self.sample_delay_ns();
        self.pending.push(ScheduledDatagram {
            deliver_at_ns: sent_at_ns.saturating_add(delay_ns),
            insertion_order: self.next_insertion_order,
            datagram,
        });
        self.next_insertion_order = self.next_insertion_order.saturating_add(1);
        self.stats.maximum_buffered =
            self.stats.maximum_buffered.max(self.pending.len());
    }

    fn make_capacity_for(&mut self, incoming: &SequencedDatagram<T>) -> bool {
        if self.config.overflow_policy == OverflowPolicy::DropNewest {
            return false;
        }

        let replace = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, scheduled)| {
                scheduled.datagram.flow_id == incoming.flow_id
                    && scheduled.datagram.sequence < incoming.sequence
            })
            .min_by_key(|(_, scheduled)| scheduled.datagram.sequence)
            .map(|(index, _)| index);
        if let Some(index) = replace {
            self.pending.swap_remove(index);
            self.stats.obsolete_evicted =
                self.stats.obsolete_evicted.saturating_add(1);
            true
        } else {
            false
        }
    }

    fn sample_delay_ns(&mut self) -> u64 {
        let base_ns = duration_ns(self.config.base_latency);
        let jitter_ns = duration_ns(self.config.jitter);
        let jittered = if jitter_ns == 0 {
            base_ns
        } else {
            let span = jitter_ns.saturating_mul(2).saturating_add(1);
            let sample = self.random.below(span);
            if sample <= jitter_ns {
                base_ns.saturating_sub(jitter_ns - sample)
            } else {
                base_ns.saturating_add(sample - jitter_ns)
            }
        };

        if self.random.chance(self.config.reordering_rate) {
            jittered.saturating_add(
                self.random
                    .below(duration_ns(self.config.reordering_window).saturating_add(1)),
            )
        } else {
            jittered
        }
    }
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SequenceDecision {
    Accepted,
    Obsolete,
    FlowCapacityExceeded,
}

/// Bounded per-flow latest-sequence filter for snapshot-like datagrams.
#[derive(Debug)]
pub struct LatestSequenceFilter {
    latest: BTreeMap<u64, u64>,
    capacity: usize,
    obsolete_dropped: u64,
}

impl LatestSequenceFilter {
    pub fn new(capacity: usize) -> Result<Self, ImpairmentConfigError> {
        if capacity == 0 {
            return Err(ImpairmentConfigError::ZeroFlowCapacity);
        }
        Ok(Self {
            latest: BTreeMap::new(),
            capacity,
            obsolete_dropped: 0,
        })
    }

    pub fn inspect<T>(&mut self, datagram: &SequencedDatagram<T>) -> SequenceDecision {
        if let Some(latest) = self.latest.get_mut(&datagram.flow_id) {
            if datagram.sequence <= *latest {
                self.obsolete_dropped = self.obsolete_dropped.saturating_add(1);
                return SequenceDecision::Obsolete;
            } else {
                *latest = datagram.sequence;
                return SequenceDecision::Accepted;
            }
        }

        if self.latest.len() == self.capacity {
            SequenceDecision::FlowCapacityExceeded
        } else {
            self.latest.insert(datagram.flow_id, datagram.sequence);
            SequenceDecision::Accepted
        }
    }

    pub fn latest_sequence(&self, flow_id: u64) -> Option<u64> {
        self.latest.get(&flow_id).copied()
    }

    pub fn tracked_flows(&self) -> usize {
        self.latest.len()
    }

    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    pub const fn obsolete_dropped(&self) -> u64 {
        self.obsolete_dropped
    }
}

#[derive(Debug)]
struct DeterministicRandom {
    state: u64,
}

impl DeterministicRandom {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9e37_79b9_7f4a_7c15
            } else {
                seed
            },
        }
    }

    fn next(&mut self) -> u64 {
        let mut value = self.state;
        value ^= value >> 12;
        value ^= value << 25;
        value ^= value >> 27;
        self.state = value;
        value.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn chance(&mut self, rate: u16) -> bool {
        rate == RATE_DENOMINATOR
            || (rate != 0 && self.below(RATE_DENOMINATOR as u64) < rate as u64)
    }

    fn below(&mut self, upper_exclusive: u64) -> u64 {
        if upper_exclusive <= 1 {
            0
        } else {
            self.next() % upper_exclusive
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn impaired_config() -> ImpairmentConfig {
        ImpairmentConfig {
            seed: 0x5eed,
            loss_rate: 0,
            duplication_rate: 5_000,
            reordering_rate: RATE_DENOMINATOR,
            base_latency: Duration::from_millis(20),
            jitter: Duration::from_millis(10),
            reordering_window: Duration::from_millis(100),
            capacity: 4,
            overflow_policy: OverflowPolicy::KeepLatestPerFlow,
        }
    }

    #[test]
    fn configuration_caps_all_modeled_latency_at_250_ms() {
        let mut config = ImpairmentConfig {
            base_latency: Duration::from_millis(250),
            ..ImpairmentConfig::default()
        };
        config.validate().expect("250 ms is permitted");

        config.jitter = Duration::from_nanos(1);
        assert_eq!(
            config.validate(),
            Err(ImpairmentConfigError::LatencyOutOfRange)
        );
        config.jitter = Duration::ZERO;
        config.loss_rate = RATE_DENOMINATOR + 1;
        assert_eq!(
            config.validate(),
            Err(ImpairmentConfigError::RateOutOfRange {
                name: "loss",
                rate: RATE_DENOMINATOR + 1,
            })
        );
    }

    #[test]
    fn identical_seed_and_input_produce_identical_delivery() {
        let config = impaired_config();
        let mut first = NetworkImpairment::new(config).expect("config");
        let mut second = NetworkImpairment::new(config).expect("config");
        for sequence in 1..=12 {
            let datagram = SequencedDatagram {
                flow_id: 7,
                sequence,
                payload: sequence,
            };
            first.send(sequence * 1_000, datagram.clone());
            second.send(sequence * 1_000, datagram);
        }

        assert_eq!(
            first.receive_ready(250_000_000),
            second.receive_ready(250_000_000)
        );
        assert_eq!(first.stats(), second.stats());
    }

    #[test]
    fn loss_and_duplication_are_configurable() {
        let mut lost = NetworkImpairment::new(ImpairmentConfig {
            loss_rate: RATE_DENOMINATOR,
            capacity: 1,
            ..ImpairmentConfig::default()
        })
        .expect("loss config");
        lost.send(
            0,
            SequencedDatagram {
                flow_id: 1,
                sequence: 1,
                payload: (),
            },
        );
        assert_eq!(lost.stats().lost, 1);
        assert_eq!(lost.pending_len(), 0);

        let mut duplicated = NetworkImpairment::new(ImpairmentConfig {
            duplication_rate: RATE_DENOMINATOR,
            capacity: 2,
            ..ImpairmentConfig::default()
        })
        .expect("duplication config");
        duplicated.send(
            0,
            SequencedDatagram {
                flow_id: 1,
                sequence: 1,
                payload: (),
            },
        );
        assert_eq!(duplicated.receive_ready(0).len(), 2);
        assert_eq!(duplicated.stats().duplicates_created, 1);
    }

    #[test]
    fn latest_snapshot_converges_with_reordering_and_bounded_storage() {
        let mut reordered_run = None;
        for seed in 1..=256 {
            let mut config = impaired_config();
            config.seed = seed;
            let mut link = NetworkImpairment::new(config).expect("config");
            for sequence in 1..=32 {
                link.send(
                    sequence * 100_000,
                    SequencedDatagram {
                        flow_id: 9,
                        sequence,
                        payload: sequence,
                    },
                );
                assert!(link.pending_len() <= link.capacity());
            }

            let delivered = link.receive_ready(250_000_000);
            let delivered_order: Vec<_> =
                delivered.iter().map(|datagram| datagram.sequence).collect();
            let mut ordered = delivered_order.clone();
            ordered.sort_unstable();
            if delivered_order != ordered {
                reordered_run = Some((link, delivered));
                break;
            }
        }
        let (link, delivered) =
            reordered_run.expect("at least one deterministic seed must reorder traffic");

        let mut receiver = LatestSequenceFilter::new(1).expect("one flow");
        for datagram in &delivered {
            receiver.inspect(datagram);
        }
        assert_eq!(receiver.latest_sequence(9), Some(32));
        assert!(
            receiver.obsolete_dropped() > 0,
            "older snapshots delivered after a newer one must be discarded"
        );
        assert_eq!(link.stats().maximum_buffered, 4);
        assert!(link.stats().obsolete_evicted > 0);
        assert_eq!(link.pending_len(), 0);
    }

    #[test]
    fn sequence_filter_has_bounded_flow_storage() {
        let mut filter = LatestSequenceFilter::new(1).expect("capacity");
        assert_eq!(
            filter.inspect(&SequencedDatagram {
                flow_id: 1,
                sequence: 1,
                payload: (),
            }),
            SequenceDecision::Accepted
        );
        assert_eq!(
            filter.inspect(&SequencedDatagram {
                flow_id: 2,
                sequence: 1,
                payload: (),
            }),
            SequenceDecision::FlowCapacityExceeded
        );
        assert_eq!(filter.tracked_flows(), 1);
    }
}
