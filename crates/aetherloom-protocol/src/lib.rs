#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(any(test, feature = "std"))]
extern crate std;

mod admission;
mod codec;
mod command;
mod error;
mod ids;
mod message;

pub use admission::{InputPool, RegionId, MAX_REGION_ID_BYTES};
pub use command::{
    CommandSet, InputBatch, PlayerCommand, ACTION_CAST, ACTION_DASH, ACTION_EXTRACT,
    ACTION_INTERACT, ACTION_JUMP, ACTION_PRIMARY, ACTION_SECONDARY, ALLOWED_ACTION_FLAGS,
    COMMAND_REDUNDANCY, MAX_LOOK_PITCH, MAX_MOVE_AXIS, MAX_SPELL_ID,
};
pub use error::{ProtocolError, ValidationError};
pub use ids::{Controller, EntityId, PlayerId, SnapshotId, TeamId};
pub use message::{
    ChunkRevision, Delivery, EntityState, EnvelopeMetadata, EventBatch, EventKind, GameEvent,
    LootEntry, MatchOutcome, MatchResult, Message, MessageEnvelope, MessageKind, PlayerMatchResult,
    SnapshotDelta, SnapshotKeyframe, TerrainChunkState, TerrainDelta, TerrainOp, TerrainOpKind,
    HEADER_BYTES, MAX_DATAGRAM_BYTES, MAX_EVENTS_PER_BATCH, MAX_RELIABLE_FRAME_BYTES,
    PROTOCOL_MAGIC, PROTOCOL_VERSION, TERRAIN_CELLS_PER_CHUNK, TERRAIN_CHUNK_SIDE,
};

/// The fixed authoritative simulation frequency.
pub const AUTHORITATIVE_HZ: u32 = 128;

/// The duration of one authoritative tick in nanoseconds.
pub const AUTHORITATIVE_TICK_NANOS: u64 = 7_812_500;

/// Maximum number of combatants addressable by one match.
pub const MAX_PLAYERS: usize = 128;
