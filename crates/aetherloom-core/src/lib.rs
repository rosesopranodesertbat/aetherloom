#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(any(test, feature = "std"))]
extern crate std;

mod checkpoint;
mod codec;
mod entity;
mod platform;
mod replication;
mod rng;
mod state;
mod terrain;

pub use aetherloom_protocol::{
    CommandSet, Controller, EntityId, PlayerCommand, PlayerId, SnapshotId, TeamId,
    AUTHORITATIVE_HZ, AUTHORITATIVE_TICK_NANOS, MAX_PLAYERS,
};
pub use checkpoint::{
    AuthoritativeCheckpoint, CheckpointError, CHECKPOINT_MAGIC, CHECKPOINT_VERSION,
};
pub use entity::{Entity, EntityKind, EntityPool, EntityPoolError};
pub use platform::{
    PlatformError, PlatformServices, PresentationEntity, PresentationFrame, PresentationPlayer,
    PresentationWorld, RendererBackend,
};
pub use replication::{
    ClientReplica, InterestTier, ReplicaError, ReplicatedEntity, ReplicationError,
    ReplicaSample, Snapshot, SnapshotKind, TerrainRevision,
    MAX_INTERPOLATION_DELAY_TICKS, MAX_INTERPOLATION_SAMPLES,
    MAX_PENDING_PREDICTION_COMMANDS, MIN_INTERPOLATION_DELAY_TICKS,
};
pub use state::{
    CommandRejection, CoreError, Inventory, MatchConfig, MatchState, PlayerOutcome,
    PlayerState, Pose, PoseFrame, RewindError, RngState, RngStreams, TickEvent,
    TickEventKind, TickEvents, WorldSeed, MAX_MATCH_ENTITIES,
    MAX_MATCH_TERRAIN_CHUNKS, MAX_REWIND_TICKS, MAX_SNAPSHOT_HISTORY_CAPACITY,
    POSE_HISTORY_TICKS,
};
pub use terrain::{
    ChunkCoord, TerrainChunk, TerrainDeformationEvent, TerrainError, CELL_WORLD_CM,
    TERRAIN_CELLS, TERRAIN_CHUNK_SIDE,
};
