#![forbid(unsafe_code)]

mod admission;
mod config;
mod deployment;
mod egress;
mod file_sinks;
mod grid;
mod join_ticket;
mod quic_admission;
mod runtime;
mod sinks;

pub use admission::{
    AdmissionError, SignedTicketVerifier, TicketVerificationError, VerifiedTicket,
};
pub use config::{ConfigError, MatchAdmissionScope, MatchBuild, ProcessConfig};
pub use deployment::{
    load_admin_token, load_quinn_server_config, lobby_decision, lobby_poll_interval,
    write_health_file, DedicatedServerConfig, DeploymentConfigError, DeploymentHealth,
    DeploymentPhase, HealthService, LobbyDecision, TlsLoadError, DEPLOYMENT_USAGE,
};
pub use egress::{EgressClass, EgressStats, EnqueueOutcome, OutboundPacket, PriorityEgress};
pub use file_sinks::{FileReplaySpool, FileSettlementSpool, FileSpoolError};
pub use grid::{CellCoord, DeterministicSpatialGrid, DEFAULT_CELL_SIZE_CM};
pub use join_ticket::{Ed25519JoinTicketVerifier, TicketVerifierConfigError};
pub use quic_admission::{
    pump_quic_connection_events, NoDirectTicketVerifier, QuicConnectionPumpError,
    QuicConnectionPumpReport, RejectedQuicAdmission, SequentialPeerIdAllocator,
    ServerPeerIdAllocator, SystemUnixTime, TicketConnectionAdmission, UnixTimeSource,
    MAX_SIGNED_CONNECTION_TICKET_BYTES,
};
pub use runtime::{
    CapacitySignal, DedicatedMatch, HealthReport, InboundRejection, ProcessError,
    ReplicationProfile, TickReport, MAX_COMMAND_FUTURE_TICKS, MAX_SIMULATION_HEADROOM_NS,
    MAX_TICK_JITTER_HEADROOM_NS,
};
pub use sinks::{
    NoopReplaySink, NoopSettlementSink, PersistenceKind, ReplayCheckpointSink, ReplayChunk,
    SettlementSink, SinkError,
};
