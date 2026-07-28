use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::codec::{Reader, Writer};
use crate::{
    EntityId, InputBatch, PlayerId, ProtocolError, SnapshotId, TeamId, ValidationError,
    MAX_LOOK_PITCH, MAX_PLAYERS,
};

pub const PROTOCOL_MAGIC: [u8; 4] = *b"ALMP";
pub const PROTOCOL_VERSION: u16 = 3;
pub const HEADER_BYTES: usize = 60;
pub const MAX_DATAGRAM_BYTES: usize = 1_200;
pub const MAX_RELIABLE_FRAME_BYTES: usize = 4 * 1024 * 1024;

const MAX_DELTA_ENTITIES: usize = 32;
const MAX_REMOVED_ENTITIES: usize = 64;
const MAX_KEYFRAME_ENTITIES: usize = 8_192;
const MAX_KEYFRAME_CHUNKS: usize = 4_096;
pub const MAX_EVENTS_PER_BATCH: usize = 16;
const MAX_LOOT_ENTRIES: usize = 64;
pub const TERRAIN_CHUNK_SIDE: u16 = 8;
pub const TERRAIN_CELLS_PER_CHUNK: usize =
    TERRAIN_CHUNK_SIDE as usize * TERRAIN_CHUNK_SIDE as usize;
const MAX_TERRAIN_OPS: usize =
    TERRAIN_CELLS_PER_CHUNK;
const NONE_PLAYER_OR_TEAM: u16 = u16::MAX;
const NONE_ENTITY: u64 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Delivery {
    Datagram = 1,
    Reliable = 2,
}

impl Delivery {
    fn from_wire(raw: u8) -> Result<Self, ProtocolError> {
        match raw {
            1 => Ok(Self::Datagram),
            2 => Ok(Self::Reliable),
            _ => Err(ProtocolError::InvalidEnum {
                field: "delivery",
                value: raw as u64,
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageKind {
    InputBatch = 1,
    SnapshotDelta = 2,
    SnapshotKeyframe = 3,
    EventBatch = 4,
    TerrainDelta = 5,
    MatchResult = 6,
}

impl MessageKind {
    fn from_wire(raw: u8) -> Result<Self, ProtocolError> {
        match raw {
            1 => Ok(Self::InputBatch),
            2 => Ok(Self::SnapshotDelta),
            3 => Ok(Self::SnapshotKeyframe),
            4 => Ok(Self::EventBatch),
            5 => Ok(Self::TerrainDelta),
            6 => Ok(Self::MatchResult),
            _ => Err(ProtocolError::UnknownMessageKind(raw)),
        }
    }

    fn allows(self, delivery: Delivery) -> bool {
        match self {
            Self::InputBatch | Self::SnapshotDelta => delivery == Delivery::Datagram,
            Self::SnapshotKeyframe | Self::TerrainDelta | Self::MatchResult => {
                delivery == Delivery::Reliable
            }
            Self::EventBatch => true,
        }
    }
}

/// Envelope fields shared by datagram and reliable transports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvelopeMetadata {
    pub content_build_hash: [u8; 16],
    pub match_epoch: u64,
    pub sequence: u32,
    pub server_tick: u64,
    pub acknowledgement_tick: u64,
}

impl EnvelopeMetadata {
    pub const fn new(
        content_build_hash: [u8; 16],
        match_epoch: u64,
        sequence: u32,
        server_tick: u64,
        acknowledgement_tick: u64,
    ) -> Self {
        Self {
            content_build_hash,
            match_epoch,
            sequence,
            server_tick,
            acknowledgement_tick,
        }
    }
}

/// Quantized authoritative entity state.
///
/// Positions are signed centimetres. Velocities are signed centimetres per
/// authoritative tick. Angles use the same yaw/pitch representation as input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityState {
    pub entity_id: EntityId,
    pub archetype: u16,
    pub owner: Option<PlayerId>,
    pub team: Option<TeamId>,
    pub position_cm: [i32; 3],
    pub velocity_cm_per_tick: [i16; 3],
    pub yaw: u16,
    pub pitch: i16,
    pub health: u16,
    pub flags: u16,
}

impl EntityState {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.archetype == 0 {
            return Err(ValidationError::InvalidArchetype(self.archetype));
        }
        if !(-MAX_LOOK_PITCH..=MAX_LOOK_PITCH).contains(&self.pitch) {
            return Err(ValidationError::LookPitchOutOfRange(self.pitch));
        }
        Ok(())
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        writer.u64(self.entity_id.get());
        writer.u16(self.archetype);
        writer.u16(
            self.owner
                .map(PlayerId::get)
                .unwrap_or(NONE_PLAYER_OR_TEAM),
        );
        writer.u16(self.team.map(TeamId::get).unwrap_or(NONE_PLAYER_OR_TEAM));
        for value in self.position_cm {
            writer.i32(value);
        }
        for value in self.velocity_cm_per_tick {
            writer.i16(value);
        }
        writer.u16(self.yaw);
        writer.i16(self.pitch);
        writer.u16(self.health);
        writer.u16(self.flags);
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let entity_id = EntityId::from_raw(reader.u64()?)?;
        let archetype = reader.u16()?;
        let owner = decode_optional_player(reader.u16()?)?;
        let team = decode_optional_team(reader.u16()?)?;
        let position_cm = [reader.i32()?, reader.i32()?, reader.i32()?];
        let velocity_cm_per_tick = [reader.i16()?, reader.i16()?, reader.i16()?];
        let yaw = reader.u16()?;
        let pitch = reader.i16()?;
        let health = reader.u16()?;
        let flags = reader.u16()?;
        let state = Self {
            entity_id,
            archetype,
            owner,
            team,
            position_cm,
            velocity_cm_per_tick,
            yaw,
            pitch,
            health,
            flags,
        };
        state.validate()?;
        Ok(state)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotDelta {
    pub snapshot_id: SnapshotId,
    pub baseline_id: SnapshotId,
    pub viewer: PlayerId,
    pub acknowledged_input_sequence: u32,
    pub entities: Vec<EntityState>,
    pub removed_entities: Vec<EntityId>,
}

impl SnapshotDelta {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_snapshot_id(self.snapshot_id)?;
        if self.baseline_id.is_none() || self.snapshot_id == self.baseline_id {
            return Err(ValidationError::InvalidBaseline {
                snapshot: self.snapshot_id.get(),
                baseline: self.baseline_id.get(),
            });
        }
        validate_count("snapshot delta entities", self.entities.len(), MAX_DELTA_ENTITIES)?;
        validate_count(
            "snapshot removed entities",
            self.removed_entities.len(),
            MAX_REMOVED_ENTITIES,
        )?;
        for entity in &self.entities {
            entity.validate()?;
        }
        validate_unique_entities(&self.entities, &self.removed_entities)?;
        Ok(())
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        writer.u32(self.snapshot_id.get());
        writer.u32(self.baseline_id.get());
        writer.u16(self.viewer.get());
        writer.u32(self.acknowledged_input_sequence);
        encode_count(writer, self.entities.len(), "snapshot delta entities")?;
        for entity in &self.entities {
            entity.encode(writer)?;
        }
        encode_count(
            writer,
            self.removed_entities.len(),
            "snapshot removed entities",
        )?;
        for entity_id in &self.removed_entities {
            writer.u64(entity_id.get());
        }
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let snapshot_id = SnapshotId::new(reader.u32()?);
        let baseline_id = SnapshotId::new(reader.u32()?);
        let viewer = PlayerId::new(reader.u16()?)?;
        let acknowledged_input_sequence = reader.u32()?;
        let entity_count =
            decode_count(reader, "snapshot delta entities", MAX_DELTA_ENTITIES)?;
        let mut entities = Vec::with_capacity(entity_count);
        for _ in 0..entity_count {
            entities.push(EntityState::decode(reader)?);
        }
        let removed_count =
            decode_count(reader, "snapshot removed entities", MAX_REMOVED_ENTITIES)?;
        let mut removed_entities = Vec::with_capacity(removed_count);
        for _ in 0..removed_count {
            removed_entities.push(EntityId::from_raw(reader.u64()?)?);
        }
        let snapshot = Self {
            snapshot_id,
            baseline_id,
            viewer,
            acknowledged_input_sequence,
            entities,
            removed_entities,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChunkRevision {
    pub x: i32,
    pub y: i32,
    pub revision: u32,
}

/// Complete authoritative terrain state for one revisioned chunk.
///
/// Keyframes include these fixed-size chunks so a client that missed one or
/// more reliable deltas can replace its terrain state instead of knowing only
/// that its local revision is stale.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainChunkState {
    pub x: i32,
    pub y: i32,
    pub revision: u32,
    pub heights_cm: Vec<i16>,
}

impl TerrainChunkState {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.revision == 0 {
            return Err(ValidationError::InvalidTerrainRevision {
                base: 0,
                new: self.revision,
            });
        }
        if self.heights_cm.len() != TERRAIN_CELLS_PER_CHUNK {
            return Err(ValidationError::InvalidTerrainCellCount {
                count: self.heights_cm.len(),
                expected: TERRAIN_CELLS_PER_CHUNK,
            });
        }
        Ok(())
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        writer.i32(self.x);
        writer.i32(self.y);
        writer.u32(self.revision);
        for height_cm in &self.heights_cm {
            writer.i16(*height_cm);
        }
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let x = reader.i32()?;
        let y = reader.i32()?;
        let revision = reader.u32()?;
        let mut heights_cm = Vec::with_capacity(TERRAIN_CELLS_PER_CHUNK);
        for _ in 0..TERRAIN_CELLS_PER_CHUNK {
            heights_cm.push(reader.i16()?);
        }
        let chunk = Self {
            x,
            y,
            revision,
            heights_cm,
        };
        chunk.validate()?;
        Ok(chunk)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotKeyframe {
    pub snapshot_id: SnapshotId,
    pub viewer: PlayerId,
    pub acknowledged_input_sequence: u32,
    pub entities: Vec<EntityState>,
    pub terrain_revisions: Vec<ChunkRevision>,
    pub terrain_chunks: Vec<TerrainChunkState>,
}

impl SnapshotKeyframe {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_snapshot_id(self.snapshot_id)?;
        validate_count(
            "snapshot keyframe entities",
            self.entities.len(),
            MAX_KEYFRAME_ENTITIES,
        )?;
        validate_count(
            "snapshot keyframe chunks",
            self.terrain_revisions.len(),
            MAX_KEYFRAME_CHUNKS,
        )?;
        validate_count(
            "snapshot keyframe terrain states",
            self.terrain_chunks.len(),
            MAX_KEYFRAME_CHUNKS,
        )?;
        for entity in &self.entities {
            entity.validate()?;
        }
        validate_unique_entity_states(&self.entities)?;
        let mut chunks = BTreeSet::new();
        for revision in &self.terrain_revisions {
            if !chunks.insert((revision.x, revision.y)) {
                return Err(ValidationError::DuplicateChunk {
                    x: revision.x,
                    y: revision.y,
                });
            }
        }
        let mut terrain_states = BTreeSet::new();
        for chunk in &self.terrain_chunks {
            chunk.validate()?;
            if !terrain_states.insert((chunk.x, chunk.y)) {
                return Err(ValidationError::DuplicateChunk {
                    x: chunk.x,
                    y: chunk.y,
                });
            }
        }
        if self.terrain_revisions.len() != self.terrain_chunks.len() {
            return Err(ValidationError::TerrainKeyframeMismatch);
        }
        for revision in &self.terrain_revisions {
            let Some(chunk) = self
                .terrain_chunks
                .iter()
                .find(|chunk| chunk.x == revision.x && chunk.y == revision.y)
            else {
                return Err(ValidationError::TerrainKeyframeMismatch);
            };
            if chunk.revision != revision.revision {
                return Err(ValidationError::TerrainKeyframeMismatch);
            }
        }
        Ok(())
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        writer.u32(self.snapshot_id.get());
        writer.u16(self.viewer.get());
        writer.u32(self.acknowledged_input_sequence);
        encode_count(writer, self.entities.len(), "snapshot keyframe entities")?;
        for entity in &self.entities {
            entity.encode(writer)?;
        }
        encode_count(
            writer,
            self.terrain_revisions.len(),
            "snapshot keyframe chunks",
        )?;
        for revision in &self.terrain_revisions {
            writer.i32(revision.x);
            writer.i32(revision.y);
            writer.u32(revision.revision);
        }
        encode_count(
            writer,
            self.terrain_chunks.len(),
            "snapshot keyframe terrain states",
        )?;
        for chunk in &self.terrain_chunks {
            chunk.encode(writer)?;
        }
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let snapshot_id = SnapshotId::new(reader.u32()?);
        let viewer = PlayerId::new(reader.u16()?)?;
        let acknowledged_input_sequence = reader.u32()?;
        let entity_count =
            decode_count(reader, "snapshot keyframe entities", MAX_KEYFRAME_ENTITIES)?;
        let mut entities = Vec::with_capacity(entity_count);
        for _ in 0..entity_count {
            entities.push(EntityState::decode(reader)?);
        }
        let revision_count =
            decode_count(reader, "snapshot keyframe chunks", MAX_KEYFRAME_CHUNKS)?;
        let mut terrain_revisions = Vec::with_capacity(revision_count);
        for _ in 0..revision_count {
            terrain_revisions.push(ChunkRevision {
                x: reader.i32()?,
                y: reader.i32()?,
                revision: reader.u32()?,
            });
        }
        let terrain_count = decode_count(
            reader,
            "snapshot keyframe terrain states",
            MAX_KEYFRAME_CHUNKS,
        )?;
        let mut terrain_chunks = Vec::with_capacity(terrain_count);
        for _ in 0..terrain_count {
            terrain_chunks.push(TerrainChunkState::decode(reader)?);
        }
        let keyframe = Self {
            snapshot_id,
            viewer,
            acknowledged_input_sequence,
            entities,
            terrain_revisions,
            terrain_chunks,
        };
        keyframe.validate()?;
        Ok(keyframe)
    }
}

/// Extensible numeric event code. Values 1 through 1,024 are reserved for
/// version-one gameplay events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventKind(u16);

impl EventKind {
    pub const SPAWN: Self = Self(1);
    pub const DESPAWN: Self = Self(2);
    pub const CAST: Self = Self(3);
    pub const HIT: Self = Self(4);
    pub const DAMAGE: Self = Self(5);
    pub const DEATH: Self = Self(6);
    pub const EXTRACTION: Self = Self(7);
    pub const TERRAIN_DEFORMED: Self = Self(8);
    pub const PLAYER_JOINED: Self = Self(9);
    pub const PLAYER_LEFT: Self = Self(10);

    pub fn new(raw: u16) -> Result<Self, ValidationError> {
        if (1..=1_024).contains(&raw) {
            Ok(Self(raw))
        } else {
            Err(ValidationError::InvalidEventKind(raw))
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameEvent {
    pub event_id: u64,
    pub tick: u64,
    pub kind: EventKind,
    pub actor: Option<EntityId>,
    pub target: Option<EntityId>,
    pub data: [i32; 4],
}

impl GameEvent {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.event_id == 0 {
            return Err(ValidationError::InvalidEventId(self.event_id));
        }
        EventKind::new(self.kind.get())?;
        Ok(())
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        writer.u64(self.event_id);
        writer.u64(self.tick);
        writer.u16(self.kind.get());
        writer.u64(self.actor.map(EntityId::get).unwrap_or(NONE_ENTITY));
        writer.u64(self.target.map(EntityId::get).unwrap_or(NONE_ENTITY));
        for value in self.data {
            writer.i32(value);
        }
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let event = Self {
            event_id: reader.u64()?,
            tick: reader.u64()?,
            kind: EventKind::new(reader.u16()?)?,
            actor: decode_optional_entity(reader.u64()?)?,
            target: decode_optional_entity(reader.u64()?)?,
            data: [reader.i32()?, reader.i32()?, reader.i32()?, reader.i32()?],
        };
        event.validate()?;
        Ok(event)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventBatch {
    pub events: Vec<GameEvent>,
}

impl EventBatch {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_non_empty("events", &self.events)?;
        validate_count("events", self.events.len(), MAX_EVENTS_PER_BATCH)?;
        let mut event_ids = BTreeSet::new();
        for event in &self.events {
            event.validate()?;
            if !event_ids.insert(event.event_id) {
                return Err(ValidationError::DuplicateEvent(event.event_id));
            }
        }
        Ok(())
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        encode_count(writer, self.events.len(), "events")?;
        for event in &self.events {
            event.encode(writer)?;
        }
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let count = decode_count(reader, "events", MAX_EVENTS_PER_BATCH)?;
        if count == 0 {
            return Err(ValidationError::EmptyCollection("events").into());
        }
        let mut events = Vec::with_capacity(count);
        for _ in 0..count {
            events.push(GameEvent::decode(reader)?);
        }
        let batch = Self { events };
        batch.validate()?;
        Ok(batch)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TerrainOpKind {
    HeightDelta = 1,
    SetMaterial = 2,
    Crater = 3,
}

impl TerrainOpKind {
    fn from_wire(raw: u8) -> Result<Self, ProtocolError> {
        match raw {
            1 => Ok(Self::HeightDelta),
            2 => Ok(Self::SetMaterial),
            3 => Ok(Self::Crater),
            _ => Err(ValidationError::InvalidTerrainOperation(raw).into()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainOp {
    pub kind: TerrainOpKind,
    pub cell_x: u16,
    pub cell_y: u16,
    pub height_delta_cm: i16,
    pub material: u8,
    pub flags: u8,
}

impl TerrainOp {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.cell_x >= TERRAIN_CHUNK_SIDE || self.cell_y >= TERRAIN_CHUNK_SIDE {
            return Err(ValidationError::InvalidTerrainCell {
                x: self.cell_x,
                y: self.cell_y,
            });
        }
        Ok(())
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        writer.u8(self.kind as u8);
        writer.u16(self.cell_x);
        writer.u16(self.cell_y);
        writer.i16(self.height_delta_cm);
        writer.u8(self.material);
        writer.u8(self.flags);
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let operation = Self {
            kind: TerrainOpKind::from_wire(reader.u8()?)?,
            cell_x: reader.u16()?,
            cell_y: reader.u16()?,
            height_delta_cm: reader.i16()?,
            material: reader.u8()?,
            flags: reader.u8()?,
        };
        operation.validate()?;
        Ok(operation)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerrainDelta {
    pub chunk_x: i32,
    pub chunk_y: i32,
    pub base_revision: u32,
    pub new_revision: u32,
    pub operations: Vec<TerrainOp>,
}

impl TerrainDelta {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.new_revision == 0
            || !serial_is_newer(self.new_revision, self.base_revision)
        {
            return Err(ValidationError::InvalidTerrainRevision {
                base: self.base_revision,
                new: self.new_revision,
            });
        }
        validate_non_empty("terrain operations", &self.operations)?;
        validate_count(
            "terrain operations",
            self.operations.len(),
            MAX_TERRAIN_OPS,
        )?;
        let mut cells = BTreeSet::new();
        for operation in &self.operations {
            operation.validate()?;
            if !cells.insert((operation.cell_x, operation.cell_y)) {
                return Err(ValidationError::DuplicateTerrainCell {
                    x: operation.cell_x,
                    y: operation.cell_y,
                });
            }
        }
        Ok(())
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        writer.i32(self.chunk_x);
        writer.i32(self.chunk_y);
        writer.u32(self.base_revision);
        writer.u32(self.new_revision);
        encode_count(writer, self.operations.len(), "terrain operations")?;
        for operation in &self.operations {
            operation.encode(writer)?;
        }
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let chunk_x = reader.i32()?;
        let chunk_y = reader.i32()?;
        let base_revision = reader.u32()?;
        let new_revision = reader.u32()?;
        let count = decode_count(reader, "terrain operations", MAX_TERRAIN_OPS)?;
        if count == 0 {
            return Err(ValidationError::EmptyCollection("terrain operations").into());
        }
        let mut operations = Vec::with_capacity(count);
        for _ in 0..count {
            operations.push(TerrainOp::decode(reader)?);
        }
        let delta = Self {
            chunk_x,
            chunk_y,
            base_revision,
            new_revision,
            operations,
        };
        delta.validate()?;
        Ok(delta)
    }
}

fn serial_is_newer(received: u32, previous: u32) -> bool {
    let distance = received.wrapping_sub(previous);
    distance != 0 && distance < (1_u32 << 31)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MatchOutcome {
    Extracted = 1,
    Defeated = 2,
    Abandoned = 3,
    Disconnected = 4,
}

impl MatchOutcome {
    fn from_wire(raw: u8) -> Result<Self, ProtocolError> {
        match raw {
            1 => Ok(Self::Extracted),
            2 => Ok(Self::Defeated),
            3 => Ok(Self::Abandoned),
            4 => Ok(Self::Disconnected),
            _ => Err(ProtocolError::InvalidEnum {
                field: "match outcome",
                value: raw as u64,
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LootEntry {
    pub item_id: u32,
    pub quantity: u32,
}

impl LootEntry {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.item_id == 0 {
            return Err(ValidationError::InvalidLootItem(self.item_id));
        }
        if self.quantity == 0 {
            return Err(ValidationError::InvalidLootQuantity(self.quantity));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerMatchResult {
    pub player_id: PlayerId,
    pub team_id: TeamId,
    pub outcome: MatchOutcome,
    pub rating_delta: i16,
    pub score: u32,
    pub banked_loot: Vec<LootEntry>,
}

impl PlayerMatchResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_count("banked loot", self.banked_loot.len(), MAX_LOOT_ENTRIES)?;
        let mut item_ids = BTreeSet::new();
        for loot in &self.banked_loot {
            loot.validate()?;
            if !item_ids.insert(loot.item_id) {
                return Err(ValidationError::DuplicateLootItem(loot.item_id));
            }
        }
        Ok(())
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        writer.u16(self.player_id.get());
        writer.u16(self.team_id.get());
        writer.u8(self.outcome as u8);
        writer.i16(self.rating_delta);
        writer.u32(self.score);
        encode_count(writer, self.banked_loot.len(), "banked loot")?;
        for loot in &self.banked_loot {
            writer.u32(loot.item_id);
            writer.u32(loot.quantity);
        }
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let player_id = PlayerId::new(reader.u16()?)?;
        let team_id = TeamId::new(reader.u16()?)?;
        let outcome = MatchOutcome::from_wire(reader.u8()?)?;
        let rating_delta = reader.i16()?;
        let score = reader.u32()?;
        let count = decode_count(reader, "banked loot", MAX_LOOT_ENTRIES)?;
        let mut banked_loot = Vec::with_capacity(count);
        for _ in 0..count {
            banked_loot.push(LootEntry {
                item_id: reader.u32()?,
                quantity: reader.u32()?,
            });
        }
        let result = Self {
            player_id,
            team_id,
            outcome,
            rating_delta,
            score,
            banked_loot,
        };
        result.validate()?;
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchResult {
    /// Globally unique idempotency key for settlement.
    pub result_id: [u8; 16],
    pub match_id: [u8; 16],
    pub completed_tick: u64,
    pub players: Vec<PlayerMatchResult>,
}

impl MatchResult {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.result_id.iter().all(|value| *value == 0) {
            return Err(ValidationError::InvalidMatchIdentifier("result_id"));
        }
        if self.match_id.iter().all(|value| *value == 0) {
            return Err(ValidationError::InvalidMatchIdentifier("match_id"));
        }
        validate_non_empty("match result players", &self.players)?;
        validate_count("match result players", self.players.len(), MAX_PLAYERS)?;
        let mut player_ids = BTreeSet::new();
        for player in &self.players {
            player.validate()?;
            if !player_ids.insert(player.player_id) {
                return Err(ValidationError::DuplicateResultPlayer(
                    player.player_id.get(),
                ));
            }
        }
        Ok(())
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        self.validate()?;
        writer.bytes(&self.result_id);
        writer.bytes(&self.match_id);
        writer.u64(self.completed_tick);
        encode_count(writer, self.players.len(), "match result players")?;
        for player in &self.players {
            player.encode(writer)?;
        }
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        let result_id = reader.fixed::<16>()?;
        let match_id = reader.fixed::<16>()?;
        let completed_tick = reader.u64()?;
        let count = decode_count(reader, "match result players", MAX_PLAYERS)?;
        if count == 0 {
            return Err(ValidationError::EmptyCollection("match result players").into());
        }
        let mut players = Vec::with_capacity(count);
        for _ in 0..count {
            players.push(PlayerMatchResult::decode(reader)?);
        }
        let result = Self {
            result_id,
            match_id,
            completed_tick,
            players,
        };
        result.validate()?;
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    InputBatch(InputBatch),
    SnapshotDelta(SnapshotDelta),
    SnapshotKeyframe(SnapshotKeyframe),
    EventBatch(EventBatch),
    TerrainDelta(TerrainDelta),
    MatchResult(MatchResult),
}

impl Message {
    pub const fn kind(&self) -> MessageKind {
        match self {
            Self::InputBatch(_) => MessageKind::InputBatch,
            Self::SnapshotDelta(_) => MessageKind::SnapshotDelta,
            Self::SnapshotKeyframe(_) => MessageKind::SnapshotKeyframe,
            Self::EventBatch(_) => MessageKind::EventBatch,
            Self::TerrainDelta(_) => MessageKind::TerrainDelta,
            Self::MatchResult(_) => MessageKind::MatchResult,
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::InputBatch(value) => value.validate(),
            Self::SnapshotDelta(value) => value.validate(),
            Self::SnapshotKeyframe(value) => value.validate(),
            Self::EventBatch(value) => value.validate(),
            Self::TerrainDelta(value) => value.validate(),
            Self::MatchResult(value) => value.validate(),
        }
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), ProtocolError> {
        match self {
            Self::InputBatch(value) => value.encode(writer),
            Self::SnapshotDelta(value) => value.encode(writer),
            Self::SnapshotKeyframe(value) => value.encode(writer),
            Self::EventBatch(value) => value.encode(writer),
            Self::TerrainDelta(value) => value.encode(writer),
            Self::MatchResult(value) => value.encode(writer),
        }
    }

    fn decode(kind: MessageKind, reader: &mut Reader<'_>) -> Result<Self, ProtocolError> {
        match kind {
            MessageKind::InputBatch => Ok(Self::InputBatch(InputBatch::decode(reader)?)),
            MessageKind::SnapshotDelta => {
                Ok(Self::SnapshotDelta(SnapshotDelta::decode(reader)?))
            }
            MessageKind::SnapshotKeyframe => {
                Ok(Self::SnapshotKeyframe(SnapshotKeyframe::decode(reader)?))
            }
            MessageKind::EventBatch => Ok(Self::EventBatch(EventBatch::decode(reader)?)),
            MessageKind::TerrainDelta => Ok(Self::TerrainDelta(TerrainDelta::decode(reader)?)),
            MessageKind::MatchResult => Ok(Self::MatchResult(MatchResult::decode(reader)?)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageEnvelope {
    pub metadata: EnvelopeMetadata,
    pub message: Message,
}

impl MessageEnvelope {
    pub const fn new(metadata: EnvelopeMetadata, message: Message) -> Self {
        Self { metadata, message }
    }

    pub fn encode_datagram(&self) -> Result<Vec<u8>, ProtocolError> {
        self.encode(Delivery::Datagram)
    }

    pub fn encode_reliable(&self) -> Result<Vec<u8>, ProtocolError> {
        self.encode(Delivery::Reliable)
    }

    pub fn decode_datagram(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > MAX_DATAGRAM_BYTES {
            return Err(ProtocolError::DatagramTooLarge {
                len: bytes.len(),
                max: MAX_DATAGRAM_BYTES,
            });
        }
        Self::decode(bytes, Delivery::Datagram)
    }

    pub fn decode_reliable(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() > MAX_RELIABLE_FRAME_BYTES {
            return Err(ProtocolError::ReliableFrameTooLarge {
                len: bytes.len(),
                max: MAX_RELIABLE_FRAME_BYTES,
            });
        }
        Self::decode(bytes, Delivery::Reliable)
    }

    fn encode(&self, delivery: Delivery) -> Result<Vec<u8>, ProtocolError> {
        self.message.validate()?;
        let kind = self.message.kind();
        if !kind.allows(delivery) {
            return Err(ProtocolError::MessageNotAllowedOnDelivery {
                kind: kind as u8,
                delivery: delivery as u8,
            });
        }

        let mut payload = Writer::with_capacity(256);
        self.message.encode(&mut payload)?;
        let payload = payload.into_inner();
        let payload_len = u32::try_from(payload.len())
            .map_err(|_| ProtocolError::NumericOverflow("payload length"))?;

        let mut writer = Writer::with_capacity(HEADER_BYTES + payload.len());
        writer.bytes(&PROTOCOL_MAGIC);
        writer.u16(PROTOCOL_VERSION);
        writer.u16(HEADER_BYTES as u16);
        writer.u8(kind as u8);
        writer.u8(delivery as u8);
        writer.u16(0);
        writer.bytes(&self.metadata.content_build_hash);
        writer.u64(self.metadata.match_epoch);
        writer.u32(self.metadata.sequence);
        writer.u64(self.metadata.server_tick);
        writer.u64(self.metadata.acknowledgement_tick);
        writer.u32(payload_len);
        writer.bytes(&payload);
        let encoded = writer.into_inner();

        match delivery {
            Delivery::Datagram if encoded.len() > MAX_DATAGRAM_BYTES => {
                Err(ProtocolError::DatagramTooLarge {
                    len: encoded.len(),
                    max: MAX_DATAGRAM_BYTES,
                })
            }
            Delivery::Reliable if encoded.len() > MAX_RELIABLE_FRAME_BYTES => {
                Err(ProtocolError::ReliableFrameTooLarge {
                    len: encoded.len(),
                    max: MAX_RELIABLE_FRAME_BYTES,
                })
            }
            _ => Ok(encoded),
        }
    }

    fn decode(bytes: &[u8], expected_delivery: Delivery) -> Result<Self, ProtocolError> {
        let mut reader = Reader::new(bytes);
        let magic = reader.fixed::<4>()?;
        if magic != PROTOCOL_MAGIC {
            return Err(ProtocolError::InvalidMagic(magic));
        }
        let version = reader.u16()?;
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(version));
        }
        let header_length = reader.u16()?;
        if header_length as usize != HEADER_BYTES {
            return Err(ProtocolError::InvalidHeaderLength(header_length));
        }
        let kind = MessageKind::from_wire(reader.u8()?)?;
        let actual_delivery = Delivery::from_wire(reader.u8()?)?;
        if actual_delivery != expected_delivery {
            return Err(ProtocolError::WrongDelivery {
                expected: expected_delivery as u8,
                actual: actual_delivery as u8,
            });
        }
        if !kind.allows(actual_delivery) {
            return Err(ProtocolError::MessageNotAllowedOnDelivery {
                kind: kind as u8,
                delivery: actual_delivery as u8,
            });
        }
        let reserved_flags = reader.u16()?;
        if reserved_flags != 0 {
            return Err(ProtocolError::NonZeroReservedFlags(reserved_flags));
        }
        let content_build_hash = reader.fixed::<16>()?;
        let match_epoch = reader.u64()?;
        let sequence = reader.u32()?;
        let server_tick = reader.u64()?;
        let acknowledgement_tick = reader.u64()?;
        let declared_payload_len = reader.u32()? as usize;
        let actual_payload_len = reader.remaining();
        if declared_payload_len != actual_payload_len {
            return Err(ProtocolError::PayloadLengthMismatch {
                declared: declared_payload_len,
                actual: actual_payload_len,
            });
        }
        let payload_bytes = reader.take(declared_payload_len)?;
        reader.finish()?;
        let mut payload_reader = Reader::new(payload_bytes);
        let message = Message::decode(kind, &mut payload_reader)?;
        payload_reader.finish()?;
        message.validate()?;
        Ok(Self {
            metadata: EnvelopeMetadata {
                content_build_hash,
                match_epoch,
                sequence,
                server_tick,
                acknowledgement_tick,
            },
            message,
        })
    }
}

fn encode_count(
    writer: &mut Writer,
    count: usize,
    field: &'static str,
) -> Result<(), ProtocolError> {
    writer.u16(
        u16::try_from(count).map_err(|_| ProtocolError::NumericOverflow(field))?,
    );
    Ok(())
}

fn decode_count(
    reader: &mut Reader<'_>,
    field: &'static str,
    max: usize,
) -> Result<usize, ProtocolError> {
    let count = reader.u16()? as usize;
    validate_count(field, count, max)?;
    Ok(count)
}

fn validate_count(
    field: &'static str,
    count: usize,
    max: usize,
) -> Result<(), ValidationError> {
    if count > max {
        Err(ValidationError::TooManyItems { field, count, max })
    } else {
        Ok(())
    }
}

fn validate_non_empty<T>(field: &'static str, values: &[T]) -> Result<(), ValidationError> {
    if values.is_empty() {
        Err(ValidationError::EmptyCollection(field))
    } else {
        Ok(())
    }
}

fn validate_snapshot_id(snapshot_id: SnapshotId) -> Result<(), ValidationError> {
    if snapshot_id.is_none() {
        Err(ValidationError::InvalidSnapshotId(snapshot_id.get()))
    } else {
        Ok(())
    }
}

fn validate_unique_entity_states(entities: &[EntityState]) -> Result<(), ValidationError> {
    let mut ids = BTreeSet::new();
    for entity in entities {
        if !ids.insert(entity.entity_id) {
            return Err(ValidationError::DuplicateEntity(entity.entity_id.get()));
        }
    }
    Ok(())
}

fn validate_unique_entities(
    entities: &[EntityState],
    removed_entities: &[EntityId],
) -> Result<(), ValidationError> {
    let mut ids = BTreeSet::new();
    for entity in entities {
        if !ids.insert(entity.entity_id) {
            return Err(ValidationError::DuplicateEntity(entity.entity_id.get()));
        }
    }
    for removed in removed_entities {
        if !ids.insert(*removed) {
            return Err(ValidationError::DuplicateEntity(removed.get()));
        }
    }
    Ok(())
}

fn decode_optional_player(raw: u16) -> Result<Option<PlayerId>, ProtocolError> {
    if raw == NONE_PLAYER_OR_TEAM {
        Ok(None)
    } else {
        Ok(Some(PlayerId::new(raw)?))
    }
}

fn decode_optional_team(raw: u16) -> Result<Option<TeamId>, ProtocolError> {
    if raw == NONE_PLAYER_OR_TEAM {
        Ok(None)
    } else {
        Ok(Some(TeamId::new(raw)?))
    }
}

fn decode_optional_entity(raw: u64) -> Result<Option<EntityId>, ProtocolError> {
    if raw == NONE_ENTITY {
        Ok(None)
    } else {
        Ok(Some(EntityId::from_raw(raw)?))
    }
}
