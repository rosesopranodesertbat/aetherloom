use alloc::vec::Vec;

use aetherloom_core::ClientReplica;
use aetherloom_protocol::{
    InputBatch, MessageEnvelope, PlayerCommand, ValidationError,
    AUTHORITATIVE_HZ, COMMAND_REDUNDANCY, MAX_DATAGRAM_BYTES,
};

use crate::IncomingServerMessage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportKind {
    Quic,
    WebSocket,
    Loopback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportPolicy {
    pub kind: TransportKind,
    pub input_hz: u16,
    pub snapshot_hz: u16,
    pub supports_unreliable_datagrams: bool,
    pub max_datagram_bytes: u16,
}

pub trait TransportMarker {
    const POLICY: TransportPolicy;
}

/// Marker for native/console competitive QUIC. A platform networking adapter
/// supplies the actual implementation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuicTransport;

impl TransportMarker for QuicTransport {
    const POLICY: TransportPolicy = TransportPolicy {
        kind: TransportKind::Quic,
        input_hz: 128,
        snapshot_hz: 128,
        supports_unreliable_datagrams: true,
        max_datagram_bytes: MAX_DATAGRAM_BYTES as u16,
    };
}

/// Marker for browser casual binary WSS. Inputs are batched at 64 Hz and
/// authoritative snapshots are scheduled at 32 Hz.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WebSocketTransport;

impl TransportMarker for WebSocketTransport {
    const POLICY: TransportPolicy = TransportPolicy {
        kind: TransportKind::WebSocket,
        input_hz: 64,
        snapshot_hz: 32,
        supports_unreliable_datagrams: false,
        max_datagram_bytes: MAX_DATAGRAM_BYTES as u16,
    };
}

/// Marker for offline and deterministic integration-test sessions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopbackTransport;

impl TransportMarker for LoopbackTransport {
    const POLICY: TransportPolicy = TransportPolicy {
        kind: TransportKind::Loopback,
        input_hz: 128,
        snapshot_hz: 128,
        supports_unreliable_datagrams: true,
        max_datagram_bytes: MAX_DATAGRAM_BYTES as u16,
    };
}

/// Socket implementations are injected here; QUIC libraries, browser APIs,
/// and console SDK types remain outside the shared client crate.
pub trait ClientTransport {
    type Error;
    type Marker: TransportMarker;

    fn send_input_batch(&mut self, batch: &InputBatch) -> Result<(), Self::Error>;

    fn send_reliable(&mut self, message: &MessageEnvelope) -> Result<(), Self::Error>;

    fn poll_incoming(&mut self) -> Result<Option<IncomingServerMessage>, Self::Error>;

    fn request_keyframe(&mut self) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputBatchSchedulerError {
    InvalidCadence,
    InvalidCommandOrder(ValidationError),
}

/// Converts the 128 Hz prediction command stream into the transport-specific
/// send cadence while retaining the newest three commands for loss recovery.
///
/// Native QUIC and loopback emit every command. Browser WSS emits every two
/// commands (64 Hz), with the prior command retained in the same binary batch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputBatchScheduler {
    commands_per_batch: u16,
    commands_since_batch: u16,
    history_newest_first: Vec<PlayerCommand>,
}

impl InputBatchScheduler {
    pub fn new(policy: TransportPolicy) -> Result<Self, InputBatchSchedulerError> {
        if policy.input_hz == 0
            || policy.input_hz > AUTHORITATIVE_HZ as u16
            || AUTHORITATIVE_HZ as u16 % policy.input_hz != 0
        {
            return Err(InputBatchSchedulerError::InvalidCadence);
        }
        let commands_per_batch = AUTHORITATIVE_HZ as u16 / policy.input_hz;
        if commands_per_batch == 0 || commands_per_batch as usize > COMMAND_REDUNDANCY {
            return Err(InputBatchSchedulerError::InvalidCadence);
        }
        Ok(Self {
            commands_per_batch,
            commands_since_batch: 0,
            history_newest_first: Vec::with_capacity(COMMAND_REDUNDANCY),
        })
    }

    pub const fn commands_per_batch(&self) -> u16 {
        self.commands_per_batch
    }

    pub const fn pending_commands(&self) -> u16 {
        self.commands_since_batch
    }

    /// Returns a batch exactly when this transport's cadence is due.
    ///
    /// State changes only after the staged history validates, so a malformed
    /// or non-monotonic command cannot corrupt future redundancy.
    pub fn push(
        &mut self,
        command: PlayerCommand,
    ) -> Result<Option<InputBatch>, InputBatchSchedulerError> {
        let mut staged = self.history_newest_first.clone();
        staged.insert(0, command);
        staged.truncate(COMMAND_REDUNDANCY);
        let validated = InputBatch::new(staged.clone())
            .map_err(InputBatchSchedulerError::InvalidCommandOrder)?;
        let staged_count = self.commands_since_batch.saturating_add(1);
        let due = staged_count >= self.commands_per_batch;
        self.history_newest_first = staged;
        self.commands_since_batch = if due { 0 } else { staged_count };
        Ok(due.then_some(validated))
    }

    /// Flushes a partial browser batch before suspend, disconnect, or an
    /// action that must not wait for the next 64 Hz send boundary.
    pub fn flush(&mut self) -> Result<Option<InputBatch>, InputBatchSchedulerError> {
        if self.commands_since_batch == 0 {
            return Ok(None);
        }
        let batch = InputBatch::new(self.history_newest_first.clone())
            .map_err(InputBatchSchedulerError::InvalidCommandOrder)?;
        self.commands_since_batch = 0;
        Ok(Some(batch))
    }
}

const JITTER_SCALE: u64 = 256;
const BASE_INTERPOLATION_TICKS: u8 = 2;
const MAX_JITTER_SAMPLE_TICKS: u64 = 16;

/// Integer EWMA of snapshot arrival jitter.
///
/// The estimator compares wall-clock spacing with authoritative tick spacing
/// and converts the result into the required adaptive 2–6 tick interpolation
/// delay. Obsolete snapshots and clock regressions do not perturb it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotJitterEstimator {
    last_arrival_nanoseconds: Option<u64>,
    last_server_tick: Option<u64>,
    jitter_ticks_q8: u64,
    interpolation_delay_ticks: u8,
}

impl SnapshotJitterEstimator {
    pub const fn new() -> Self {
        Self {
            last_arrival_nanoseconds: None,
            last_server_tick: None,
            jitter_ticks_q8: 0,
            interpolation_delay_ticks: BASE_INTERPOLATION_TICKS,
        }
    }

    pub const fn interpolation_delay_ticks(&self) -> u8 {
        self.interpolation_delay_ticks
    }

    pub const fn estimated_jitter_ticks_q8(&self) -> u64 {
        self.jitter_ticks_q8
    }

    pub fn observe(&mut self, arrival_nanoseconds: u64, server_tick: u64) -> u8 {
        let current_arrival = arrival_nanoseconds;
        let (Some(previous_arrival), Some(previous_tick)) =
            (self.last_arrival_nanoseconds, self.last_server_tick)
        else {
            self.last_arrival_nanoseconds = Some(arrival_nanoseconds);
            self.last_server_tick = Some(server_tick);
            return self.interpolation_delay_ticks;
        };
        if arrival_nanoseconds < previous_arrival || server_tick <= previous_tick {
            return self.interpolation_delay_ticks;
        }

        let arrival_delta = arrival_nanoseconds - previous_arrival;
        let server_delta_ticks = server_tick - previous_tick;
        let expected_nanoseconds = (server_delta_ticks as u128)
            .saturating_mul(1_000_000_000_u128)
            / AUTHORITATIVE_HZ as u128;
        let actual_nanoseconds = arrival_delta as u128;
        let deviation_nanoseconds = actual_nanoseconds.abs_diff(expected_nanoseconds);
        let sample_q8 = deviation_nanoseconds
            .saturating_mul(AUTHORITATIVE_HZ as u128)
            .saturating_mul(JITTER_SCALE as u128)
            / 1_000_000_000_u128;
        let sample_q8 = u64::try_from(sample_q8)
            .unwrap_or(u64::MAX)
            .min(MAX_JITTER_SAMPLE_TICKS * JITTER_SCALE);
        self.jitter_ticks_q8 = self
            .jitter_ticks_q8
            .saturating_mul(7)
            .saturating_add(sample_q8)
            / 8;
        let jitter_ticks = self
            .jitter_ticks_q8
            .saturating_add(JITTER_SCALE - 1)
            / JITTER_SCALE;
        self.interpolation_delay_ticks = BASE_INTERPOLATION_TICKS
            .saturating_add(u8::try_from(jitter_ticks).unwrap_or(u8::MAX))
            .clamp(
                aetherloom_core::MIN_INTERPOLATION_DELAY_TICKS,
                aetherloom_core::MAX_INTERPOLATION_DELAY_TICKS,
            );
        self.last_arrival_nanoseconds = Some(current_arrival);
        self.last_server_tick = Some(server_tick);
        self.interpolation_delay_ticks
    }

    pub fn observe_and_apply(
        &mut self,
        arrival_nanoseconds: u64,
        server_tick: u64,
        replica: &mut ClientReplica,
    ) -> u8 {
        let delay = self.observe(arrival_nanoseconds, server_tick);
        replica.set_interpolation_delay_ticks(delay);
        delay
    }
}

impl Default for SnapshotJitterEstimator {
    fn default() -> Self {
        Self::new()
    }
}
