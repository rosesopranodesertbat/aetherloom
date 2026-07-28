#![deny(unsafe_code)]
#![deny(warnings)]

use core::ptr;

use aetherloom_core::{
    CommandSet, Controller, CoreError, EntityId, EntityKind, MatchConfig, MatchState,
    PlayerCommand, PlayerId, PlayerSpawn, TeamId, TickEvent, WorldSeed, AUTHORITATIVE_HZ,
};
use aetherloom_protocol::MAX_SPELL_ID;

pub const BRIDGE_MAX_PLAYERS: usize = 8;
pub const SNAPSHOT_MAGIC: i32 = 0x314d_4c41;
pub const SNAPSHOT_ABI_VERSION: i32 = 2;
pub const SNAPSHOT_HEADER_WORDS: usize = 16;
pub const SNAPSHOT_PLAYER_WORDS: usize = 16;
pub const SNAPSHOT_PROJECTILE_WORDS: usize = 15;
pub const SNAPSHOT_EVENT_WORDS: usize = 13;
pub const INPUT_HOLD_TICKS: u8 = 2;

pub const STATUS_OK: i32 = 0;
pub const STATUS_NOT_INITIALIZED: i32 = -1;
pub const STATUS_INVALID_PLAYER: i32 = -2;
pub const STATUS_INVALID_TEAM: i32 = -3;
pub const STATUS_SLOT_ACTIVE: i32 = -4;
pub const STATUS_SLOT_EMPTY: i32 = -5;
pub const STATUS_NOT_HUMAN: i32 = -6;
pub const STATUS_INVALID_INPUT: i32 = -7;
pub const STATUS_CORE_REJECTED: i32 = -8;

const MATCH_MAX_ENTITIES: u32 = 128;
const MATCH_MAX_TERRAIN_CHUNKS: u32 = 64;
const MAX_LAST_TICK_EVENTS: usize = 512;
const SNAPSHOT_CAPACITY_WORDS: usize = SNAPSHOT_HEADER_WORDS
    + BRIDGE_MAX_PLAYERS * SNAPSHOT_PLAYER_WORDS
    + MATCH_MAX_ENTITIES as usize * SNAPSHOT_PROJECTILE_WORDS
    + MAX_LAST_TICK_EVENTS * SNAPSHOT_EVENT_WORDS;

const DUEL_SPAWNS: [PlayerSpawn; BRIDGE_MAX_PLAYERS] = [
    PlayerSpawn::new([-250, 0, 0], 0),
    PlayerSpawn::new([250, 0, 0], 32_768),
    PlayerSpawn::new([0, 0, -250], 16_384),
    PlayerSpawn::new([0, 0, 250], 49_152),
    PlayerSpawn::new([-180, 0, -180], 8_192),
    PlayerSpawn::new([180, 0, 180], 40_960),
    PlayerSpawn::new([-180, 0, 180], 57_344),
    PlayerSpawn::new([180, 0, -180], 24_576),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BridgeError {
    NotInitialized,
    InvalidPlayer,
    InvalidTeam,
    SlotActive,
    SlotEmpty,
    NotHuman,
    InvalidInput,
    CoreRejected,
}

impl BridgeError {
    const fn status(self) -> i32 {
        match self {
            Self::NotInitialized => STATUS_NOT_INITIALIZED,
            Self::InvalidPlayer => STATUS_INVALID_PLAYER,
            Self::InvalidTeam => STATUS_INVALID_TEAM,
            Self::SlotActive => STATUS_SLOT_ACTIVE,
            Self::SlotEmpty => STATUS_SLOT_EMPTY,
            Self::NotHuman => STATUS_NOT_HUMAN,
            Self::InvalidInput => STATUS_INVALID_INPUT,
            Self::CoreRejected => STATUS_CORE_REJECTED,
        }
    }

    const fn from_core(error: CoreError) -> Self {
        match error {
            CoreError::InvalidPlayer(_) => Self::InvalidPlayer,
            CoreError::PlayerAlreadyActive(_) => Self::SlotActive,
            CoreError::PlayerInactive(_) => Self::SlotEmpty,
            _ => Self::CoreRejected,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LatestInput {
    move_x: i16,
    move_y: i16,
    move_vertical: i16,
    look_yaw: u16,
    look_pitch: i16,
    action_flags: u16,
    requested_spell: Option<u8>,
}

impl LatestInput {
    const fn neutral(look_yaw: u16, look_pitch: i16) -> Self {
        Self {
            move_x: 0,
            move_y: 0,
            move_vertical: 0,
            look_yaw,
            look_pitch,
            action_flags: 0,
            requested_spell: None,
        }
    }

    fn command(self, target_tick: u64, sequence: u32) -> Result<PlayerCommand, BridgeError> {
        PlayerCommand::new(
            target_tick,
            sequence,
            self.move_x,
            self.move_y,
            self.move_vertical,
            self.look_yaw,
            self.look_pitch,
            self.action_flags,
            self.requested_spell,
        )
        .map_err(|_| BridgeError::InvalidInput)
    }
}

#[derive(Debug)]
struct WorkerMatch {
    state: MatchState,
    latest_inputs: [LatestInput; BRIDGE_MAX_PLAYERS],
    input_ticks_remaining: [u8; BRIDGE_MAX_PLAYERS],
    last_events: Vec<TickEvent>,
    snapshot: Vec<i32>,
}

impl WorkerMatch {
    fn new(seed: u64) -> Result<Self, BridgeError> {
        let config = MatchConfig::new(
            BRIDGE_MAX_PLAYERS as u16,
            MATCH_MAX_ENTITIES,
            MATCH_MAX_TERRAIN_CHUNKS,
        )
        .map_err(BridgeError::from_core)?;
        let mut instance = Self {
            state: MatchState::new(config, WorldSeed::new(seed)),
            latest_inputs: [LatestInput::neutral(0, 0); BRIDGE_MAX_PLAYERS],
            input_ticks_remaining: [0; BRIDGE_MAX_PLAYERS],
            last_events: Vec::with_capacity(MAX_LAST_TICK_EVENTS),
            snapshot: Vec::with_capacity(SNAPSHOT_CAPACITY_WORDS),
        };
        instance.rebuild_snapshot();
        Ok(instance)
    }

    fn add_player(
        &mut self,
        raw_player: i32,
        raw_team: i32,
        controller: Controller,
    ) -> Result<(), BridgeError> {
        let player = bridge_player_id(raw_player)?;
        let team = bridge_team_id(raw_team)?;
        let spawn = DUEL_SPAWNS[player.index()];
        self.state
            .add_player_at_spawn(player, team, controller, spawn)
            .map_err(BridgeError::from_core)?;
        self.latest_inputs[player.index()] = LatestInput::neutral(spawn.yaw(), 0);
        self.input_ticks_remaining[player.index()] = 0;
        self.rebuild_snapshot();
        Ok(())
    }

    fn remove_player(&mut self, raw_player: i32) -> Result<(), BridgeError> {
        let player = bridge_player_id(raw_player)?;
        if self
            .state
            .player(player)
            .map_or(true, |state| state.controller == Controller::Empty)
        {
            return Err(BridgeError::SlotEmpty);
        }
        self.state
            .remove_player(player)
            .map_err(BridgeError::from_core)?;
        self.latest_inputs[player.index()] = LatestInput::neutral(0, 0);
        self.input_ticks_remaining[player.index()] = 0;
        self.rebuild_snapshot();
        Ok(())
    }

    fn set_controller(
        &mut self,
        raw_player: i32,
        controller: Controller,
    ) -> Result<(), BridgeError> {
        let player = bridge_player_id(raw_player)?;
        let (yaw, pitch) = self
            .state
            .player(player)
            .filter(|state| state.controller != Controller::Empty)
            .map(|state| (state.yaw, state.pitch))
            .ok_or(BridgeError::SlotEmpty)?;
        self.state
            .set_controller(player, controller)
            .map_err(BridgeError::from_core)?;
        self.latest_inputs[player.index()] = LatestInput::neutral(yaw, pitch);
        self.input_ticks_remaining[player.index()] = 0;
        self.rebuild_snapshot();
        Ok(())
    }

    fn reset_player(&mut self, raw_player: i32) -> Result<(), BridgeError> {
        let player = bridge_player_id(raw_player)?;
        let spawn = DUEL_SPAWNS[player.index()];
        self.state
            .reset_player_at_spawn(player, spawn)
            .map_err(BridgeError::from_core)?;
        self.state.clear_projectiles();
        self.latest_inputs[player.index()] = LatestInput::neutral(spawn.yaw(), 0);
        self.input_ticks_remaining[player.index()] = 0;
        self.rebuild_snapshot();
        Ok(())
    }

    fn clear_input(&mut self, raw_player: i32) -> Result<(), BridgeError> {
        let player = bridge_player_id(raw_player)?;
        let (yaw, pitch) = self
            .state
            .player(player)
            .filter(|state| state.controller != Controller::Empty)
            .map(|state| (state.yaw, state.pitch))
            .ok_or(BridgeError::SlotEmpty)?;
        self.latest_inputs[player.index()] = LatestInput::neutral(yaw, pitch);
        self.input_ticks_remaining[player.index()] = 0;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn submit_input(
        &mut self,
        raw_player: i32,
        move_x: i32,
        move_y: i32,
        move_vertical: i32,
        look_yaw: i32,
        look_pitch: i32,
        action_flags: i32,
        requested_spell: i32,
    ) -> Result<(), BridgeError> {
        let player = bridge_player_id(raw_player)?;
        let player_state = self
            .state
            .player(player)
            .filter(|state| state.controller != Controller::Empty)
            .ok_or(BridgeError::SlotEmpty)?;
        if player_state.controller != Controller::Human {
            return Err(BridgeError::NotHuman);
        }

        let input = LatestInput {
            move_x: i16::try_from(move_x).map_err(|_| BridgeError::InvalidInput)?,
            move_y: i16::try_from(move_y).map_err(|_| BridgeError::InvalidInput)?,
            move_vertical: i16::try_from(move_vertical)
                .map_err(|_| BridgeError::InvalidInput)?,
            look_yaw: u16::try_from(look_yaw).map_err(|_| BridgeError::InvalidInput)?,
            look_pitch: i16::try_from(look_pitch).map_err(|_| BridgeError::InvalidInput)?,
            action_flags: u16::try_from(action_flags).map_err(|_| BridgeError::InvalidInput)?,
            requested_spell: decode_spell(requested_spell)?,
        };
        let sequence = next_sequence(player_state.last_accepted_sequence());
        input.command(self.state.tick(), sequence)?;
        self.latest_inputs[player.index()] = input;
        self.input_ticks_remaining[player.index()] = INPUT_HOLD_TICKS;
        Ok(())
    }

    fn advance_tick(&mut self) -> Result<(), BridgeError> {
        let tick = self.state.tick();
        let mut commands = CommandSet::new(tick);
        for raw in 0..BRIDGE_MAX_PLAYERS as u16 {
            let player = PlayerId::new(raw).map_err(|_| BridgeError::InvalidPlayer)?;
            let Some(player_state) = self.state.player(player) else {
                return Err(BridgeError::CoreRejected);
            };
            if player_state.controller != Controller::Human {
                continue;
            }
            let player_index = player.index();
            let sequence = next_sequence(player_state.last_accepted_sequence());
            let input = if self.input_ticks_remaining[player_index] == 0 {
                LatestInput::neutral(player_state.yaw, player_state.pitch)
            } else {
                self.input_ticks_remaining[player_index] -= 1;
                self.latest_inputs[player_index]
            };
            let command = input.command(tick, sequence)?;
            commands
                .insert(player, command)
                .map_err(|_| BridgeError::CoreRejected)?;
        }

        let events = self
            .state
            .advance_tick(commands)
            .map_err(BridgeError::from_core)?;
        self.last_events = events.events;
        self.rebuild_snapshot();
        Ok(())
    }

    fn rebuild_snapshot(&mut self) {
        let state = &self.state;
        let last_events = &self.last_events;
        let snapshot = &mut self.snapshot;
        snapshot.clear();
        snapshot.resize(SNAPSHOT_HEADER_WORDS, 0);

        let players_offset = snapshot.len();
        let mut player_count = 0_usize;
        for player in state.players() {
            if player.controller == Controller::Empty {
                continue;
            }
            player_count += 1;
            snapshot.push(i32::from(player.id.get()));
            snapshot.push(i32::from(u8::from(player.controller)));
            snapshot.push(player.team.map_or(-1, |team| i32::from(team.get())));
            push_optional_entity(snapshot, player.entity_id);
            snapshot.extend_from_slice(&player.position_cm);
            snapshot.push(i32::from(player.velocity_cm_per_tick[0]));
            snapshot.push(i32::from(player.velocity_cm_per_tick[1]));
            snapshot.push(i32::from(player.velocity_cm_per_tick[2]));
            snapshot.push(i32::from(player.yaw));
            snapshot.push(i32::from(player.pitch));
            snapshot.push(i32::from(player.health));
            snapshot.push(i32::from(player.cooldown_ticks));
            snapshot.push(player.outcome as i32);
        }

        let projectiles_offset = snapshot.len();
        let mut projectile_count = 0_usize;
        for entity_id in state.entities().active_ids_sorted() {
            let Some(entity) = state.entities().get(entity_id) else {
                continue;
            };
            if entity.kind != EntityKind::Projectile {
                continue;
            }
            projectile_count += 1;
            push_entity(snapshot, entity.id);
            snapshot.push(entity.owner.map_or(-1, |owner| i32::from(owner.get())));
            snapshot.push(entity.team.map_or(-1, |team| i32::from(team.get())));
            snapshot.extend_from_slice(&entity.position_cm);
            snapshot.push(i32::from(entity.velocity_cm_per_tick[0]));
            snapshot.push(i32::from(entity.velocity_cm_per_tick[1]));
            snapshot.push(i32::from(entity.velocity_cm_per_tick[2]));
            snapshot.push(i32::from(entity.yaw));
            snapshot.push(i32::from(entity.pitch));
            snapshot.push(i32::from(entity.lifetime_ticks));
            snapshot.push(i32::from(entity.flags));
            snapshot.push(i32::from(entity.health));
        }

        let events_offset = snapshot.len();
        for event in last_events {
            push_u64(snapshot, event.event_id);
            push_u64(snapshot, event.tick);
            snapshot.push(event.kind as i32);
            push_optional_entity(snapshot, event.actor);
            push_optional_entity(snapshot, event.target);
            snapshot.extend_from_slice(&event.data);
        }

        snapshot[0] = SNAPSHOT_MAGIC;
        snapshot[1] = SNAPSHOT_ABI_VERSION;
        snapshot[2] = snapshot.len() as i32;
        snapshot[3] = low_word(state.tick());
        snapshot[4] = high_word(state.tick());
        snapshot[5] = AUTHORITATIVE_HZ as i32;
        snapshot[6] = player_count as i32;
        snapshot[7] = SNAPSHOT_PLAYER_WORDS as i32;
        snapshot[8] = projectile_count as i32;
        snapshot[9] = SNAPSHOT_PROJECTILE_WORDS as i32;
        snapshot[10] = last_events.len() as i32;
        snapshot[11] = SNAPSHOT_EVENT_WORDS as i32;
        snapshot[12] = players_offset as i32;
        snapshot[13] = projectiles_offset as i32;
        snapshot[14] = events_offset as i32;
        snapshot[15] = 0;
    }
}

fn bridge_player_id(raw: i32) -> Result<PlayerId, BridgeError> {
    let raw = u16::try_from(raw).map_err(|_| BridgeError::InvalidPlayer)?;
    if usize::from(raw) >= BRIDGE_MAX_PLAYERS {
        return Err(BridgeError::InvalidPlayer);
    }
    PlayerId::new(raw).map_err(|_| BridgeError::InvalidPlayer)
}

fn bridge_team_id(raw: i32) -> Result<TeamId, BridgeError> {
    let raw = u16::try_from(raw).map_err(|_| BridgeError::InvalidTeam)?;
    TeamId::new(raw).map_err(|_| BridgeError::InvalidTeam)
}

fn decode_spell(raw: i32) -> Result<Option<u8>, BridgeError> {
    if raw == -1 {
        return Ok(None);
    }
    let spell = u8::try_from(raw).map_err(|_| BridgeError::InvalidInput)?;
    if spell > MAX_SPELL_ID {
        return Err(BridgeError::InvalidInput);
    }
    Ok(Some(spell))
}

const fn next_sequence(previous: Option<u32>) -> u32 {
    match previous {
        None => 1,
        Some(previous) => match previous.wrapping_add(1) {
            0 => 1,
            next => next,
        },
    }
}

fn push_optional_entity(words: &mut Vec<i32>, entity: Option<EntityId>) {
    if let Some(entity) = entity {
        push_entity(words, entity);
    } else {
        words.extend_from_slice(&[0, 0]);
    }
}

fn push_entity(words: &mut Vec<i32>, entity: EntityId) {
    push_u64(words, entity.get());
}

fn push_u64(words: &mut Vec<i32>, value: u64) {
    words.push(low_word(value));
    words.push(high_word(value));
}

const fn low_word(value: u64) -> i32 {
    value as u32 as i32
}

const fn high_word(value: u64) -> i32 {
    (value >> 32) as u32 as i32
}

#[derive(Debug)]
struct Runtime {
    instance: Option<WorkerMatch>,
}

impl Runtime {
    const fn empty() -> Self {
        Self { instance: None }
    }
}

static RUNTIME: spin::Mutex<Runtime> = spin::Mutex::new(Runtime::empty());

fn with_instance(operation: impl FnOnce(&mut WorkerMatch) -> Result<(), BridgeError>) -> i32 {
    let runtime = &mut *RUNTIME.lock();
    let Some(instance) = runtime.instance.as_mut() else {
        return BridgeError::NotInitialized.status();
    };
    match operation(instance) {
        Ok(()) => STATUS_OK,
        Err(error) => error.status(),
    }
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_abi_version() -> i32 {
    SNAPSHOT_ABI_VERSION
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_authoritative_hz() -> i32 {
    AUTHORITATIVE_HZ as i32
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_max_players() -> i32 {
    BRIDGE_MAX_PLAYERS as i32
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_init(seed_low: u32, seed_high: u32) -> i32 {
    let seed = u64::from(seed_low) | (u64::from(seed_high) << 32);
    match WorkerMatch::new(seed) {
        Ok(instance) => {
            RUNTIME.lock().instance = Some(instance);
            STATUS_OK
        }
        Err(error) => error.status(),
    }
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_add_human(player: i32, team: i32) -> i32 {
    with_instance(|instance| instance.add_player(player, team, Controller::Human))
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_add_bot(player: i32, team: i32) -> i32 {
    with_instance(|instance| instance.add_player(player, team, Controller::Bot))
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_remove_player(player: i32) -> i32 {
    with_instance(|instance| instance.remove_player(player))
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_set_human(player: i32) -> i32 {
    with_instance(|instance| instance.set_controller(player, Controller::Human))
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_set_bot(player: i32) -> i32 {
    with_instance(|instance| instance.set_controller(player, Controller::Bot))
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_reset_player(player: i32) -> i32 {
    with_instance(|instance| instance.reset_player(player))
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_clear_input(player: i32) -> i32 {
    with_instance(|instance| instance.clear_input(player))
}

#[allow(unsafe_code)]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn worker_match_submit_input(
    player: i32,
    move_x: i32,
    move_y: i32,
    move_vertical: i32,
    look_yaw: i32,
    look_pitch: i32,
    action_flags: i32,
    requested_spell: i32,
) -> i32 {
    with_instance(|instance| {
        instance.submit_input(
            player,
            move_x,
            move_y,
            move_vertical,
            look_yaw,
            look_pitch,
            action_flags,
            requested_spell,
        )
    })
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_advance_tick() -> i32 {
    with_instance(WorkerMatch::advance_tick)
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_snapshot_ptr() -> *const i32 {
    RUNTIME
        .lock()
        .instance
        .as_ref()
        .map_or(ptr::null(), |instance| instance.snapshot.as_ptr())
}

#[allow(unsafe_code)]
#[no_mangle]
pub extern "C" fn worker_match_snapshot_len() -> u32 {
    RUNTIME
        .lock()
        .instance
        .as_ref()
        .map_or(0, |instance| instance.snapshot.len() as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aetherloom_protocol::{ACTION_CAST, MAX_MOVE_AXIS};

    fn header_tick(words: &[i32]) -> u64 {
        u64::from(words[3] as u32) | (u64::from(words[4] as u32) << 32)
    }

    #[test]
    fn initialized_snapshot_has_the_versioned_empty_layout() {
        let instance = WorkerMatch::new(7).unwrap();
        let words = &instance.snapshot;
        assert_eq!(words[0], SNAPSHOT_MAGIC);
        assert_eq!(words[1], SNAPSHOT_ABI_VERSION);
        assert_eq!(words[2] as usize, words.len());
        assert_eq!(header_tick(words), 0);
        assert_eq!(words[5], 128);
        assert_eq!(words[6], 0);
        assert_eq!(words[7], SNAPSHOT_PLAYER_WORDS as i32);
        assert_eq!(words[8], 0);
        assert_eq!(words[9], SNAPSHOT_PROJECTILE_WORDS as i32);
        assert_eq!(words[10], 0);
        assert_eq!(words[11], SNAPSHOT_EVENT_WORDS as i32);
        assert_eq!(words[12], SNAPSHOT_HEADER_WORDS as i32);
        assert_eq!(words[13], SNAPSHOT_HEADER_WORDS as i32);
        assert_eq!(words[14], SNAPSHOT_HEADER_WORDS as i32);
    }

    #[test]
    fn human_input_is_validated_then_resequenced_at_128_hz() {
        let mut instance = WorkerMatch::new(8).unwrap();
        instance.add_player(0, 0, Controller::Human).unwrap();
        instance
            .submit_input(0, i32::from(MAX_MOVE_AXIS), 0, 0, 0, 0, 0, -1)
            .unwrap();
        assert_eq!(
            instance.submit_input(0, i32::from(MAX_MOVE_AXIS) + 1, 0, 0, 0, 0, 0, -1),
            Err(BridgeError::InvalidInput)
        );

        instance.advance_tick().unwrap();
        instance.advance_tick().unwrap();
        let player = instance.state.player(PlayerId::new(0).unwrap()).unwrap();
        assert_eq!(player.position_cm, [-234, 0, 0]);
        assert_eq!(player.last_accepted_sequence(), Some(2));
        let stationary = player.position_cm;
        assert_eq!(header_tick(&instance.snapshot), 2);

        instance.advance_tick().unwrap();
        let player = instance.state.player(PlayerId::new(0).unwrap()).unwrap();
        assert_eq!(player.position_cm, stationary);
        assert_eq!(player.last_accepted_sequence(), Some(3));

        instance
            .submit_input(0, i32::from(MAX_MOVE_AXIS), 0, 0, 0, 0, 0, -1)
            .unwrap();
        instance.clear_input(0).unwrap();
        instance.advance_tick().unwrap();
        let player = instance.state.player(PlayerId::new(0).unwrap()).unwrap();
        assert_eq!(player.position_cm, stationary);
        assert_eq!(player.last_accepted_sequence(), Some(4));
    }

    #[test]
    fn vertical_input_and_pitch_reach_the_version_two_player_snapshot() {
        let mut instance = WorkerMatch::new(12).unwrap();
        instance.add_player(0, 0, Controller::Human).unwrap();
        instance
            .submit_input(
                0,
                0,
                0,
                i32::from(MAX_MOVE_AXIS),
                1_234,
                2_345,
                0,
                -1,
            )
            .unwrap();

        instance.advance_tick().unwrap();
        let offset = instance.snapshot[12] as usize;
        assert_eq!(instance.snapshot[7], 16);
        assert_eq!(instance.snapshot[offset + 6], 5);
        assert_eq!(instance.snapshot[offset + 9], 5);
        assert_eq!(instance.snapshot[offset + 11], 1_234);
        assert_eq!(instance.snapshot[offset + 12], 2_345);

        instance.advance_tick().unwrap();
        instance.advance_tick().unwrap();
        let player = instance.state.player(PlayerId::new(0).unwrap()).unwrap();
        assert_eq!(player.pitch, 2_345);
        assert_eq!(player.position_cm[1], 10);
        assert_eq!(player.velocity_cm_per_tick[1], 0);
    }

    #[test]
    fn bot_takeover_and_return_to_human_preserve_sequence_safety() {
        let mut instance = WorkerMatch::new(9).unwrap();
        instance.add_player(0, 0, Controller::Human).unwrap();
        instance.advance_tick().unwrap();
        instance.set_controller(0, Controller::Bot).unwrap();
        instance.advance_tick().unwrap();
        assert_eq!(
            instance
                .state
                .player(PlayerId::new(0).unwrap())
                .unwrap()
                .last_accepted_sequence(),
            Some(2)
        );

        instance.set_controller(0, Controller::Human).unwrap();
        let before = instance
            .state
            .player(PlayerId::new(0).unwrap())
            .unwrap()
            .position_cm;
        instance.advance_tick().unwrap();
        let player = instance.state.player(PlayerId::new(0).unwrap()).unwrap();
        assert_eq!(player.position_cm, before);
        assert_eq!(player.last_accepted_sequence(), Some(3));
    }

    #[test]
    fn real_projectile_damage_and_events_reach_the_snapshot() {
        let mut instance = WorkerMatch::new(10).unwrap();
        instance.add_player(0, 0, Controller::Human).unwrap();
        instance.add_player(1, 1, Controller::Human).unwrap();
        instance
            .submit_input(0, 0, 0, 0, 0, 0, i32::from(ACTION_CAST), 0)
            .unwrap();

        let mut saw_damage = false;
        for _ in 0..32 {
            instance.advance_tick().unwrap();
            let count = instance.snapshot[10] as usize;
            let offset = instance.snapshot[14] as usize;
            for index in 0..count {
                let kind = instance.snapshot[offset + index * SNAPSHOT_EVENT_WORDS + 4];
                if kind == aetherloom_core::TickEventKind::Damage as i32 {
                    saw_damage = true;
                }
            }
            if saw_damage {
                break;
            }
        }

        assert!(saw_damage);
        assert!(
            instance
                .state
                .player(PlayerId::new(1).unwrap())
                .unwrap()
                .health
                < 100
        );
    }

    #[test]
    fn reset_restores_the_fixed_close_range_spawn() {
        let mut instance = WorkerMatch::new(11).unwrap();
        instance.add_player(0, 0, Controller::Human).unwrap();
        instance
            .submit_input(0, i32::from(MAX_MOVE_AXIS), 0, 0, 0, 0, 0, -1)
            .unwrap();
        instance.advance_tick().unwrap();
        assert_ne!(
            instance
                .state
                .player(PlayerId::new(0).unwrap())
                .unwrap()
                .position_cm,
            DUEL_SPAWNS[0].position_cm()
        );

        instance
            .submit_input(0, 0, 0, 0, 0, 0, i32::from(ACTION_CAST), 0)
            .unwrap();
        instance.advance_tick().unwrap();
        assert!(instance
            .state
            .entities()
            .iter()
            .any(|entity| entity.kind == EntityKind::Projectile));

        instance.reset_player(0).unwrap();
        let player = instance.state.player(PlayerId::new(0).unwrap()).unwrap();
        assert_eq!(player.position_cm, DUEL_SPAWNS[0].position_cm());
        assert_eq!(player.yaw, DUEL_SPAWNS[0].yaw());
        assert_eq!(player.health, 100);
        assert_eq!(player.last_accepted_sequence(), Some(2));
        assert!(instance
            .state
            .entities()
            .iter()
            .all(|entity| entity.kind != EntityKind::Projectile));
    }
}
