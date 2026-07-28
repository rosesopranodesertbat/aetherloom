use alloc::vec::Vec;
use core::fmt;

use aetherloom_protocol::{
    CommandSet, Controller, EntityId, PlayerCommand, PlayerId, TeamId, ValidationError,
    ACTION_CAST, ACTION_EXTRACT, AUTHORITATIVE_HZ, MAX_MOVE_AXIS, MAX_PLAYERS,
};

use crate::codec::checksum64;
use crate::entity::{Entity, EntityKind, EntityPool, EntityPoolError};
use crate::replication::StoredSnapshot;
use crate::rng::{
    DeterministicRng, AI_STREAM, COMBAT_STREAM, ENVIRONMENT_STREAM, GENERATION_STREAM,
};
use crate::terrain::{
    deform, height_cm_at, world_to_chunk_cell, ChunkCoord, TerrainChunk,
    TerrainDeformationEvent, TerrainError,
};
use crate::{AuthoritativeCheckpoint, CheckpointError, SnapshotId};

pub const PLAYER_PLANAR_SPEED_CM_PER_TICK: i32 = 10;
pub const PLAYER_VERTICAL_SPEED_CM_PER_TICK: i32 = 6;
pub const PLAYER_MIN_ALTITUDE_CM: i32 = 0;
pub const PLAYER_MAX_ALTITUDE_CM: i32 = 4_300;
/// The browser flight model preserves full forward thrust while reducing
/// strafe to 55%, so its intended diagonal is slightly longer than one axis.
/// Cap arbitrary/custom clients at that same deterministic envelope.
pub const PLAYER_MAX_PLANAR_INPUT_MAGNITUDE: i32 = 2_338;
const PLAYER_HEALTH: u16 = 100;
const FIREBOLT_SPELL_ID: u8 = 0;
const MEND_SPELL_ID: u8 = 7;
const FIREBOLT_COOLDOWN_TICKS: u16 = 36;
const MEND_COOLDOWN_TICKS: u16 = 896;
const MEND_HEALTH: u16 = 55;
pub(crate) const SPELL_SLOT_COUNT: usize = aetherloom_protocol::SPELL_COOLDOWN_SLOTS;
const PROJECTILE_LIFETIME_TICKS: u16 = 410;
const PROJECTILE_SPEED_CM_PER_SECOND: i32 = 2_200;
const PROJECTILE_MUZZLE_FORWARD_CM: i32 = 14;
const PROJECTILE_MUZZLE_HEIGHT_CM: i32 = 23;
const PROJECTILE_HIT_RADIUS_CM: i64 = 75;
/// Hard allocation ceiling for any match configuration or decoded checkpoint.
pub const MAX_MATCH_ENTITIES: u32 = 65_536;
/// Hard allocation ceiling for authoritative terrain chunks.
pub const MAX_MATCH_TERRAIN_CHUNKS: u32 = 16_384;
/// Per-viewer baseline history is deliberately bounded.
pub const MAX_SNAPSHOT_HISTORY_CAPACITY: u16 = 64;
/// `ceil(0.256 * 128)`: the authoritative rewind ring covers at least 256 ms.
pub const POSE_HISTORY_TICKS: usize = 33;
/// `floor(0.200 * 128)`: targeting rewind is never permitted beyond 200 ms.
pub const MAX_REWIND_TICKS: u64 = 25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldSeed(u64);

impl WorldSeed {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchConfig {
    pub(crate) max_players: u16,
    pub(crate) max_entities: u32,
    pub(crate) max_terrain_chunks: u32,
    /// Number of retained baselines for each viewer, not for the whole match.
    pub(crate) snapshot_history_capacity: u16,
}

impl MatchConfig {
    pub fn new(
        max_players: u16,
        max_entities: u32,
        max_terrain_chunks: u32,
    ) -> Result<Self, CoreError> {
        if max_players == 0 || max_players as usize > MAX_PLAYERS {
            return Err(CoreError::InvalidConfig("max_players must be in 1..=128"));
        }
        if max_entities < max_players as u32 || max_entities > MAX_MATCH_ENTITIES {
            return Err(CoreError::InvalidConfig(
                "max_entities must fit every player and stay within the allocation ceiling",
            ));
        }
        if max_terrain_chunks == 0 || max_terrain_chunks > MAX_MATCH_TERRAIN_CHUNKS {
            return Err(CoreError::InvalidConfig(
                "max_terrain_chunks must be non-zero and stay within the allocation ceiling",
            ));
        }
        Ok(Self {
            max_players,
            max_entities,
            max_terrain_chunks,
            snapshot_history_capacity: 4,
        })
    }

    pub const fn competitive() -> Self {
        Self {
            max_players: MAX_PLAYERS as u16,
            max_entities: 8_192,
            max_terrain_chunks: 4_096,
            snapshot_history_capacity: 4,
        }
    }

    pub fn with_snapshot_history_capacity(mut self, capacity: u16) -> Result<Self, CoreError> {
        if capacity == 0 || capacity > MAX_SNAPSHOT_HISTORY_CAPACITY {
            return Err(CoreError::InvalidConfig(
                "snapshot history capacity must be within the per-viewer ceiling",
            ));
        }
        self.snapshot_history_capacity = capacity;
        Ok(self)
    }

    pub const fn max_players(self) -> u16 {
        self.max_players
    }

    pub const fn max_entities(self) -> u32 {
        self.max_entities
    }

    pub const fn max_terrain_chunks(self) -> u32 {
        self.max_terrain_chunks
    }

    pub const fn snapshot_history_capacity(self) -> u16 {
        self.snapshot_history_capacity
    }
}

impl Default for MatchConfig {
    fn default() -> Self {
        Self::competitive()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    /// Reserved loadout item identifiers. Zero denotes an empty slot.
    pub loadout: [u16; 4],
    pub consumables: [u16; 4],
    pub unbanked_resources: u32,
    pub banked_resources: u32,
}

/// An explicit deterministic player placement.
///
/// Match hosts use this when a mode owns its spawn layout. The legacy
/// [`MatchState::add_player`] path intentionally retains its seeded random
/// placement and consumes the same generation-RNG draws as before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerSpawn {
    position_cm: [i32; 3],
    yaw: u16,
}

impl PlayerSpawn {
    pub const fn new(position_cm: [i32; 3], yaw: u16) -> Self {
        Self { position_cm, yaw }
    }

    pub const fn position_cm(self) -> [i32; 3] {
        self.position_cm
    }

    pub const fn yaw(self) -> u16 {
        self.yaw
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PlayerOutcome {
    Active = 1,
    Extracted = 2,
    Defeated = 3,
    Disconnected = 4,
    Abandoned = 5,
}

impl PlayerOutcome {
    pub(crate) fn from_u8(value: u8) -> Result<Self, CheckpointError> {
        match value {
            1 => Ok(Self::Active),
            2 => Ok(Self::Extracted),
            3 => Ok(Self::Defeated),
            4 => Ok(Self::Disconnected),
            5 => Ok(Self::Abandoned),
            _ => Err(CheckpointError::InvalidValue("player outcome")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerState {
    pub id: PlayerId,
    pub team: Option<TeamId>,
    pub controller: Controller,
    pub entity_id: Option<EntityId>,
    pub position_cm: [i32; 3],
    pub velocity_cm_per_tick: [i16; 3],
    pub yaw: u16,
    pub pitch: i16,
    pub health: u16,
    pub spell_cooldown_ticks: [u16; SPELL_SLOT_COUNT],
    pub inventory: Inventory,
    pub outcome: PlayerOutcome,
    pub(crate) last_sequence: Option<u32>,
    pub(crate) bot_next_sequence: u32,
}

impl PlayerState {
    fn empty(id: PlayerId) -> Self {
        Self {
            id,
            team: None,
            controller: Controller::Empty,
            entity_id: None,
            position_cm: [0; 3],
            velocity_cm_per_tick: [0; 3],
            yaw: 0,
            pitch: 0,
            health: 0,
            spell_cooldown_ticks: [0; SPELL_SLOT_COUNT],
            inventory: Inventory::default(),
            outcome: PlayerOutcome::Defeated,
            last_sequence: None,
            bot_next_sequence: 1,
        }
    }

    pub const fn last_accepted_sequence(&self) -> Option<u32> {
        self.last_sequence
    }

    pub fn cooldown_ticks(&self, spell: u8) -> Option<u16> {
        self.spell_cooldown_ticks.get(spell as usize).copied()
    }

    pub fn max_cooldown_ticks(&self) -> u16 {
        self.spell_cooldown_ticks.iter().copied().max().unwrap_or(0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RngState(u64);

impl RngState {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RngStreams {
    pub generation: RngState,
    pub ai: RngState,
    pub combat: RngState,
    pub environment: RngState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TickEventKind {
    PlayerJoined = 1,
    PlayerLeft = 2,
    Cast = 3,
    Damage = 4,
    Defeat = 5,
    Extraction = 6,
    EntitySpawned = 7,
    EntityDespawned = 8,
    TerrainDeformed = 9,
    ProjectileImpact = 10,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickEvent {
    pub event_id: u64,
    pub tick: u64,
    pub kind: TickEventKind,
    pub actor: Option<EntityId>,
    pub target: Option<EntityId>,
    pub data: [i32; 4],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TickEvents {
    pub tick: u64,
    pub applied_commands: Vec<PlayerId>,
    pub events: Vec<TickEvent>,
    pub terrain: Vec<TerrainDeformationEvent>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pose {
    pub entity_id: EntityId,
    pub position_cm: [i32; 3],
    pub yaw: u16,
    pub pitch: i16,
    pub health: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoseFrame {
    pub tick: u64,
    pub poses: Vec<Pose>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewindError {
    NoHistory,
    FutureTick { latest: u64, requested: u64 },
    TooOld {
        latest: u64,
        requested: u64,
        maximum_age_ticks: u64,
    },
    MissingTick(u64),
    MissingEntity(EntityId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandRejection {
    StaleTick { current: u64, received: u64 },
    FutureTick { current: u64, received: u64 },
    UnknownPlayer(PlayerId),
    EmptySlot(PlayerId),
    DuplicateSequence { player: PlayerId, sequence: u32 },
    ObsoleteSequence {
        player: PlayerId,
        previous: u32,
        received: u32,
    },
    InvalidShape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreError {
    InvalidConfig(&'static str),
    InvalidPlayer(PlayerId),
    PlayerAlreadyActive(PlayerId),
    PlayerInactive(PlayerId),
    Entity(EntityPoolError),
    Terrain(TerrainError),
    Command(CommandRejection),
    Checkpoint(CheckpointError),
}

impl fmt::Display for CoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl From<EntityPoolError> for CoreError {
    fn from(value: EntityPoolError) -> Self {
        Self::Entity(value)
    }
}

impl From<TerrainError> for CoreError {
    fn from(value: TerrainError) -> Self {
        Self::Terrain(value)
    }
}

impl From<CheckpointError> for CoreError {
    fn from(value: CheckpointError) -> Self {
        Self::Checkpoint(value)
    }
}

/// The complete owned authoritative state for one match.
///
/// This type deliberately contains no renderer, sound, camera, HUD, particle,
/// platform, socket, or wall-clock state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MatchState {
    pub(crate) config: MatchConfig,
    pub(crate) seed: WorldSeed,
    pub(crate) tick: u64,
    pub(crate) players: Vec<PlayerState>,
    pub(crate) entities: EntityPool,
    pub(crate) terrain: Vec<TerrainChunk>,
    pub(crate) generation_rng: DeterministicRng,
    pub(crate) ai_rng: DeterministicRng,
    pub(crate) combat_rng: DeterministicRng,
    pub(crate) environment_rng: DeterministicRng,
    pub(crate) next_event_id: u64,
    pub(crate) pose_history: Vec<PoseFrame>,
    pub(crate) next_snapshot_id: u32,
    pub(crate) snapshot_history: Vec<StoredSnapshot>,
}

impl MatchState {
    pub fn new(config: MatchConfig, seed: WorldSeed) -> Self {
        let mut players = Vec::with_capacity(config.max_players as usize);
        for raw in 0..config.max_players {
            let id = PlayerId::new(raw).expect("validated match player count");
            players.push(PlayerState::empty(id));
        }
        Self {
            config,
            seed,
            tick: 0,
            players,
            entities: EntityPool::new(config.max_entities),
            terrain: Vec::new(),
            generation_rng: DeterministicRng::seeded(seed.get(), GENERATION_STREAM),
            ai_rng: DeterministicRng::seeded(seed.get(), AI_STREAM),
            combat_rng: DeterministicRng::seeded(seed.get(), COMBAT_STREAM),
            environment_rng: DeterministicRng::seeded(seed.get(), ENVIRONMENT_STREAM),
            next_event_id: 1,
            pose_history: Vec::new(),
            next_snapshot_id: 1,
            snapshot_history: Vec::new(),
        }
    }

    pub const fn config(&self) -> MatchConfig {
        self.config
    }

    pub const fn seed(&self) -> WorldSeed {
        self.seed
    }

    /// The next authoritative tick that will be simulated.
    pub const fn tick(&self) -> u64 {
        self.tick
    }

    pub const fn tick_rate_hz(&self) -> u32 {
        AUTHORITATIVE_HZ
    }

    pub fn players(&self) -> &[PlayerState] {
        &self.players
    }

    pub fn player(&self, id: PlayerId) -> Option<&PlayerState> {
        self.players.get(id.index())
    }

    pub fn entities(&self) -> &EntityPool {
        &self.entities
    }

    pub fn terrain(&self) -> &[TerrainChunk] {
        &self.terrain
    }

    pub fn rng_states(&self) -> RngStreams {
        RngStreams {
            generation: RngState(self.generation_rng.state()),
            ai: RngState(self.ai_rng.state()),
            combat: RngState(self.combat_rng.state()),
            environment: RngState(self.environment_rng.state()),
        }
    }

    pub fn add_player(
        &mut self,
        id: PlayerId,
        team: TeamId,
        controller: Controller,
    ) -> Result<EntityId, CoreError> {
        let index = self.validate_player_add(id, controller)?;
        let spawn = PlayerSpawn::new(
            [
                self.generation_rng.range_i32(-8_000, 8_000),
                0,
                self.generation_rng.range_i32(-8_000, 8_000),
            ],
            0,
        );
        self.insert_player_at_spawn(index, id, team, controller, spawn)
    }

    /// Adds a player at an exact host-selected placement without consuming
    /// generation RNG.
    ///
    /// This is intended for deterministic arenas, replay fixtures, and mode
    /// transitions whose spawn geometry is already authoritative.
    pub fn add_player_at_spawn(
        &mut self,
        id: PlayerId,
        team: TeamId,
        controller: Controller,
        spawn: PlayerSpawn,
    ) -> Result<EntityId, CoreError> {
        let index = self.validate_player_add(id, controller)?;
        self.insert_player_at_spawn(index, id, team, controller, spawn)
    }

    fn validate_player_add(
        &self,
        id: PlayerId,
        controller: Controller,
    ) -> Result<usize, CoreError> {
        let index = id.index();
        if index >= self.players.len() {
            return Err(CoreError::InvalidPlayer(id));
        }
        if controller == Controller::Empty {
            return Err(CoreError::InvalidConfig(
                "add_player requires Human or Bot controller",
            ));
        }
        if self.players[index].controller != Controller::Empty {
            return Err(CoreError::PlayerAlreadyActive(id));
        }
        if self.entities.len() >= self.entities.capacity() as usize {
            return Err(CoreError::Entity(EntityPoolError::Capacity));
        }
        Ok(index)
    }

    fn insert_player_at_spawn(
        &mut self,
        index: usize,
        id: PlayerId,
        team: TeamId,
        controller: Controller,
        spawn: PlayerSpawn,
    ) -> Result<EntityId, CoreError> {
        let mut entity = Entity::new(EntityKind::Player);
        entity.owner = Some(id);
        entity.team = Some(team);
        entity.position_cm = spawn.position_cm;
        entity.yaw = spawn.yaw;
        entity.health = PLAYER_HEALTH;
        let entity_id = self.entities.spawn(entity)?;

        self.players[index] = PlayerState {
            id,
            team: Some(team),
            controller,
            entity_id: Some(entity_id),
            position_cm: spawn.position_cm,
            velocity_cm_per_tick: [0; 3],
            yaw: spawn.yaw,
            pitch: 0,
            health: PLAYER_HEALTH,
            spell_cooldown_ticks: [0; SPELL_SLOT_COUNT],
            inventory: Inventory::default(),
            outcome: PlayerOutcome::Active,
            last_sequence: None,
            bot_next_sequence: 1,
        };
        Ok(entity_id)
    }

    /// Restores an active player to a deterministic combat spawn.
    ///
    /// Controller, team, entity identity, inventory, and the accepted command
    /// sequence are preserved. Pose, health, cooldown, and outcome are reset
    /// atomically in both the player slot and its authoritative entity. Only
    /// this entity is removed from pose history, so lag compensation cannot
    /// rewind it through the teleport without disrupting other players.
    pub fn reset_player_at_spawn(
        &mut self,
        id: PlayerId,
        spawn: PlayerSpawn,
    ) -> Result<(), CoreError> {
        let Some(player) = self.players.get(id.index()) else {
            return Err(CoreError::InvalidPlayer(id));
        };
        if player.controller == Controller::Empty {
            return Err(CoreError::PlayerInactive(id));
        }
        let entity_id = player.entity_id.ok_or(CoreError::PlayerInactive(id))?;
        let entity = self
            .entities
            .get(entity_id)
            .ok_or(CoreError::Entity(EntityPoolError::InvalidId))?;
        if entity.kind != EntityKind::Player || entity.owner != Some(id) {
            return Err(CoreError::Entity(EntityPoolError::Corrupt(
                "player slot does not own its player entity",
            )));
        }

        let player = &mut self.players[id.index()];
        player.position_cm = spawn.position_cm;
        player.velocity_cm_per_tick = [0; 3];
        player.yaw = spawn.yaw;
        player.pitch = 0;
        player.health = PLAYER_HEALTH;
        player.spell_cooldown_ticks = [0; SPELL_SLOT_COUNT];
        player.outcome = PlayerOutcome::Active;

        let entity = self
            .entities
            .get_mut(entity_id)
            .ok_or(CoreError::Entity(EntityPoolError::InvalidId))?;
        entity.position_cm = spawn.position_cm;
        entity.velocity_cm_per_tick = [0; 3];
        entity.yaw = spawn.yaw;
        entity.pitch = 0;
        entity.health = PLAYER_HEALTH;
        entity.flags = 0;
        entity.lifetime_ticks = 0;
        for frame in &mut self.pose_history {
            frame.poses.retain(|pose| pose.entity_id != entity_id);
        }
        Ok(())
    }

    pub fn remove_player(&mut self, id: PlayerId) -> Result<(), CoreError> {
        if self.players.get(id.index()).is_none() {
            return Err(CoreError::InvalidPlayer(id));
        }
        self.remove_projectiles_owned_by(id);
        let player = &mut self.players[id.index()];
        if let Some(entity) = player.entity_id {
            let _ = self.entities.remove(entity);
        }
        *player = PlayerState::empty(id);
        Ok(())
    }

    /// Removes every in-flight projectile.
    ///
    /// Round hosts call this before repositioning combatants so an earlier
    /// round cannot damage freshly reset players. This administrative mutation
    /// emits no gameplay event; callers must publish the resulting snapshot.
    pub fn clear_projectiles(&mut self) -> usize {
        let projectile_ids: Vec<EntityId> = self
            .entities
            .active_ids_sorted()
            .into_iter()
            .filter(|id| {
                self.entities
                    .get(*id)
                    .is_some_and(|entity| entity.kind == EntityKind::Projectile)
            })
            .collect();
        let count = projectile_ids.len();
        for projectile_id in projectile_ids {
            let _ = self.entities.remove(projectile_id);
        }
        count
    }

    fn remove_projectiles_owned_by(&mut self, owner: PlayerId) {
        let projectile_ids: Vec<EntityId> = self
            .entities
            .active_ids_sorted()
            .into_iter()
            .filter(|id| {
                self.entities.get(*id).is_some_and(|entity| {
                    entity.kind == EntityKind::Projectile && entity.owner == Some(owner)
                })
            })
            .collect();
        for projectile_id in projectile_ids {
            let _ = self.entities.remove(projectile_id);
        }
    }

    pub fn set_controller(
        &mut self,
        id: PlayerId,
        controller: Controller,
    ) -> Result<(), CoreError> {
        let Some(player) = self.players.get_mut(id.index()) else {
            return Err(CoreError::InvalidPlayer(id));
        };
        if player.controller == Controller::Empty && controller != Controller::Empty {
            return Err(CoreError::InvalidConfig(
                "activate a slot through add_player so it receives an entity",
            ));
        }
        player.controller = controller;
        if controller == Controller::Bot {
            player.bot_next_sequence = player
                .last_sequence
                .map(next_command_sequence)
                .unwrap_or(1);
        }
        if controller == Controller::Empty {
            if let Some(entity) = player.entity_id.take() {
                let _ = self.entities.remove(entity);
            }
            player.team = None;
            player.outcome = PlayerOutcome::Defeated;
        }
        Ok(())
    }

    /// Validates and advances exactly one 7.8125 ms authoritative tick.
    ///
    /// The entire external command set is checked before RNG or gameplay state
    /// can change. Bots then generate `PlayerCommand` values and enter the same
    /// deterministic application path as human commands.
    pub fn advance_tick(&mut self, commands: CommandSet) -> Result<TickEvents, CoreError> {
        if commands.tick() < self.tick {
            return Err(CoreError::Command(CommandRejection::StaleTick {
                current: self.tick,
                received: commands.tick(),
            }));
        }
        if commands.tick() > self.tick {
            return Err(CoreError::Command(CommandRejection::FutureTick {
                current: self.tick,
                received: commands.tick(),
            }));
        }
        self.validate_commands(&commands)?;

        let mut effective = commands;
        for index in 0..self.players.len() {
            let player = self.players[index];
            if player.controller == Controller::Bot
                && player.outcome == PlayerOutcome::Active
                && effective.get(player.id).is_none()
            {
                let command = self.generate_bot_command(player)?;
                effective
                    .insert(player.id, command)
                    .map_err(|_| CoreError::Command(CommandRejection::InvalidShape))?;
            }
        }

        // Missing input always clears transient movement.
        for player in &mut self.players {
            if player.controller != Controller::Empty && effective.get(player.id).is_none() {
                player.velocity_cm_per_tick = [0; 3];
                if let Some(entity_id) = player.entity_id {
                    if let Some(entity) = self.entities.get_mut(entity_id) {
                        entity.velocity_cm_per_tick = [0; 3];
                    }
                }
            }
            for cooldown in &mut player.spell_cooldown_ticks {
                *cooldown = cooldown.saturating_sub(1);
            }
        }

        let mut tick_events = TickEvents {
            tick: self.tick,
            applied_commands: Vec::with_capacity(effective.len()),
            events: Vec::new(),
            terrain: Vec::new(),
        };

        for (player_id, command) in effective.iter() {
            self.apply_player_command(player_id, *command, &mut tick_events)?;
            tick_events.applied_commands.push(player_id);
        }
        self.simulate_projectiles(&mut tick_events)?;
        self.record_pose_frame();
        self.tick = self.tick.wrapping_add(1);
        Ok(tick_events)
    }

    fn validate_commands(&self, commands: &CommandSet) -> Result<(), CoreError> {
        for (player_id, command) in commands.iter() {
            command
                .validate()
                .map_err(|_: ValidationError| CoreError::Command(CommandRejection::InvalidShape))?;
            let Some(player) = self.players.get(player_id.index()) else {
                return Err(CoreError::Command(CommandRejection::UnknownPlayer(
                    player_id,
                )));
            };
            if player.controller == Controller::Empty {
                return Err(CoreError::Command(CommandRejection::EmptySlot(
                    player_id,
                )));
            }
            if let Some(previous) = player.last_sequence {
                if command.sequence() == previous {
                    return Err(CoreError::Command(CommandRejection::DuplicateSequence {
                        player: player_id,
                        sequence: previous,
                    }));
                }
                if !sequence_is_newer(command.sequence(), previous) {
                    return Err(CoreError::Command(CommandRejection::ObsoleteSequence {
                        player: player_id,
                        previous,
                        received: command.sequence(),
                    }));
                }
            }
        }
        Ok(())
    }

    fn generate_bot_command(&mut self, player: PlayerState) -> Result<PlayerCommand, CoreError> {
        let move_x = self.ai_rng.range_i32(-2_047, 2_047) as i16;
        let move_y = self.ai_rng.range_i32(-2_047, 2_047) as i16;
        let look_yaw = self.ai_rng.next_u64() as u16;
        let cast = self.ai_rng.next_u64() % AUTHORITATIVE_HZ as u64 == 0;
        PlayerCommand::new(
            self.tick,
            player.bot_next_sequence,
            move_x,
            move_y,
            0,
            look_yaw,
            0,
            if cast { ACTION_CAST } else { 0 },
            if cast { Some(0) } else { None },
        )
        .map_err(|_| CoreError::Command(CommandRejection::InvalidShape))
    }

    fn apply_player_command(
        &mut self,
        player_id: PlayerId,
        command: PlayerCommand,
        events: &mut TickEvents,
    ) -> Result<(), CoreError> {
        let index = player_id.index();
        let entity_capacity_available =
            self.entities.len() < self.entities.capacity() as usize;
        let (requested_spell, should_cast, should_extract, source_entity, team, position) = {
            let player = &mut self.players[index];
            player.last_sequence = Some(command.sequence());
            if player.controller == Controller::Bot {
                player.bot_next_sequence = next_command_sequence(command.sequence());
            }
            if player.outcome != PlayerOutcome::Active {
                return Ok(());
            }

            let (move_x, move_z) =
                cap_planar_input(i32::from(command.move_x()), i32::from(command.move_y()));
            let velocity_x =
                move_x * PLAYER_PLANAR_SPEED_CM_PER_TICK / i32::from(MAX_MOVE_AXIS);
            let velocity_z =
                move_z * PLAYER_PLANAR_SPEED_CM_PER_TICK / i32::from(MAX_MOVE_AXIS);
            let requested_velocity_y =
                command.move_vertical() as i32 * PLAYER_VERTICAL_SPEED_CM_PER_TICK
                    / i32::from(MAX_MOVE_AXIS);
            let next_y = player.position_cm[1]
                .saturating_add(requested_velocity_y)
                .clamp(PLAYER_MIN_ALTITUDE_CM, PLAYER_MAX_ALTITUDE_CM);
            let velocity_y = next_y - player.position_cm[1];
            player.velocity_cm_per_tick = [
                velocity_x as i16,
                velocity_y as i16,
                velocity_z as i16,
            ];
            player.position_cm[0] = player.position_cm[0].saturating_add(velocity_x);
            player.position_cm[1] = next_y;
            player.position_cm[2] = player.position_cm[2].saturating_add(velocity_z);
            player.yaw = command.look_yaw();
            player.pitch = command.look_pitch();
            let position = player.position_cm;
            let source_entity = player.entity_id;
            let team = player.team;
            let requested_spell = command.requested_spell();
            let implemented_spell = matches!(
                requested_spell,
                Some(FIREBOLT_SPELL_ID) | Some(MEND_SPELL_ID)
            );
            let needs_entity = requested_spell == Some(FIREBOLT_SPELL_ID);
            let spell_ready = requested_spell
                .and_then(|spell| player.spell_cooldown_ticks.get(spell as usize))
                .is_some_and(|cooldown| *cooldown == 0);
            let should_cast = command.action_flags() & ACTION_CAST != 0
                && implemented_spell
                && spell_ready
                && (!needs_entity || entity_capacity_available);
            let should_extract = command.action_flags() & ACTION_EXTRACT != 0;

            if should_cast {
                if requested_spell == Some(MEND_SPELL_ID) {
                    player.spell_cooldown_ticks[MEND_SPELL_ID as usize] =
                        MEND_COOLDOWN_TICKS;
                } else {
                    player.spell_cooldown_ticks[FIREBOLT_SPELL_ID as usize] =
                        FIREBOLT_COOLDOWN_TICKS;
                }
            }
            if should_extract {
                player.outcome = PlayerOutcome::Extracted;
                player.inventory.banked_resources = player
                    .inventory
                    .banked_resources
                    .saturating_add(player.inventory.unbanked_resources);
                player.inventory.unbanked_resources = 0;
                player.velocity_cm_per_tick = [0; 3];
            }
            (
                requested_spell,
                should_cast,
                should_extract,
                source_entity,
                team,
                position,
            )
        };

        if let Some(entity_id) = source_entity {
            if let Some(entity) = self.entities.get_mut(entity_id) {
                entity.position_cm = position;
                entity.velocity_cm_per_tick = self.players[index].velocity_cm_per_tick;
                entity.yaw = command.look_yaw();
                entity.pitch = command.look_pitch();
            }
        }

        if should_cast && requested_spell == Some(FIREBOLT_SPELL_ID) {
            let direction =
                projectile_direction_q30(command.look_yaw(), command.look_pitch());
            let velocity = projectile_velocity(direction, 0);
            let mut projectile = Entity::new(EntityKind::Projectile);
            projectile.owner = Some(player_id);
            projectile.team = team;
            // The collision origin stays close to the player so the first
            // segment cannot skip nearby targets. Presentation eases the
            // viewer-owned projectile from the farther, below-eye hand muzzle.
            projectile.position_cm = [
                position[0].saturating_add(scale_projectile_direction(
                    direction[0],
                    PROJECTILE_MUZZLE_FORWARD_CM,
                )),
                position[1].saturating_add(PROJECTILE_MUZZLE_HEIGHT_CM),
                position[2].saturating_add(scale_projectile_direction(
                    direction[2],
                    PROJECTILE_MUZZLE_FORWARD_CM,
                )),
            ];
            projectile.velocity_cm_per_tick = velocity;
            projectile.yaw = command.look_yaw();
            projectile.pitch = command.look_pitch();
            projectile.health = 1;
            projectile.lifetime_ticks = PROJECTILE_LIFETIME_TICKS;
            let projectile_id = self.entities.spawn(projectile)?;
            events.events.push(self.next_event(
                TickEventKind::Cast,
                source_entity,
                Some(projectile_id),
                [i32::from(FIREBOLT_SPELL_ID), 0, 0, 0],
            ));
            events.events.push(self.next_event(
                TickEventKind::EntitySpawned,
                source_entity,
                Some(projectile_id),
                [EntityKind::Projectile as i32, 0, 0, 0],
            ));
        } else if should_cast && requested_spell == Some(MEND_SPELL_ID) {
            let healed = self.players[index].health.saturating_add(MEND_HEALTH).min(PLAYER_HEALTH);
            self.players[index].health = healed;
            if let Some(entity_id) = source_entity {
                if let Some(entity) = self.entities.get_mut(entity_id) {
                    entity.health = healed;
                }
            }
            events.events.push(self.next_event(
                TickEventKind::Cast,
                source_entity,
                source_entity,
                [i32::from(MEND_SPELL_ID), i32::from(healed), 0, 0],
            ));
        }

        if should_extract {
            events.events.push(self.next_event(
                TickEventKind::Extraction,
                source_entity,
                None,
                [player_id.get() as i32, 0, 0, 0],
            ));
        }
        Ok(())
    }

    fn simulate_projectiles(&mut self, events: &mut TickEvents) -> Result<(), CoreError> {
        let projectile_ids: Vec<EntityId> = self
            .entities
            .active_ids_sorted()
            .into_iter()
            .filter(|id| {
                self.entities
                    .get(*id)
                    .is_some_and(|entity| entity.kind == EntityKind::Projectile)
            })
            .collect();

        for projectile_id in projectile_ids {
            let Some(mut projectile) = self.entities.get(projectile_id).copied() else {
                continue;
            };
            let elapsed_ticks =
                PROJECTILE_LIFETIME_TICKS.saturating_sub(projectile.lifetime_ticks);
            let direction = projectile_direction_q30(projectile.yaw, projectile.pitch);
            projectile.velocity_cm_per_tick =
                projectile_velocity(direction, elapsed_ticks);
            for axis in 0..3 {
                projectile.position_cm[axis] = projectile.position_cm[axis]
                    .saturating_add(projectile.velocity_cm_per_tick[axis] as i32);
            }
            projectile.lifetime_ticks = projectile.lifetime_ticks.saturating_sub(1);
            let terrain_height_cm = height_cm_at(&self.terrain, projectile.position_cm);
            let hit_terrain = projectile.position_cm[1] <= terrain_height_cm;
            if hit_terrain {
                projectile.position_cm[1] = terrain_height_cm;
            }
            if let Some(authoritative) = self.entities.get_mut(projectile_id) {
                *authoritative = projectile;
            }

            let mut hit_player = None;
            for player in &self.players {
                if player.controller == Controller::Empty
                    || player.outcome != PlayerOutcome::Active
                    || Some(player.id) == projectile.owner
                    || player.team == projectile.team
                {
                    continue;
                }
                let dx =
                    i128::from(player.position_cm[0]) - i128::from(projectile.position_cm[0]);
                let dy =
                    i128::from(player.position_cm[1]) - i128::from(projectile.position_cm[1]);
                let dz =
                    i128::from(player.position_cm[2]) - i128::from(projectile.position_cm[2]);
                if dx * dx + dy * dy + dz * dz
                    <= i128::from(PROJECTILE_HIT_RADIUS_CM)
                        * i128::from(PROJECTILE_HIT_RADIUS_CM)
                {
                    hit_player = Some(player.id);
                    break;
                }
            }

            let expired = projectile.lifetime_ticks == 0;
            if let Some(target) = hit_player {
                self.apply_damage(projectile.owner, target, 34, events);
            }
            if hit_player.is_some() || hit_terrain {
                let actor =
                    projectile.owner.and_then(|owner| self.players[owner.index()].entity_id);
                let target = hit_player
                    .and_then(|player| self.players[player.index()].entity_id);
                events.events.push(self.next_event(
                    TickEventKind::ProjectileImpact,
                    actor,
                    target,
                    [
                        projectile.position_cm[0],
                        projectile.position_cm[1],
                        projectile.position_cm[2],
                        i32::from(FIREBOLT_SPELL_ID),
                    ],
                ));
            }
            if hit_player.is_some() || hit_terrain || expired {
                let _ = self.entities.remove(projectile_id);
                events.events.push(self.next_event(
                    TickEventKind::EntityDespawned,
                    projectile.owner.and_then(|owner| self.players[owner.index()].entity_id),
                    Some(projectile_id),
                    [0; 4],
                ));
                if hit_terrain && hit_player.is_none() {
                    let (coord, cell_x, cell_y) = world_to_chunk_cell(projectile.position_cm);
                    let delta = -(3 + (self.environment_rng.next_u64() % 5) as i16);
                    let deformation = deform(
                        &mut self.terrain,
                        self.config.max_terrain_chunks,
                        self.tick,
                        coord,
                        cell_x,
                        cell_y,
                        delta,
                    );
                    match deformation {
                        Ok(deformation) => {
                            events.terrain.push(deformation);
                            events.events.push(self.next_event(
                                TickEventKind::TerrainDeformed,
                                None,
                                None,
                                [
                                    coord.x,
                                    coord.y,
                                    cell_x as i32,
                                    cell_y as i32,
                                ],
                            ));
                        }
                        // A full terrain store cannot make a validated tick
                        // partially fail. The authoritative projectile still
                        // resolves; persistence/telemetry can surface pressure.
                        Err(TerrainError::Capacity) => {}
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
        Ok(())
    }

    fn apply_damage(
        &mut self,
        owner: Option<PlayerId>,
        target: PlayerId,
        damage: u16,
        events: &mut TickEvents,
    ) {
        let target_index = target.index();
        let target_entity;
        let defeated;
        {
            let target_player = &mut self.players[target_index];
            target_player.health = target_player.health.saturating_sub(damage);
            target_entity = target_player.entity_id;
            defeated = target_player.health == 0;
            if defeated {
                target_player.outcome = PlayerOutcome::Defeated;
                target_player.velocity_cm_per_tick = [0; 3];
            }
        }
        if let Some(entity_id) = target_entity {
            if let Some(entity) = self.entities.get_mut(entity_id) {
                entity.health = self.players[target_index].health;
                if defeated {
                    entity.velocity_cm_per_tick = [0; 3];
                }
            }
        }
        let actor = owner.and_then(|player| self.players[player.index()].entity_id);
        events.events.push(self.next_event(
            TickEventKind::Damage,
            actor,
            target_entity,
            [damage as i32, self.players[target_index].health as i32, 0, 0],
        ));
        if defeated {
            events.events.push(self.next_event(
                TickEventKind::Defeat,
                actor,
                target_entity,
                [target.get() as i32, 0, 0, 0],
            ));
        }
    }

    pub fn deform_terrain(
        &mut self,
        chunk: ChunkCoord,
        cell_x: u8,
        cell_y: u8,
        height_delta_cm: i16,
    ) -> Result<TerrainDeformationEvent, CoreError> {
        Ok(deform(
            &mut self.terrain,
            self.config.max_terrain_chunks,
            self.tick,
            chunk,
            cell_x,
            cell_y,
            height_delta_cm,
        )?)
    }

    fn record_pose_frame(&mut self) {
        let mut poses: Vec<Pose> = self
            .entities
            .iter()
            .filter(|entity| entity.kind == EntityKind::Player)
            .map(|entity| Pose {
                entity_id: entity.id,
                position_cm: entity.position_cm,
                yaw: entity.yaw,
                pitch: entity.pitch,
                health: entity.health,
            })
            .collect();
        poses.sort_unstable_by_key(|pose| pose.entity_id);
        if self.pose_history.len() == POSE_HISTORY_TICKS {
            self.pose_history.remove(0);
        }
        self.pose_history.push(PoseFrame {
            tick: self.tick,
            poses,
        });
    }

    pub fn pose_history(&self) -> &[PoseFrame] {
        &self.pose_history
    }

    /// Returns the historical player pose used for instantaneous hit
    /// validation. Requests older than 25 ticks are rejected even while the
    /// longer 33-tick diagnostic history is still retained.
    pub fn rewind_pose(
        &self,
        entity_id: EntityId,
        target_tick: u64,
    ) -> Result<Pose, RewindError> {
        let Some(latest) = self.pose_history.last().map(|frame| frame.tick) else {
            return Err(RewindError::NoHistory);
        };
        if target_tick > latest {
            return Err(RewindError::FutureTick {
                latest,
                requested: target_tick,
            });
        }
        if latest - target_tick > MAX_REWIND_TICKS {
            return Err(RewindError::TooOld {
                latest,
                requested: target_tick,
                maximum_age_ticks: MAX_REWIND_TICKS,
            });
        }
        let frame = self
            .pose_history
            .iter()
            .find(|frame| frame.tick == target_tick)
            .ok_or(RewindError::MissingTick(target_tick))?;
        frame
            .poses
            .iter()
            .find(|pose| pose.entity_id == entity_id)
            .copied()
            .ok_or(RewindError::MissingEntity(entity_id))
    }

    fn next_event(
        &mut self,
        kind: TickEventKind,
        actor: Option<EntityId>,
        target: Option<EntityId>,
        data: [i32; 4],
    ) -> TickEvent {
        let event = TickEvent {
            event_id: self.next_event_id,
            tick: self.tick,
            kind,
            actor,
            target,
            data,
        };
        self.next_event_id = self.next_event_id.wrapping_add(1);
        if self.next_event_id == 0 {
            self.next_event_id = 1;
        }
        event
    }

    pub fn checkpoint(&self) -> AuthoritativeCheckpoint {
        AuthoritativeCheckpoint::from_state(self)
    }

    pub fn restore(checkpoint: AuthoritativeCheckpoint) -> Result<Self, CoreError> {
        Ok(checkpoint.restore_state()?)
    }

    /// Hashes gameplay state only. Replication cursors, snapshot history, and
    /// all presentation/cosmetic state are deliberately excluded.
    pub fn authoritative_hash(&self) -> u64 {
        checksum64(&crate::checkpoint::encode_authoritative_payload(self, false))
    }

    pub(crate) fn snapshot_id(&mut self) -> SnapshotId {
        let id = SnapshotId::new(self.next_snapshot_id);
        self.next_snapshot_id = self.next_snapshot_id.wrapping_add(1);
        if self.next_snapshot_id == 0 {
            self.next_snapshot_id = 1;
        }
        id
    }
}

fn sequence_is_newer(received: u32, previous: u32) -> bool {
    let distance = received.wrapping_sub(previous);
    distance != 0 && distance < (1_u32 << 31)
}

fn next_command_sequence(previous: u32) -> u32 {
    match previous.wrapping_add(1) {
        0 => 1,
        next => next,
    }
}

pub(crate) fn cap_planar_input(move_x: i32, move_z: i32) -> (i32, i32) {
    let magnitude_squared =
        i64::from(move_x) * i64::from(move_x) + i64::from(move_z) * i64::from(move_z);
    let maximum = i64::from(PLAYER_MAX_PLANAR_INPUT_MAGNITUDE);
    if magnitude_squared <= maximum * maximum {
        return (move_x, move_z);
    }
    let floor = integer_square_root(magnitude_squared as u64);
    let magnitude = if floor * floor == magnitude_squared as u64 {
        floor
    } else {
        floor + 1
    } as i64;
    (
        (i64::from(move_x) * maximum / magnitude) as i32,
        (i64::from(move_z) * maximum / magnitude) as i32,
    )
}

fn integer_square_root(value: u64) -> u64 {
    if value < 2 {
        return value;
    }
    let mut estimate = value;
    let mut next = (estimate + 1) / 2;
    while next < estimate {
        estimate = next;
        next = (estimate + value / estimate) / 2;
    }
    estimate
}

const CORDIC_GAIN_INVERSE_Q30: i64 = 652_032_874;
const CORDIC_ATAN_TURN_UNITS: [i32; 15] = [
    8_192, 4_836, 2_555, 1_297, 651, 326, 163, 81, 41, 20, 10, 5, 3, 1, 1,
];

/// Deterministic fixed-point sine/cosine in binary turn units, where 65,536 is
/// one complete turn. CORDIC keeps authoritative aiming identical across
/// native, console, and Wasm targets without relying on platform libm.
fn sin_cos_turn_units(raw_angle: i32) -> (i64, i64) {
    let mut angle = (raw_angle + 32_768).rem_euclid(65_536) - 32_768;
    let sign = if angle > 16_384 {
        angle -= 32_768;
        -1_i64
    } else if angle < -16_384 {
        angle += 32_768;
        -1_i64
    } else {
        1_i64
    };
    let mut x = CORDIC_GAIN_INVERSE_Q30;
    let mut y = 0_i64;
    let mut remaining = angle;
    for (shift, step) in CORDIC_ATAN_TURN_UNITS.iter().copied().enumerate() {
        let previous_x = x;
        let previous_y = y;
        if remaining >= 0 {
            x = previous_x - (previous_y >> shift);
            y = previous_y + (previous_x >> shift);
            remaining -= step;
        } else {
            x = previous_x + (previous_y >> shift);
            y = previous_y - (previous_x >> shift);
            remaining += step;
        }
    }
    (y * sign, x * sign)
}

fn round_shift_q30(value: i128) -> i64 {
    const HALF: i128 = 1_i128 << 29;
    if value >= 0 {
        ((value + HALF) >> 30) as i64
    } else {
        -(((-value + HALF) >> 30) as i64)
    }
}

fn projectile_direction_q30(yaw: u16, pitch: i16) -> [i64; 3] {
    let (sin_yaw, cos_yaw) = sin_cos_turn_units(i32::from(yaw));
    let (sin_pitch, cos_pitch) = sin_cos_turn_units(i32::from(pitch));
    let horizontal_x = round_shift_q30(i128::from(cos_yaw) * i128::from(cos_pitch));
    let horizontal_z = round_shift_q30(i128::from(sin_yaw) * i128::from(cos_pitch));
    [horizontal_x, sin_pitch, horizontal_z]
}

fn projectile_velocity(direction_q30: [i64; 3], elapsed_ticks: u16) -> [i16; 3] {
    let before_distance =
        i128::from(PROJECTILE_SPEED_CM_PER_SECOND)
            * i128::from(elapsed_ticks)
            / i128::from(AUTHORITATIVE_HZ);
    let after_distance =
        i128::from(PROJECTILE_SPEED_CM_PER_SECOND)
            * i128::from(u32::from(elapsed_ticks) + 1)
            / i128::from(AUTHORITATIVE_HZ);
    direction_q30.map(|component| {
        let before = round_shift_q30(i128::from(component) * before_distance);
        let after = round_shift_q30(i128::from(component) * after_distance);
        (after - before) as i16
    })
}

fn scale_projectile_direction(direction_q30: i64, distance_cm: i32) -> i32 {
    round_shift_q30(i128::from(direction_q30) * i128::from(distance_cm)) as i32
}
