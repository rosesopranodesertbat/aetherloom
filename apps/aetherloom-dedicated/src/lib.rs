#![forbid(unsafe_code)]

mod admission;
mod config;
mod egress;
mod grid;
mod quic_admission;
mod runtime;
mod sinks;

pub use admission::{
    AdmissionError, SignedTicketVerifier, TicketVerificationError, VerifiedTicket,
};
pub use config::{ConfigError, MatchBuild, ProcessConfig};
pub use egress::{
    EgressClass, EgressStats, EnqueueOutcome, OutboundPacket, PriorityEgress,
};
pub use grid::{CellCoord, DeterministicSpatialGrid, DEFAULT_CELL_SIZE_CM};
pub use quic_admission::{
    pump_quic_connection_events, NoDirectTicketVerifier,
    QuicConnectionPumpError, QuicConnectionPumpReport,
    RejectedQuicAdmission, SequentialPeerIdAllocator,
    ServerPeerIdAllocator, SystemUnixTime, TicketConnectionAdmission,
    UnixTimeSource, MAX_SIGNED_CONNECTION_TICKET_BYTES,
};
pub use runtime::{
    CapacitySignal, DedicatedMatch, HealthReport, InboundRejection, ProcessError,
    ReplicationProfile, TickReport, MAX_COMMAND_FUTURE_TICKS,
    MAX_SIMULATION_HEADROOM_NS, MAX_TICK_JITTER_HEADROOM_NS,
};
pub use sinks::{
    NoopReplaySink, NoopSettlementSink, PersistenceKind, ReplayChunk,
    ReplayCheckpointSink, SettlementSink, SinkError,
};
