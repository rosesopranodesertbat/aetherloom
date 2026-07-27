use alloc::vec::Vec;
use core::fmt;

use aetherloom_protocol::{
    CommandSet, Controller, EntityId, PlayerCommand, PlayerId, TeamId, ValidationError,
    ACTION_CAST, ACTION_EXTRACT, AUTHORITATIVE_HZ, MAX_PLAYERS,
};

use crate::codec::checksum64;
use crate::entity::{Entity, EntityKind, EntityPool, EntityPoolError};
use crate::replication::StoredSnapshot;
use crate::rng::{
    DeterministicRng, AI_STREAM, COMBAT_STREAM, ENVIRONMENT_STREAM, GENERATION_STREAM,
};
use crate::terrain::{
    deform, world_to_chunk_cell, ChunkCoord, TerrainChunk, TerrainDeformationEvent, TerrainError,
};
use crate::{AuthoritativeCheckpoint, CheckpointError, SnapshotId};

const MAX_INPUT_SPEED_CM_PER_TICK: i32 = 8;
const PLAYER_HEALTH: u16 = 100;
const CAST_COOLDOWN_TICKS: u16 = AUTHORITATIVE_HZ as u16 / 4;
const PROJECTILE_LIFETIME_TICKS: u16 = AUTHORITATIVE_HZ as u16 / 4;
const PROJECTILE_SPEED_CM_PER_TICK: i16 = 20;
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
    pub health: u16,
    pub cooldown_ticks: u16,
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
            health: 0,
            cooldown_ticks: 0,
            inventory: Inventory::default(),
            outcome: PlayerOutcome::Defeated,
            last_sequence: None,
            bot_next_sequence: 1,
        }
    }

    pub const fn last_accepted_sequence(&self) -> Option<u32> {
        self.last_sequence
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

        let position = [
            self.generation_rng.range_i32(-8_000, 8_000),
            0,
            self.generation_rng.range_i32(-8_000, 8_000),
        ];
        let mut entity = Entity::new(EntityKind::Player);
        entity.owner = Some(id);
        entity.team = Some(team);
        entity.position_cm = position;
        entity.health = PLAYER_HEALTH;
        let entity_id = self.entities.spawn(entity)?;

        self.players[index] = PlayerState {
            id,
            team: Some(team),
            controller,
            entity_id: Some(entity_id),
            position_cm: position,
            velocity_cm_per_tick: [0; 3],
            yaw: 0,
            health: PLAYER_HEALTH,
            cooldown_ticks: 0,
            inventory: Inventory::default(),
            outcome: PlayerOutcome::Active,
            last_sequence: None,
            bot_next_sequence: 1,
        };
        Ok(entity_id)
    }

    pub fn remove_player(&mut self, id: PlayerId) -> Result<(), CoreError> {
        let Some(player) = self.players.get_mut(id.index()) else {
            return Err(CoreError::InvalidPlayer(id));
        };
        if let Some(entity) = player.entity_id {
            let _ = self.entities.remove(entity);
        }
        *player = PlayerState::empty(id);
        Ok(())
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
            player.cooldown_ticks = player.cooldown_ticks.saturating_sub(1);
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
        let (should_cast, should_extract, source_entity, team, position) = {
            let player = &mut self.players[index];
            player.last_sequence = Some(command.sequence());
            if player.controller == Controller::Bot {
                player.bot_next_sequence = next_command_sequence(command.sequence());
            }
            if player.outcome != PlayerOutcome::Active {
                return Ok(());
            }

            let velocity_x =
                command.move_x() as i32 * MAX_INPUT_SPEED_CM_PER_TICK / 2_047;
            let velocity_z =
                command.move_y() as i32 * MAX_INPUT_SPEED_CM_PER_TICK / 2_047;
            player.velocity_cm_per_tick = [velocity_x as i16, 0, velocity_z as i16];
            player.position_cm[0] = player.position_cm[0].saturating_add(velocity_x);
            player.position_cm[2] = player.position_cm[2].saturating_add(velocity_z);
            player.yaw = command.look_yaw();
            let position = player.position_cm;
            let source_entity = player.entity_id;
            let team = player.team;
            let should_cast = command.action_flags() & ACTION_CAST != 0
                && command.requested_spell().is_some()
                && player.cooldown_ticks == 0
                && entity_capacity_available;
            let should_extract = command.action_flags() & ACTION_EXTRACT != 0;

            if should_cast {
                player.cooldown_ticks = CAST_COOLDOWN_TICKS;
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
            (should_cast, should_extract, source_entity, team, position)
        };

        if let Some(entity_id) = source_entity {
            if let Some(entity) = self.entities.get_mut(entity_id) {
                entity.position_cm = position;
                entity.velocity_cm_per_tick = self.players[index].velocity_cm_per_tick;
                entity.yaw = command.look_yaw();
            }
        }

        if should_cast {
            let direction = direction_from_yaw(command.look_yaw());
            let mut projectile = Entity::new(EntityKind::Projectile);
            projectile.owner = Some(player_id);
            projectile.team = team;
            projectile.position_cm = position;
            projectile.velocity_cm_per_tick = [
                direction[0] * PROJECTILE_SPEED_CM_PER_TICK,
                0,
                direction[1] * PROJECTILE_SPEED_CM_PER_TICK,
            ];
            projectile.health = 1;
            projectile.lifetime_ticks = PROJECTILE_LIFETIME_TICKS;
            let projectile_id = self.entities.spawn(projectile)?;
            events.events.push(self.next_event(
                TickEventKind::Cast,
                source_entity,
                Some(projectile_id),
                [command.requested_spell().unwrap_or(0) as i32, 0, 0, 0],
            ));
            events.events.push(self.next_event(
                TickEventKind::EntitySpawned,
                source_entity,
                Some(projectile_id),
                [EntityKind::Projectile as i32, 0, 0, 0],
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
            for axis in 0..3 {
                projectile.position_cm[axis] = projectile.position_cm[axis]
                    .saturating_add(projectile.velocity_cm_per_tick[axis] as i32);
            }
            projectile.lifetime_ticks = projectile.lifetime_ticks.saturating_sub(1);
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
                let damage = 18 + (self.combat_rng.next_u64() % 5) as u16;
                self.apply_damage(projectile.owner, target, damage, events);
            }
            if hit_player.is_some() || expired {
                let _ = self.entities.remove(projectile_id);
                events.events.push(self.next_event(
                    TickEventKind::EntityDespawned,
                    projectile.owner.and_then(|owner| self.players[owner.index()].entity_id),
                    Some(projectile_id),
                    [0; 4],
                ));
                if expired {
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

fn direction_from_yaw(yaw: u16) -> [i16; 2] {
    match ((yaw as u32 + 4_096) / 8_192) & 7 {
        0 => [1, 0],
        1 => [1, 1],
        2 => [0, 1],
        3 => [-1, 1],
        4 => [-1, 0],
        5 => [-1, -1],
        6 => [0, -1],
        _ => [1, -1],
    }
}
