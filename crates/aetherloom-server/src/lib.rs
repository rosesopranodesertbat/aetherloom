//! Platform-neutral foundations for an authoritative Aetherloom match server.
//!
//! This crate deliberately contains no QUIC, TLS, Cloudflare, or game-simulation
//! dependency. Platform crates implement [`NetworkTransport`] and place the
//! resulting adapter behind [`DedicatedQuicHost`] or [`CloudflareWssHost`].
//! Keeping those integrations outside this crate prevents transport and
//! credential choices from leaking into deterministic gameplay.

mod host;
mod impairment;
mod performance;
mod policy;
mod queue;
mod scheduler;
mod transport;

pub use host::{
    CloudflareWssHost, DedicatedQuicHost, HostError, HostKind, HostState, MatchHost,
};
pub use impairment::{
    ImpairmentConfig, ImpairmentConfigError, ImpairmentStats, LatestSequenceFilter,
    NetworkImpairment, OverflowPolicy, SequenceDecision, SequencedDatagram,
    MAX_ONE_WAY_LATENCY, RATE_DENOMINATOR,
};
pub use performance::{
    run_performance_gate, GateViolation, PerformanceGateConfig,
    PerformanceGateConfigError, PerformanceGateReport, PerformanceGateRunError,
    QueueLoadSample, SoakTickContext, MAX_SIMULATION_P99, MAX_SOAK_DURATION,
    MAX_MEDIAN_EGRESS_BYTES_PER_SECOND, MAX_P95_EGRESS_BYTES_PER_SECOND,
    MAX_START_JITTER_P99, PERFORMANCE_GATE_PLAYERS, PRODUCTION_SOAK_DURATION,
};
pub use policy::{
    disconnect_action, can_reconnect, DisconnectAction, BOT_TAKEOVER_AFTER,
    BOT_TAKEOVER_AFTER_TICKS, CLEAR_INPUT_ON_DISCONNECT, RECONNECT_GRACE_PERIOD,
    RECONNECT_GRACE_TICKS,
};
pub use queue::{
    bounded_channel, runtime_channels, BoundedReceiver, BoundedSender, NetworkIo,
    QueueConfigError, QueueReceiveError, QueueSendError, SimulationIo,
};
pub use scheduler::{
    MonotonicClock, SchedulerError, SystemClock, TickId, TickMetrics, TickMetricsSnapshot,
    TickPermit, TickSample, TickScheduler, AUTHORITATIVE_HZ, TICK_PERIOD,
    TICK_PERIOD_NS,
};
pub use transport::{
    DeliveryKind, DisconnectReason, InboundMessage, LoopbackTransport, NetworkTransport,
    PeerId, TransportCapabilities, TransportError, MAX_GAMEPLAY_DATAGRAM_BYTES,
};
