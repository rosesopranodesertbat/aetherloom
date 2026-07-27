use alloc::vec::Vec;
use core::fmt;

use aetherloom_protocol::{Controller, EntityId, PlayerId, TeamId};

use crate::codec::{checksum64, Reader, Writer};
use crate::entity::{Entity, EntityKind, EntityPool};
use crate::rng::DeterministicRng;
use crate::state::{
    Inventory, MatchConfig, MatchState, PlayerOutcome, PlayerState, Pose, PoseFrame,
    WorldSeed, POSE_HISTORY_TICKS,
};
use crate::terrain::{validate_chunks, ChunkCoord, TerrainChunk, TERRAIN_CELLS};

pub const CHECKPOINT_MAGIC: [u8; 4] = *b"ALCP";
pub const CHECKPOINT_VERSION: u16 = 1;
const HEADER_BYTES: usize = 20;
const MAX_CHECKPOINT_BYTES: usize = 64 * 1024 * 1024;
const NONE_U16: u16 = u16::MAX;
const NONE_ENTITY: u64 = 0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoritativeCheckpoint {
    version: u16,
    payload: Vec<u8>,
}

impl AuthoritativeCheckpoint {
    pub(crate) fn from_state(state: &MatchState) -> Self {
        Self {
            version: CHECKPOINT_VERSION,
            payload: encode_authoritative_payload(state, true),
        }
    }

    pub const fn version(&self) -> u16 {
        self.version
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(HEADER_BYTES + self.payload.len());
        writer.raw(&CHECKPOINT_MAGIC);
        writer.u16(self.version);
        writer.u16(0);
        writer.u32(self.payload.len() as u32);
        writer.u64(checksum64(&self.payload));
        writer.raw(&self.payload);
        writer.finish()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CheckpointError> {
        if bytes.len() > MAX_CHECKPOINT_BYTES {
            return Err(CheckpointError::TooLarge);
        }
        let mut reader = Reader::new(bytes);
        if reader.exact_bytes(4)? != CHECKPOINT_MAGIC {
            return Err(CheckpointError::BadMagic);
        }
        let version = reader.u16()?;
        if version != CHECKPOINT_VERSION {
            return Err(CheckpointError::UnsupportedVersion(version));
        }
        if reader.u16()? != 0 {
            return Err(CheckpointError::InvalidValue("checkpoint header flags"));
        }
        let payload_len = reader.u32()? as usize;
        if payload_len > MAX_CHECKPOINT_BYTES - HEADER_BYTES {
            return Err(CheckpointError::TooLarge);
        }
        let expected_checksum = reader.u64()?;
        let payload = reader.exact_bytes(payload_len)?.to_vec();
        reader.finish()?;
        if checksum64(&payload) != expected_checksum {
            return Err(CheckpointError::ChecksumMismatch);
        }
        // Fully validate at the trust boundary rather than deferring until use.
        let checkpoint = Self { version, payload };
        let _ = checkpoint.restore_state()?;
        Ok(checkpoint)
    }

    pub(crate) fn restore_state(&self) -> Result<MatchState, CheckpointError> {
        if self.version != CHECKPOINT_VERSION {
            return Err(CheckpointError::UnsupportedVersion(self.version));
        }
        decode_authoritative_payload(&self.payload)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckpointError {
    Truncated,
    TrailingBytes,
    TooLarge,
    BadMagic,
    UnsupportedVersion(u16),
    ChecksumMismatch,
    InvalidValue(&'static str),
    CountOverflow(&'static str),
    InvalidEntity,
    InvalidTerrain,
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

pub(crate) fn encode_authoritative_payload(
    state: &MatchState,
    include_replication_cursor: bool,
) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.u16(state.config.max_players);
    writer.u32(state.config.max_entities);
    writer.u32(state.config.max_terrain_chunks);
    writer.u16(state.config.snapshot_history_capacity);
    writer.u64(state.seed.get());
    writer.u64(state.tick);
    writer.u64(state.generation_rng.state());
    writer.u64(state.ai_rng.state());
    writer.u64(state.combat_rng.state());
    writer.u64(state.environment_rng.state());
    writer.u64(state.next_event_id);
    if include_replication_cursor {
        writer.u32(state.next_snapshot_id);
    }

    writer.u16(state.players.len() as u16);
    for player in &state.players {
        writer.u16(player.id.get());
        writer.u16(player.team.map(TeamId::get).unwrap_or(NONE_U16));
        writer.u8(player.controller as u8);
        writer.u64(player.entity_id.map(EntityId::get).unwrap_or(NONE_ENTITY));
        for value in player.position_cm {
            writer.i32(value);
        }
        for value in player.velocity_cm_per_tick {
            writer.i16(value);
        }
        writer.u16(player.yaw);
        writer.u16(player.health);
        writer.u16(player.cooldown_ticks);
        for value in player.inventory.loadout {
            writer.u16(value);
        }
        for value in player.inventory.consumables {
            writer.u16(value);
        }
        writer.u32(player.inventory.unbanked_resources);
        writer.u32(player.inventory.banked_resources);
        writer.u8(player.outcome as u8);
        writer.bool(player.last_sequence.is_some());
        writer.u32(player.last_sequence.unwrap_or(0));
        writer.u32(player.bot_next_sequence);
    }

    writer.u32(state.entities.capacity());
    writer.u32(state.entities.slot_generations().count() as u32);
    for generation in state.entities.slot_generations() {
        writer.u32(generation);
    }
    writer.u32(state.entities.dense_entities().len() as u32);
    for entity in state.entities.dense_entities() {
        encode_entity(&mut writer, entity);
    }
    writer.u32(state.entities.free_slots().len() as u32);
    for index in state.entities.free_slots() {
        writer.u32(*index);
    }

    writer.u8(state.pose_history.len() as u8);
    for frame in &state.pose_history {
        writer.u64(frame.tick);
        writer.u16(frame.poses.len() as u16);
        for pose in &frame.poses {
            writer.u64(pose.entity_id.get());
            for value in pose.position_cm {
                writer.i32(value);
            }
            writer.u16(pose.yaw);
            writer.u16(pose.health);
        }
    }

    writer.u32(state.terrain.len() as u32);
    for chunk in &state.terrain {
        writer.i32(chunk.coord.x);
        writer.i32(chunk.coord.y);
        writer.u32(chunk.revision);
        writer.u16(chunk.heights_cm.len() as u16);
        for height in &chunk.heights_cm {
            writer.i16(*height);
        }
    }
    writer.finish()
}

fn decode_authoritative_payload(payload: &[u8]) -> Result<MatchState, CheckpointError> {
    let mut reader = Reader::new(payload);
    let max_players = reader.u16()?;
    let max_entities = reader.u32()?;
    let max_terrain_chunks = reader.u32()?;
    let snapshot_history_capacity = reader.u16()?;
    let mut config = MatchConfig::new(max_players, max_entities, max_terrain_chunks)
        .map_err(|_| CheckpointError::InvalidValue("match config"))?;
    config = config
        .with_snapshot_history_capacity(snapshot_history_capacity)
        .map_err(|_| CheckpointError::InvalidValue("snapshot history capacity"))?;
    let seed = WorldSeed::new(reader.u64()?);
    let tick = reader.u64()?;
    let generation_rng = DeterministicRng::from_state(reader.u64()?);
    let ai_rng = DeterministicRng::from_state(reader.u64()?);
    let combat_rng = DeterministicRng::from_state(reader.u64()?);
    let environment_rng = DeterministicRng::from_state(reader.u64()?);
    let next_event_id = reader.u64()?;
    if next_event_id == 0 {
        return Err(CheckpointError::InvalidValue("next event id"));
    }
    let next_snapshot_id = reader.u32()?;
    if next_snapshot_id == 0 {
        return Err(CheckpointError::InvalidValue("next snapshot id"));
    }

    let player_count = reader.u16()? as usize;
    if player_count != max_players as usize {
        return Err(CheckpointError::InvalidValue("player slot count"));
    }
    let mut players = reserved_vec(player_count, "player slots")?;
    for expected_index in 0..player_count {
        let id = PlayerId::new(reader.u16()?)
            .map_err(|_| CheckpointError::InvalidValue("player id"))?;
        if id.index() != expected_index {
            return Err(CheckpointError::InvalidValue("player slot order"));
        }
        let team_raw = reader.u16()?;
        let team = if team_raw == NONE_U16 {
            None
        } else {
            Some(
                TeamId::new(team_raw)
                    .map_err(|_| CheckpointError::InvalidValue("team id"))?,
            )
        };
        let controller = Controller::from_wire(reader.u8()?)
            .map_err(|_| CheckpointError::InvalidValue("controller"))?;
        let entity_raw = reader.u64()?;
        let entity_id = if entity_raw == NONE_ENTITY {
            None
        } else {
            Some(
                EntityId::from_raw(entity_raw)
                    .map_err(|_| CheckpointError::InvalidValue("player entity id"))?,
            )
        };
        let position_cm = [reader.i32()?, reader.i32()?, reader.i32()?];
        let velocity_cm_per_tick = [reader.i16()?, reader.i16()?, reader.i16()?];
        let yaw = reader.u16()?;
        let health = reader.u16()?;
        let cooldown_ticks = reader.u16()?;
        let mut loadout = [0_u16; 4];
        for value in &mut loadout {
            *value = reader.u16()?;
        }
        let mut consumables = [0_u16; 4];
        for value in &mut consumables {
            *value = reader.u16()?;
        }
        let inventory = Inventory {
            loadout,
            consumables,
            unbanked_resources: reader.u32()?,
            banked_resources: reader.u32()?,
        };
        let outcome = PlayerOutcome::from_u8(reader.u8()?)?;
        let has_sequence = reader.bool()?;
        let sequence = reader.u32()?;
        let bot_next_sequence = reader.u32()?;
        if (has_sequence && sequence == 0)
            || (!has_sequence && sequence != 0)
            || bot_next_sequence == 0
        {
            return Err(CheckpointError::InvalidValue("player command sequence"));
        }
        let last_sequence = has_sequence.then_some(sequence);
        if controller == Controller::Bot
            && bot_next_sequence != last_sequence.map(next_nonzero_sequence).unwrap_or(1)
        {
            return Err(CheckpointError::InvalidValue("bot command sequence"));
        }
        if controller == Controller::Empty && (team.is_some() || entity_id.is_some()) {
            return Err(CheckpointError::InvalidValue("occupied empty player slot"));
        }
        if controller != Controller::Empty && (team.is_none() || entity_id.is_none()) {
            return Err(CheckpointError::InvalidValue("active player slot incomplete"));
        }
        players.push(PlayerState {
            id,
            team,
            controller,
            entity_id,
            position_cm,
            velocity_cm_per_tick,
            yaw,
            health,
            cooldown_ticks,
            inventory,
            outcome,
            last_sequence,
            bot_next_sequence,
        });
    }

    let pool_capacity = reader.u32()?;
    if pool_capacity != max_entities {
        return Err(CheckpointError::InvalidValue("entity pool capacity"));
    }
    let slot_count = checked_count_with_bytes(
        reader.u32()?,
        max_entities,
        reader.remaining(),
        4,
        "entity slot count",
    )?;
    let mut generations = reserved_vec(slot_count, "entity generations")?;
    for _ in 0..slot_count {
        let generation = reader.u32()?;
        if generation == 0 {
            return Err(CheckpointError::InvalidValue("zero entity generation"));
        }
        generations.push(generation);
    }
    let dense_count = checked_count_with_bytes(
        reader.u32()?,
        max_entities,
        reader.remaining(),
        41,
        "dense entity count",
    )?;
    let mut dense = reserved_vec(dense_count, "dense entities")?;
    for _ in 0..dense_count {
        dense.push(decode_entity(&mut reader)?);
    }
    let free_count = checked_count_with_bytes(
        reader.u32()?,
        max_entities,
        reader.remaining(),
        4,
        "free entity count",
    )?;
    let mut free = reserved_vec(free_count, "free entity slots")?;
    for _ in 0..free_count {
        free.push(reader.u32()?);
    }
    let entities = EntityPool::restore_parts(pool_capacity, generations, dense, free)
        .map_err(|_| CheckpointError::InvalidEntity)?;
    for entity in entities.iter() {
        if entity
            .owner
            .is_some_and(|owner| owner.index() >= players.len())
        {
            return Err(CheckpointError::InvalidEntity);
        }
        if entity.kind == EntityKind::Player {
            let Some(owner) = entity.owner else {
                return Err(CheckpointError::InvalidEntity);
            };
            if players[owner.index()].entity_id != Some(entity.id) {
                return Err(CheckpointError::InvalidEntity);
            }
        }
    }
    for player in &players {
        if let Some(entity_id) = player.entity_id {
            let Some(entity) = entities.get(entity_id) else {
                return Err(CheckpointError::InvalidEntity);
            };
            if entity.kind != EntityKind::Player
                || entity.owner != Some(player.id)
                || entity.team != player.team
                || entity.position_cm != player.position_cm
                || entity.velocity_cm_per_tick != player.velocity_cm_per_tick
                || entity.yaw != player.yaw
                || entity.health != player.health
            {
                return Err(CheckpointError::InvalidEntity);
            }
        }
    }

    let pose_frame_count = reader.u8()? as usize;
    if pose_frame_count > POSE_HISTORY_TICKS {
        return Err(CheckpointError::CountOverflow("pose history frames"));
    }
    let mut pose_history = reserved_vec(pose_frame_count, "pose history frames")?;
    let mut previous_tick = None;
    for _ in 0..pose_frame_count {
        let pose_tick = reader.u64()?;
        if previous_tick.is_some_and(|previous| previous >= pose_tick) || pose_tick >= tick {
            return Err(CheckpointError::InvalidValue("pose history tick order"));
        }
        previous_tick = Some(pose_tick);
        let pose_count = reader.u16()? as usize;
        if pose_count > max_players as usize {
            return Err(CheckpointError::CountOverflow("pose count"));
        }
        let mut poses = reserved_vec(pose_count, "pose history entries")?;
        let mut previous_id = None;
        for _ in 0..pose_count {
            let entity_id =
                EntityId::from_raw(reader.u64()?).map_err(|_| CheckpointError::InvalidEntity)?;
            if previous_id.is_some_and(|previous| previous >= entity_id) {
                return Err(CheckpointError::InvalidValue("pose entity order"));
            }
            previous_id = Some(entity_id);
            poses.push(Pose {
                entity_id,
                position_cm: [reader.i32()?, reader.i32()?, reader.i32()?],
                yaw: reader.u16()?,
                health: reader.u16()?,
            });
        }
        pose_history.push(PoseFrame {
            tick: pose_tick,
            poses,
        });
    }

    let chunk_count = checked_count_with_bytes(
        reader.u32()?,
        max_terrain_chunks,
        reader.remaining(),
        14 + TERRAIN_CELLS * 2,
        "terrain chunk count",
    )?;
    let mut terrain = reserved_vec(chunk_count, "terrain chunks")?;
    for _ in 0..chunk_count {
        let coord = ChunkCoord {
            x: reader.i32()?,
            y: reader.i32()?,
        };
        let revision = reader.u32()?;
        if revision == 0 {
            return Err(CheckpointError::InvalidTerrain);
        }
        let cell_count = reader.u16()? as usize;
        if cell_count != TERRAIN_CELLS {
            return Err(CheckpointError::InvalidTerrain);
        }
        let mut heights_cm = reserved_vec(cell_count, "terrain cells")?;
        for _ in 0..cell_count {
            heights_cm.push(reader.i16()?);
        }
        terrain.push(TerrainChunk {
            coord,
            revision,
            heights_cm,
        });
    }
    validate_chunks(&terrain).map_err(|_| CheckpointError::InvalidTerrain)?;
    reader.finish()?;

    Ok(MatchState {
        config,
        seed,
        tick,
        players,
        entities,
        terrain,
        generation_rng,
        ai_rng,
        combat_rng,
        environment_rng,
        next_event_id,
        pose_history,
        next_snapshot_id,
        snapshot_history: Vec::new(),
    })
}

fn encode_entity(writer: &mut Writer, entity: &Entity) {
    writer.u64(entity.id.get());
    writer.u8(entity.kind as u8);
    writer.u16(entity.owner.map(PlayerId::get).unwrap_or(NONE_U16));
    writer.u16(entity.team.map(TeamId::get).unwrap_or(NONE_U16));
    for value in entity.position_cm {
        writer.i32(value);
    }
    for value in entity.velocity_cm_per_tick {
        writer.i16(value);
    }
    writer.u16(entity.yaw);
    writer.i16(entity.pitch);
    writer.u16(entity.health);
    writer.u16(entity.flags);
    writer.u16(entity.lifetime_ticks);
}

fn decode_entity(reader: &mut Reader<'_>) -> Result<Entity, CheckpointError> {
    let id = EntityId::from_raw(reader.u64()?).map_err(|_| CheckpointError::InvalidEntity)?;
    let kind = EntityKind::from_u8(reader.u8()?).map_err(|_| CheckpointError::InvalidEntity)?;
    let owner_raw = reader.u16()?;
    let owner = if owner_raw == NONE_U16 {
        None
    } else {
        Some(
            PlayerId::new(owner_raw).map_err(|_| CheckpointError::InvalidEntity)?,
        )
    };
    let team_raw = reader.u16()?;
    let team = if team_raw == NONE_U16 {
        None
    } else {
        Some(TeamId::new(team_raw).map_err(|_| CheckpointError::InvalidEntity)?)
    };
    Ok(Entity {
        id,
        kind,
        owner,
        team,
        position_cm: [reader.i32()?, reader.i32()?, reader.i32()?],
        velocity_cm_per_tick: [reader.i16()?, reader.i16()?, reader.i16()?],
        yaw: reader.u16()?,
        pitch: reader.i16()?,
        health: reader.u16()?,
        flags: reader.u16()?,
        lifetime_ticks: reader.u16()?,
    })
}

fn checked_count(
    value: u32,
    maximum: u32,
    field: &'static str,
) -> Result<usize, CheckpointError> {
    if value > maximum {
        Err(CheckpointError::CountOverflow(field))
    } else {
        Ok(value as usize)
    }
}

fn checked_count_with_bytes(
    value: u32,
    maximum: u32,
    remaining_bytes: usize,
    minimum_item_bytes: usize,
    field: &'static str,
) -> Result<usize, CheckpointError> {
    let count = checked_count(value, maximum, field)?;
    let minimum_bytes = count
        .checked_mul(minimum_item_bytes)
        .ok_or(CheckpointError::CountOverflow(field))?;
    if minimum_bytes > remaining_bytes {
        return Err(CheckpointError::Truncated);
    }
    Ok(count)
}

fn reserved_vec<T>(
    capacity: usize,
    field: &'static str,
) -> Result<Vec<T>, CheckpointError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| CheckpointError::CountOverflow(field))?;
    Ok(values)
}

fn next_nonzero_sequence(previous: u32) -> u32 {
    match previous.wrapping_add(1) {
        0 => 1,
        next => next,
    }
}
