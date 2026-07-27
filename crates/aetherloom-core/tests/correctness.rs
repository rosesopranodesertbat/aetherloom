use aetherloom_core::{
    AuthoritativeCheckpoint, ChunkCoord, ClientReplica, CommandRejection, CommandSet,
    Controller, CoreError, Entity, EntityKind, EntityPool, InterestTier, MatchConfig,
    MatchState, PlayerCommand, PlayerId, PresentationWorld, ReplicaError,
    ReplicatedEntity, ReplicationError, RewindError, Snapshot, SnapshotId,
    SnapshotKind, TeamId, WorldSeed, MAX_INTERPOLATION_DELAY_TICKS,
    MAX_INTERPOLATION_SAMPLES, MAX_PENDING_PREDICTION_COMMANDS,
    MAX_REWIND_TICKS, MIN_INTERPOLATION_DELAY_TICKS, POSE_HISTORY_TICKS,
};
use aetherloom_protocol::ACTION_CAST;
use std::collections::BTreeSet;

fn config() -> MatchConfig {
    MatchConfig::new(8, 256, 64).unwrap()
}

fn player(raw: u16) -> PlayerId {
    PlayerId::new(raw).unwrap()
}

fn team(raw: u16) -> TeamId {
    TeamId::new(raw).unwrap()
}

fn command(
    tick: u64,
    sequence: u32,
    move_x: i16,
    move_y: i16,
    cast: bool,
) -> PlayerCommand {
    PlayerCommand::new(
        tick,
        sequence,
        move_x,
        move_y,
        0,
        0,
        if cast { ACTION_CAST } else { 0 },
        cast.then_some(0),
    )
    .unwrap()
}

fn commands(tick: u64, id: PlayerId, command: PlayerCommand) -> CommandSet {
    let mut set = CommandSet::new(tick);
    set.insert(id, command).unwrap();
    set
}

#[test]
fn repeated_replays_produce_identical_events_and_hashes() {
    let mut first = MatchState::new(config(), WorldSeed::new(0xA37E));
    let mut second = MatchState::new(config(), WorldSeed::new(0xA37E));
    for state in [&mut first, &mut second] {
        state
            .add_player(player(0), team(0), Controller::Human)
            .unwrap();
        state
            .add_player(player(1), team(1), Controller::Bot)
            .unwrap();
    }

    for tick in 0..300_u64 {
        let input = command(
            tick,
            tick as u32 + 1,
            ((tick as i32 * 37 % 4_095) - 2_047) as i16,
            ((tick as i32 * 53 % 4_095) - 2_047) as i16,
            tick % 40 == 0,
        );
        let first_events = first
            .advance_tick(commands(tick, player(0), input))
            .unwrap();
        let second_events = second
            .advance_tick(commands(tick, player(0), input))
            .unwrap();
        assert_eq!(first_events, second_events);
        assert_eq!(first.authoritative_hash(), second.authoritative_hash());
    }
}

#[test]
fn cosmetic_seed_and_presentation_extraction_do_not_change_authority() {
    let mut state = MatchState::new(config(), WorldSeed::new(9));
    state
        .add_player(player(0), team(0), Controller::Human)
        .unwrap();
    let before = state.authoritative_hash();

    let mut presentation = PresentationWorld::new(1);
    let first = presentation.extract(&state, player(0), 0.25);
    presentation.set_cosmetic_seed(u64::MAX);
    let second = presentation.extract(&state, player(0), 0.75);

    assert_ne!(first.cosmetic_seed, second.cosmetic_seed);
    assert_eq!(before, state.authoritative_hash());
}

#[test]
fn presented_and_headless_replays_keep_identical_authoritative_state() {
    let mut headless = MatchState::new(config(), WorldSeed::new(0x55AA));
    let mut presented = MatchState::new(config(), WorldSeed::new(0x55AA));
    for state in [&mut headless, &mut presented] {
        state
            .add_player(player(0), team(0), Controller::Human)
            .unwrap();
    }
    let mut presentation = PresentationWorld::new(0);
    for tick in 0..96_u64 {
        let input = command(tick, tick as u32 + 1, 1_200, -700, tick % 32 == 0);
        headless
            .advance_tick(commands(tick, player(0), input))
            .unwrap();
        presented
            .advance_tick(commands(tick, player(0), input))
            .unwrap();
        presentation.set_cosmetic_seed(tick.wrapping_mul(0x9E37_79B9));
        let _frame = presentation.extract(&presented, player(0), 0.5);
        assert_eq!(
            headless.authoritative_hash(),
            presented.authoritative_hash()
        );
    }
}

#[test]
fn checkpoint_round_trip_restores_exact_future_state() {
    let mut state = MatchState::new(config(), WorldSeed::new(42));
    state
        .add_player(player(0), team(0), Controller::Human)
        .unwrap();
    state
        .deform_terrain(ChunkCoord { x: -2, y: 4 }, 3, 5, -12)
        .unwrap();
    for tick in 0..40_u64 {
        state
            .advance_tick(commands(
                tick,
                player(0),
                command(tick, tick as u32 + 1, 2_047, -1_024, tick == 0),
            ))
            .unwrap();
    }

    let encoded = state.checkpoint().encode();
    let decoded = AuthoritativeCheckpoint::decode(&encoded).unwrap();
    let mut restored = MatchState::restore(decoded).unwrap();
    assert_eq!(state.authoritative_hash(), restored.authoritative_hash());
    assert_eq!(encoded, restored.checkpoint().encode());
    assert_eq!(state.terrain(), restored.terrain());
    assert_eq!(state.pose_history(), restored.pose_history());

    for tick in 40..72_u64 {
        let set = commands(
            tick,
            player(0),
            command(tick, tick as u32 + 1, -500, 1_250, false),
        );
        assert_eq!(
            state.advance_tick(set.clone()).unwrap(),
            restored.advance_tick(set).unwrap()
        );
    }
    assert_eq!(state.authoritative_hash(), restored.authoritative_hash());
}

#[test]
fn entity_slot_reuse_never_validates_stale_id() {
    let mut pool = EntityPool::new(1);
    let first = pool.spawn(Entity::new(EntityKind::Resource)).unwrap();
    assert!(pool.contains(first));
    pool.remove(first).unwrap();
    let second = pool.spawn(Entity::new(EntityKind::Resource)).unwrap();

    assert_eq!(first.index(), second.index());
    assert_ne!(first.generation(), second.generation());
    assert!(!pool.contains(first));
    assert!(pool.get(first).is_none());
    assert!(pool.contains(second));
}

#[test]
fn stale_future_and_duplicate_sequences_are_rejected_without_mutation() {
    let mut state = MatchState::new(config(), WorldSeed::new(7));
    state
        .add_player(player(0), team(0), Controller::Human)
        .unwrap();
    state
        .advance_tick(commands(0, player(0), command(0, 10, 0, 0, false)))
        .unwrap();
    let stable_hash = state.authoritative_hash();

    assert!(matches!(
        state.advance_tick(CommandSet::new(0)),
        Err(CoreError::Command(CommandRejection::StaleTick { .. }))
    ));
    assert_eq!(stable_hash, state.authoritative_hash());
    assert!(matches!(
        state.advance_tick(CommandSet::new(2)),
        Err(CoreError::Command(CommandRejection::FutureTick { .. }))
    ));
    assert_eq!(stable_hash, state.authoritative_hash());
    assert!(matches!(
        state.advance_tick(commands(1, player(0), command(1, 10, 0, 0, false))),
        Err(CoreError::Command(CommandRejection::DuplicateSequence {
            sequence: 10,
            ..
        }))
    ));
    assert_eq!(stable_hash, state.authoritative_hash());
}

#[test]
fn missing_baselines_require_resynchronization() {
    let mut state = MatchState::new(config(), WorldSeed::new(11));
    state
        .add_player(player(0), team(0), Controller::Human)
        .unwrap();
    let keyframe = state.replicate(player(0), SnapshotId::NONE).unwrap();
    let mut replica = ClientReplica::new(player(0));
    assert!(replica.apply(keyframe.clone()).unwrap());

    let missing = SnapshotId::new(keyframe.snapshot_id.get() + 100);
    assert!(matches!(
        state.replicate(player(0), missing),
        Err(ReplicationError::BaselineMissing {
            resync_required: true,
            ..
        })
    ));

    state
        .advance_tick(commands(0, player(0), command(0, 1, 500, 0, false)))
        .unwrap();
    let delta = state.replicate(player(0), keyframe.snapshot_id).unwrap();
    let mut fresh = ClientReplica::new(player(0));
    assert!(matches!(
        fresh.apply(delta),
        Err(ReplicaError::BaselineMismatch { .. })
    ));
    assert!(fresh.resync_requested());
}

#[test]
fn every_competitive_viewer_keeps_an_independent_baseline() {
    let mut state = MatchState::new(MatchConfig::competitive(), WorldSeed::new(128));
    let mut baselines = Vec::new();
    for raw in 0..128 {
        let viewer = player(raw);
        state
            .add_player(viewer, team(raw), Controller::Human)
            .unwrap();
        baselines.push(state.replicate(viewer, SnapshotId::NONE).unwrap().snapshot_id);
    }

    state.advance_tick(CommandSet::new(0)).unwrap();
    for raw in 0..128 {
        let snapshot = state.replicate(player(raw), baselines[raw as usize]);
        assert!(
            snapshot.is_ok(),
            "viewer {raw} lost its baseline to another viewer"
        );
    }
}

#[test]
fn crowded_deltas_stay_inside_the_protocol_datagram_budget() {
    let mut state = MatchState::new(MatchConfig::competitive(), WorldSeed::new(129));
    for raw in 0..128 {
        state
            .add_player(player(raw), team(raw), Controller::Human)
            .unwrap();
    }
    let baseline = state
        .replicate(player(0), SnapshotId::NONE)
        .unwrap()
        .snapshot_id;

    for tick in 0..4_u64 {
        let mut inputs = CommandSet::new(tick);
        for raw in 0..128 {
            inputs
                .insert(player(raw), command(tick, tick as u32 + 1, 2_047, 0, false))
                .unwrap();
        }
        state.advance_tick(inputs).unwrap();
    }

    let delta = state.replicate(player(0), baseline).unwrap();
    let encoded_size =
        60 + 18 + delta.entities.len() * 40 + delta.removed_entities.len() * 8;
    assert!(
        encoded_size <= 1_200,
        "replication acknowledged an unsendable {encoded_size}-byte delta"
    );
}

#[test]
fn crowded_deltas_eventually_cover_every_moving_player() {
    let mut state = MatchState::new(MatchConfig::competitive(), WorldSeed::new(130));
    for raw in 0..128 {
        state
            .add_player(player(raw), team(raw), Controller::Human)
            .unwrap();
    }
    let viewer = player(127);
    let mut baseline = state
        .replicate(viewer, SnapshotId::NONE)
        .unwrap()
        .snapshot_id;
    let mut updated_players = BTreeSet::new();

    for tick in 0..128_u64 {
        let mut inputs = CommandSet::new(tick);
        for raw in 0..128 {
            inputs
                .insert(
                    player(raw),
                    command(tick, tick as u32 + 1, 2_047, 0, false),
                )
                .unwrap();
        }
        state.advance_tick(inputs).unwrap();
        let delta = state.replicate(viewer, baseline).unwrap();
        baseline = delta.snapshot_id;
        for entity in delta.entities {
            if let Some(owner) = entity.owner {
                updated_players.insert(owner);
            }
        }
    }

    assert_eq!(
        updated_players.len(),
        128,
        "a bounded moving delta must not starve stable high entity IDs"
    );
}

#[test]
fn pose_history_is_bounded_and_rewind_is_capped_at_200_ms() {
    let mut state = MatchState::new(config(), WorldSeed::new(123));
    let entity = state
        .add_player(player(0), team(0), Controller::Human)
        .unwrap();
    for tick in 0..40_u64 {
        state
            .advance_tick(commands(
                tick,
                player(0),
                command(tick, tick as u32 + 1, 100, 0, false),
            ))
            .unwrap();
    }
    assert_eq!(state.pose_history().len(), POSE_HISTORY_TICKS);
    assert_eq!(state.pose_history().last().unwrap().tick, 39);
    assert!(state.rewind_pose(entity, 39 - MAX_REWIND_TICKS).is_ok());
    assert!(matches!(
        state.rewind_pose(entity, 39 - MAX_REWIND_TICKS - 1),
        Err(RewindError::TooOld {
            maximum_age_ticks: MAX_REWIND_TICKS,
            ..
        })
    ));
}

#[test]
fn prediction_reconciles_and_interpolation_delay_clamps() {
    let mut state = MatchState::new(config(), WorldSeed::new(81));
    state
        .add_player(player(0), team(0), Controller::Human)
        .unwrap();
    let keyframe = state.replicate(player(0), SnapshotId::NONE).unwrap();
    let mut replica = ClientReplica::new(player(0));
    replica.apply(keyframe.clone()).unwrap();
    replica.set_interpolation_delay_ticks(0);
    assert_eq!(
        replica.interpolation_delay_ticks(),
        MIN_INTERPOLATION_DELAY_TICKS
    );
    replica.set_interpolation_delay_ticks(u8::MAX);
    assert_eq!(
        replica.interpolation_delay_ticks(),
        MAX_INTERPOLATION_DELAY_TICKS
    );

    let input = command(0, 1, 2_047, 0, false);
    replica.predict_local(input);
    state
        .advance_tick(commands(0, player(0), input))
        .unwrap();
    let delta = state.replicate(player(0), keyframe.snapshot_id).unwrap();
    assert!(delta
        .entities
        .iter()
        .all(|entity| entity.interest == InterestTier::CombatCritical128Hz));
    replica.apply(delta).unwrap();
    assert_eq!(
        replica.predicted_position_cm(),
        Some(state.player(player(0)).unwrap().position_cm)
    );
}

#[test]
fn client_rejects_delta_without_baseline_and_bounds_prediction_history() {
    let viewer = player(0);
    let mut replica = ClientReplica::new(viewer);
    let delta = Snapshot {
        snapshot_id: SnapshotId::new(1),
        baseline_id: SnapshotId::NONE,
        kind: SnapshotKind::Delta,
        viewer,
        server_tick: 1,
        acknowledged_input_sequence: 0,
        entities: Vec::new(),
        removed_entities: Vec::new(),
        terrain_revisions: Vec::new(),
    };
    assert!(matches!(
        replica.apply(delta),
        Err(ReplicaError::MissingBaseline { .. })
    ));

    for index in 0..(MAX_PENDING_PREDICTION_COMMANDS + 32) {
        replica.predict_local(command(
            index as u64,
            index as u32 + 1,
            0,
            0,
            false,
        ));
    }
    assert_eq!(
        replica.pending_prediction_count(),
        MAX_PENDING_PREDICTION_COMMANDS
    );
    assert!(replica.resync_requested());

    let incomplete_ack = Snapshot {
        snapshot_id: SnapshotId::new(1),
        baseline_id: SnapshotId::NONE,
        kind: SnapshotKind::Keyframe,
        viewer,
        server_tick: 2,
        acknowledged_input_sequence: 1,
        entities: Vec::new(),
        removed_entities: Vec::new(),
        terrain_revisions: Vec::new(),
    };
    replica.apply(incomplete_ack).unwrap();
    assert!(
        replica.resync_requested(),
        "an ack older than discarded prediction history cannot repair the gap"
    );

    let complete_ack = Snapshot {
        snapshot_id: SnapshotId::new(2),
        baseline_id: SnapshotId::NONE,
        kind: SnapshotKind::Keyframe,
        viewer,
        server_tick: 3,
        acknowledged_input_sequence: 32,
        entities: Vec::new(),
        removed_entities: Vec::new(),
        terrain_revisions: Vec::new(),
    };
    replica.apply(complete_ack).unwrap();
    assert!(!replica.resync_requested());
    assert_eq!(
        replica.pending_prediction_count(),
        MAX_PENDING_PREDICTION_COMMANDS
    );
}

#[test]
fn zero_acknowledgement_keeps_pre_wrap_prediction_pending() {
    let viewer = player(0);
    let mut replica = ClientReplica::new(viewer);
    replica.predict_local(command(0, u32::MAX, 0, 0, false));
    replica
        .apply(Snapshot {
            snapshot_id: SnapshotId::new(1),
            baseline_id: SnapshotId::NONE,
            kind: SnapshotKind::Keyframe,
            viewer,
            server_tick: 0,
            acknowledged_input_sequence: 0,
            entities: Vec::new(),
            removed_entities: Vec::new(),
            terrain_revisions: Vec::new(),
        })
        .unwrap();

    assert_eq!(replica.pending_prediction_count(), 1);
}

#[test]
fn corrupt_checkpoint_is_rejected() {
    let state = MatchState::new(config(), WorldSeed::new(1));
    let mut encoded = state.checkpoint().encode();
    let last = encoded.len() - 1;
    encoded[last] ^= 0x80;
    assert!(AuthoritativeCheckpoint::decode(&encoded).is_err());
}

#[test]
fn hostile_checkpoint_capacity_fields_are_rejected_before_allocation() {
    fn checksum64(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    for field_offset in [22_usize, 26_usize] {
        let state = MatchState::new(config(), WorldSeed::new(2));
        let mut encoded = state.checkpoint().encode();
        encoded[field_offset..field_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let checksum = checksum64(&encoded[20..]);
        encoded[12..20].copy_from_slice(&checksum.to_le_bytes());
        assert!(AuthoritativeCheckpoint::decode(&encoded).is_err());
    }
}

#[test]
fn checkpoint_rejects_conflicting_player_and_entity_authority() {
    fn checksum64(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    let mut state = MatchState::new(config(), WorldSeed::new(4));
    state
        .add_player(player(0), team(0), Controller::Human)
        .unwrap();
    let mut encoded = state.checkpoint().encode();
    // Checkpoint v1: the first player's health follows the fixed 20-byte
    // envelope and 107 bytes of authoritative payload fields.
    encoded[127..129].copy_from_slice(&50_u16.to_le_bytes());
    let checksum = checksum64(&encoded[20..]);
    encoded[12..20].copy_from_slice(&checksum.to_le_bytes());

    assert!(AuthoritativeCheckpoint::decode(&encoded).is_err());
}

#[test]
fn checkpoint_rejects_ghost_or_out_of_range_entity_owners() {
    fn checksum64(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    let mut state = MatchState::new(config(), WorldSeed::new(5));
    state
        .add_player(player(0), team(0), Controller::Human)
        .unwrap();
    state
        .advance_tick(commands(0, player(0), command(0, 1, 0, 0, true)))
        .unwrap();
    let encoded = state.checkpoint().encode();
    // With two occupied entity slots, the second dense entity begins at byte
    // 723 in checkpoint v1. An owner beyond this match's eight slots could
    // later index the player array during damage attribution.
    let mut out_of_range_owner = encoded.clone();
    out_of_range_owner[732..734].copy_from_slice(&127_u16.to_le_bytes());
    let checksum = checksum64(&out_of_range_owner[20..]);
    out_of_range_owner[12..20].copy_from_slice(&checksum.to_le_bytes());
    assert!(AuthoritativeCheckpoint::decode(&out_of_range_owner).is_err());

    // Turning that projectile into another player used to create an unowned
    // ghost outside the player-slot authority mapping.
    let mut ghost_player = encoded;
    ghost_player[731] = EntityKind::Player as u8;
    let checksum = checksum64(&ghost_player[20..]);
    ghost_player[12..20].copy_from_slice(&checksum.to_le_bytes());
    assert!(AuthoritativeCheckpoint::decode(&ghost_player).is_err());
}

#[test]
fn checkpoint_rejects_noncanonical_and_inconsistent_sequence_state() {
    fn checksum64(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    let mut state = MatchState::new(config(), WorldSeed::new(6));
    state
        .add_player(player(0), team(0), Controller::Bot)
        .unwrap();
    let encoded = state.checkpoint().encode();

    let mut absent_but_nonzero = encoded.clone();
    absent_but_nonzero[157..161].copy_from_slice(&1_u32.to_le_bytes());
    let checksum = checksum64(&absent_but_nonzero[20..]);
    absent_but_nonzero[12..20].copy_from_slice(&checksum.to_le_bytes());
    assert!(AuthoritativeCheckpoint::decode(&absent_but_nonzero).is_err());

    let mut inconsistent_bot_next = encoded;
    inconsistent_bot_next[161..165].copy_from_slice(&2_u32.to_le_bytes());
    let checksum = checksum64(&inconsistent_bot_next[20..]);
    inconsistent_bot_next[12..20].copy_from_slice(&checksum.to_le_bytes());
    assert!(AuthoritativeCheckpoint::decode(&inconsistent_bot_next).is_err());
}

#[test]
fn bot_takeover_continues_the_human_input_sequence() {
    let mut state = MatchState::new(config(), WorldSeed::new(3));
    state
        .add_player(player(0), team(0), Controller::Human)
        .unwrap();
    state
        .advance_tick(commands(0, player(0), command(0, 100, 0, 0, false)))
        .unwrap();
    state.set_controller(player(0), Controller::Bot).unwrap();
    state.advance_tick(CommandSet::new(1)).unwrap();

    assert_eq!(
        state.player(player(0)).unwrap().last_accepted_sequence(),
        Some(101)
    );
}

#[test]
fn authoritative_sequences_wrap_from_maximum_to_one() {
    let mut state = MatchState::new(config(), WorldSeed::new(7));
    state
        .add_player(player(0), team(0), Controller::Human)
        .unwrap();
    state
        .advance_tick(commands(
            0,
            player(0),
            command(0, u32::MAX, 0, 0, false),
        ))
        .unwrap();
    state.set_controller(player(0), Controller::Bot).unwrap();
    state.advance_tick(CommandSet::new(1)).unwrap();

    assert_eq!(
        state.player(player(0)).unwrap().last_accepted_sequence(),
        Some(1)
    );
    let checkpoint = state.checkpoint().encode();
    assert!(AuthoritativeCheckpoint::decode(&checkpoint).is_ok());
}

#[test]
fn replica_retains_bounded_remote_history_and_interpolates_at_adaptive_delay() {
    fn remote(entity_id: aetherloom_core::EntityId, x: i32) -> ReplicatedEntity {
        ReplicatedEntity {
            entity_id,
            kind: EntityKind::Player,
            owner: Some(player(1)),
            team: Some(team(1)),
            position_cm: [x, 0, 0],
            velocity_cm_per_tick: [0; 3],
            yaw: 0,
            pitch: 0,
            health: 100,
            flags: 0,
            interest: InterestTier::CombatCritical128Hz,
            scheduled_hz: 128,
        }
    }

    let entity_id = aetherloom_core::EntityId::new(3, 1).unwrap();
    let mut replica = ClientReplica::new(player(0));
    replica
        .apply(Snapshot {
            snapshot_id: SnapshotId::new(1),
            baseline_id: SnapshotId::NONE,
            kind: SnapshotKind::Keyframe,
            viewer: player(0),
            server_tick: 100,
            acknowledged_input_sequence: 0,
            entities: vec![remote(entity_id, 0)],
            removed_entities: Vec::new(),
            terrain_revisions: Vec::new(),
        })
        .unwrap();
    replica
        .apply(Snapshot {
            snapshot_id: SnapshotId::new(2),
            baseline_id: SnapshotId::new(1),
            kind: SnapshotKind::Delta,
            viewer: player(0),
            server_tick: 104,
            acknowledged_input_sequence: 0,
            entities: vec![remote(entity_id, 40)],
            removed_entities: Vec::new(),
            terrain_revisions: Vec::new(),
        })
        .unwrap();

    replica.set_interpolation_delay_ticks(2);
    assert_eq!(
        replica.interpolated_position_cm(entity_id, 0),
        Some([20, 0, 0])
    );

    for offset in 1..=MAX_INTERPOLATION_SAMPLES + 3 {
        let snapshot_id = 2 + offset as u32;
        replica
            .apply(Snapshot {
                snapshot_id: SnapshotId::new(snapshot_id),
                baseline_id: SnapshotId::new(snapshot_id - 1),
                kind: SnapshotKind::Delta,
                viewer: player(0),
                server_tick: 104 + offset as u64,
                acknowledged_input_sequence: 0,
                entities: vec![remote(entity_id, 40 + offset as i32 * 10)],
                removed_entities: Vec::new(),
                terrain_revisions: Vec::new(),
            })
            .unwrap();
    }
    assert_eq!(
        replica.interpolation_samples().len(),
        MAX_INTERPOLATION_SAMPLES
    );
}

#[test]
fn replica_rejects_newer_snapshot_ids_that_regress_server_time() {
    let mut replica = ClientReplica::new(player(0));
    replica
        .apply(Snapshot {
            snapshot_id: SnapshotId::new(1),
            baseline_id: SnapshotId::NONE,
            kind: SnapshotKind::Keyframe,
            viewer: player(0),
            server_tick: 50,
            acknowledged_input_sequence: 0,
            entities: Vec::new(),
            removed_entities: Vec::new(),
            terrain_revisions: Vec::new(),
        })
        .unwrap();
    assert_eq!(
        replica.apply(Snapshot {
            snapshot_id: SnapshotId::new(2),
            baseline_id: SnapshotId::new(1),
            kind: SnapshotKind::Delta,
            viewer: player(0),
            server_tick: 49,
            acknowledged_input_sequence: 0,
            entities: Vec::new(),
            removed_entities: Vec::new(),
            terrain_revisions: Vec::new(),
        }),
        Err(ReplicaError::ServerTickRegression {
            current: 50,
            received: 49,
        })
    );
    assert!(replica.resync_requested());
}
