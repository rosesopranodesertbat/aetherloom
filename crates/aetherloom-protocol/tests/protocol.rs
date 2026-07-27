use aetherloom_protocol::{
    CommandSet, Controller, Delivery, EntityId, EntityState, EnvelopeMetadata, EventBatch,
    EventKind, GameEvent, InputBatch, LootEntry, MatchOutcome, MatchResult, Message,
    MessageEnvelope, PlayerCommand, PlayerId, PlayerMatchResult, ProtocolError, SnapshotDelta,
    SnapshotId, SnapshotKeyframe, TeamId, TerrainDelta, TerrainOp, TerrainOpKind, ValidationError,
    ACTION_CAST, AUTHORITATIVE_HZ, AUTHORITATIVE_TICK_NANOS, HEADER_BYTES, MAX_DATAGRAM_BYTES,
    PROTOCOL_MAGIC, PROTOCOL_VERSION,
};

fn metadata() -> EnvelopeMetadata {
    EnvelopeMetadata::new([0xAB; 16], 0x0102_0304_0506_0708, 0x1122_3344, 900, 896)
}

fn command(tick: u64, sequence: u32) -> PlayerCommand {
    PlayerCommand::new(
        tick,
        sequence,
        1_024,
        -512,
        49_152,
        -4_096,
        ACTION_CAST,
        Some(3),
    )
    .unwrap()
}

fn input_envelope(commands: Vec<PlayerCommand>) -> MessageEnvelope {
    MessageEnvelope::new(
        metadata(),
        Message::InputBatch(InputBatch::new(commands).unwrap()),
    )
}

fn entity(index: u32) -> EntityState {
    EntityState {
        entity_id: EntityId::new(index, 1).unwrap(),
        archetype: 2,
        owner: Some(PlayerId::new((index % 128) as u16).unwrap()),
        team: Some(TeamId::new((index % 32) as u16).unwrap()),
        position_cm: [index as i32 * 10, -250, 4_096],
        velocity_cm_per_tick: [12, -7, 0],
        yaw: 12_345,
        pitch: -1_234,
        health: 900,
        flags: 3,
    }
}

#[test]
fn constants_describe_exact_128_hz_tick() {
    assert_eq!(AUTHORITATIVE_HZ, 128);
    assert_eq!(AUTHORITATIVE_TICK_NANOS * AUTHORITATIVE_HZ as u64, 1_000_000_000);
    assert_eq!(MAX_DATAGRAM_BYTES, 1_200);
}

#[test]
fn typed_ids_and_controller_reject_out_of_range_values() {
    assert_eq!(PlayerId::new(127).unwrap().index(), 127);
    assert_eq!(
        PlayerId::new(128),
        Err(ValidationError::InvalidPlayerId(128))
    );
    assert_eq!(
        TeamId::new(128),
        Err(ValidationError::InvalidTeamId(128))
    );
    assert_eq!(
        EntityId::new(4, 0),
        Err(ValidationError::InvalidEntityId(4))
    );
    assert_eq!(Controller::try_from(2), Ok(Controller::Bot));
    assert_eq!(
        Controller::try_from(4),
        Err(ValidationError::InvalidController(4))
    );
}

#[test]
fn entity_generation_prevents_slot_aliasing() {
    let old = EntityId::new(42, 1).unwrap();
    let replacement = EntityId::new(42, 2).unwrap();
    assert_ne!(old, replacement);
    assert_eq!(old.index(), replacement.index());
    assert_ne!(old.generation(), replacement.generation());
}

#[test]
fn player_command_rejects_out_of_range_quantized_fields() {
    assert_eq!(
        PlayerCommand::new(1, 0, 0, 0, 0, 0, 0, None),
        Err(ValidationError::ZeroCommandSequence)
    );
    assert!(matches!(
        PlayerCommand::new(1, 1, 2_048, 0, 0, 0, 0, None),
        Err(ValidationError::MoveAxisOutOfRange {
            axis: "move_x",
            value: 2_048
        })
    ));
    assert_eq!(
        PlayerCommand::new(1, 1, 0, 0, 0, 16_385, 0, None),
        Err(ValidationError::LookPitchOutOfRange(16_385))
    );
    assert_eq!(
        PlayerCommand::new(1, 1, 0, 0, 0, 0, 0x8000, None),
        Err(ValidationError::UnsupportedActionFlags(0x8000))
    );
    assert_eq!(
        PlayerCommand::new(1, 1, 0, 0, 0, 0, 0, Some(32)),
        Err(ValidationError::SpellOutOfRange(32))
    );
}

#[test]
fn command_set_is_bounded_deterministic_and_rejects_duplicates() {
    let mut set = CommandSet::new(50);
    set.insert(PlayerId::new(9).unwrap(), command(50, 90))
        .unwrap();
    set.insert(PlayerId::new(2).unwrap(), command(50, 20))
        .unwrap();
    assert_eq!(
        set.insert(PlayerId::new(2).unwrap(), command(50, 21)),
        Err(ValidationError::DuplicatePlayer(2))
    );
    assert_eq!(
        set.insert(PlayerId::new(3).unwrap(), command(51, 30)),
        Err(ValidationError::CommandTickMismatch {
            expected: 50,
            actual: 51
        })
    );
    let ids: Vec<_> = set.iter().map(|(id, _)| id.get()).collect();
    assert_eq!(ids, vec![2, 9]);
}

#[test]
fn input_batch_rejects_duplicate_and_misordered_commands() {
    let duplicate_sequence = InputBatch::new(
        vec![command(12, 5), command(11, 5)],
    );
    assert_eq!(
        duplicate_sequence,
        Err(ValidationError::DuplicateCommandSequence(5))
    );

    let duplicate_tick = InputBatch::new(
        vec![command(12, 5), command(12, 4)],
    );
    assert_eq!(
        duplicate_tick,
        Err(ValidationError::DuplicateCommandTick(12))
    );

    let reversed = InputBatch::new(
        vec![command(11, 4), command(12, 5)],
    );
    assert_eq!(
        reversed,
        Err(ValidationError::CommandsNotNewestFirst)
    );

    let wrapped = InputBatch::new(
        vec![command(12, 1), command(11, u32::MAX)],
    );
    assert!(wrapped.is_ok(), "sequence wrap must retain newest-first ordering");
}

#[test]
fn input_datagram_round_trips_and_is_explicit_little_endian() {
    let envelope = input_envelope(vec![command(102, 202), command(101, 201), command(100, 200)]);
    let bytes = envelope.encode_datagram().unwrap();
    assert!(bytes.len() <= MAX_DATAGRAM_BYTES);
    assert_eq!(&bytes[0..4], &PROTOCOL_MAGIC);
    assert_eq!(&bytes[4..6], &PROTOCOL_VERSION.to_le_bytes());
    assert_eq!(&bytes[6..8], &(HEADER_BYTES as u16).to_le_bytes());
    assert_eq!(&bytes[28..36], &metadata().match_epoch.to_le_bytes());
    assert_eq!(&bytes[36..40], &metadata().sequence.to_le_bytes());
    assert_eq!(&bytes[40..48], &metadata().server_tick.to_le_bytes());
    assert_eq!(
        &bytes[48..56],
        &metadata().acknowledgement_tick.to_le_bytes()
    );
    assert_eq!(
        &bytes[HEADER_BYTES + 1..HEADER_BYTES + 9],
        &102u64.to_le_bytes()
    );
    assert_eq!(MessageEnvelope::decode_datagram(&bytes).unwrap(), envelope);
}

#[test]
fn input_datagram_contains_commands_but_no_identity_claim() {
    let bytes = input_envelope(vec![command(10, 20)])
        .encode_datagram()
        .unwrap();
    const ENCODED_COMMAND_BYTES: usize = 23;
    assert_eq!(bytes.len(), HEADER_BYTES + 1 + ENCODED_COMMAND_BYTES);
    assert_eq!(bytes[HEADER_BYTES], 1, "first payload byte is command count");
}

#[test]
fn decoder_rejects_malformed_headers_and_payloads() {
    let envelope = input_envelope(vec![command(10, 20)]);
    let original = envelope.encode_datagram().unwrap();

    let mut wrong_version = original.clone();
    wrong_version[4..6].copy_from_slice(&99u16.to_le_bytes());
    assert_eq!(
        MessageEnvelope::decode_datagram(&wrong_version),
        Err(ProtocolError::UnsupportedVersion(99))
    );

    let mut unknown_kind = original.clone();
    unknown_kind[8] = 99;
    assert_eq!(
        MessageEnvelope::decode_datagram(&unknown_kind),
        Err(ProtocolError::UnknownMessageKind(99))
    );

    let mut reserved_flags = original.clone();
    reserved_flags[10..12].copy_from_slice(&1u16.to_le_bytes());
    assert_eq!(
        MessageEnvelope::decode_datagram(&reserved_flags),
        Err(ProtocolError::NonZeroReservedFlags(1))
    );

    let truncated = &original[..original.len() - 1];
    assert!(matches!(
        MessageEnvelope::decode_datagram(truncated),
        Err(ProtocolError::PayloadLengthMismatch { .. })
    ));
}

#[test]
fn decoder_rejects_out_of_range_axis() {
    let mut bad_axis = input_envelope(vec![command(10, 20)])
        .encode_datagram()
        .unwrap();
    let move_x_offset = HEADER_BYTES + 1 + 8 + 4;
    bad_axis[move_x_offset..move_x_offset + 2].copy_from_slice(&i16::MAX.to_le_bytes());
    assert_eq!(
        MessageEnvelope::decode_datagram(&bad_axis),
        Err(ProtocolError::Validation(
            ValidationError::MoveAxisOutOfRange {
                axis: "move_x",
                value: i16::MAX
            }
        ))
    );
}

#[test]
fn decoder_rejects_duplicate_redundant_commands() {
    let mut bytes = input_envelope(vec![command(10, 20), command(9, 19)])
        .encode_datagram()
        .unwrap();
    let first_sequence_offset = HEADER_BYTES + 1 + 8;
    let command_bytes = 23;
    let second_sequence_offset = first_sequence_offset + command_bytes;
    let sequence = bytes[first_sequence_offset..first_sequence_offset + 4].to_vec();
    bytes[second_sequence_offset..second_sequence_offset + 4].copy_from_slice(&sequence);
    assert_eq!(
        MessageEnvelope::decode_datagram(&bytes),
        Err(ProtocolError::Validation(
            ValidationError::DuplicateCommandSequence(20)
        ))
    );
}

#[test]
fn datagram_limit_is_enforced_on_encode_and_decode() {
    let delta = SnapshotDelta {
        snapshot_id: SnapshotId::new(5),
        baseline_id: SnapshotId::new(4),
        viewer: PlayerId::new(0).unwrap(),
        acknowledged_input_sequence: 10,
        entities: (0..32).map(entity).collect(),
        removed_entities: Vec::new(),
    };
    let envelope = MessageEnvelope::new(metadata(), Message::SnapshotDelta(delta));
    assert!(matches!(
        envelope.encode_datagram(),
        Err(ProtocolError::DatagramTooLarge {
            max: MAX_DATAGRAM_BYTES,
            ..
        })
    ));

    let oversized = vec![0; MAX_DATAGRAM_BYTES + 1];
    assert_eq!(
        MessageEnvelope::decode_datagram(&oversized),
        Err(ProtocolError::DatagramTooLarge {
            len: MAX_DATAGRAM_BYTES + 1,
            max: MAX_DATAGRAM_BYTES
        })
    );
}

#[test]
fn small_snapshot_delta_round_trips_as_a_datagram() {
    let delta = SnapshotDelta {
        snapshot_id: SnapshotId::new(5),
        baseline_id: SnapshotId::new(4),
        viewer: PlayerId::new(0).unwrap(),
        acknowledged_input_sequence: 10,
        entities: vec![entity(1), entity(2)],
        removed_entities: vec![EntityId::new(3, 1).unwrap()],
    };
    let envelope = MessageEnvelope::new(metadata(), Message::SnapshotDelta(delta));
    let bytes = envelope.encode_datagram().unwrap();
    assert_eq!(MessageEnvelope::decode_datagram(&bytes).unwrap(), envelope);
}

#[test]
fn snapshot_rejects_duplicate_entity_across_updated_and_removed_lists() {
    let repeated = entity(4);
    let delta = SnapshotDelta {
        snapshot_id: SnapshotId::new(2),
        baseline_id: SnapshotId::new(1),
        viewer: PlayerId::new(0).unwrap(),
        acknowledged_input_sequence: 9,
        entities: vec![repeated.clone()],
        removed_entities: vec![repeated.entity_id],
    };
    assert_eq!(
        delta.validate(),
        Err(ValidationError::DuplicateEntity(repeated.entity_id.get()))
    );
}

#[test]
fn snapshot_delta_requires_a_real_baseline() {
    let delta = SnapshotDelta {
        snapshot_id: SnapshotId::new(2),
        baseline_id: SnapshotId::NONE,
        viewer: PlayerId::new(0).unwrap(),
        acknowledged_input_sequence: 9,
        entities: vec![entity(1)],
        removed_entities: Vec::new(),
    };
    assert_eq!(
        delta.validate(),
        Err(ValidationError::InvalidBaseline {
            snapshot: 2,
            baseline: 0
        })
    );
}

#[test]
fn reliable_keyframe_round_trips_and_rejects_datagram_channel() {
    let keyframe = SnapshotKeyframe {
        snapshot_id: SnapshotId::new(55),
        viewer: PlayerId::new(3).unwrap(),
        acknowledged_input_sequence: 80,
        entities: vec![entity(1), entity(2)],
        terrain_revisions: vec![
            aetherloom_protocol::ChunkRevision {
                x: -2,
                y: 5,
                revision: 9,
            },
            aetherloom_protocol::ChunkRevision {
                x: -1,
                y: 5,
                revision: 3,
            },
        ],
        terrain_chunks: vec![
            aetherloom_protocol::TerrainChunkState {
                x: -2,
                y: 5,
                revision: 9,
                heights_cm: vec![
                    -12;
                    aetherloom_protocol::TERRAIN_CELLS_PER_CHUNK
                ],
            },
            aetherloom_protocol::TerrainChunkState {
                x: -1,
                y: 5,
                revision: 3,
                heights_cm: vec![
                    4;
                    aetherloom_protocol::TERRAIN_CELLS_PER_CHUNK
                ],
            },
        ],
    };
    let envelope = MessageEnvelope::new(metadata(), Message::SnapshotKeyframe(keyframe));
    assert_eq!(
        envelope.encode_datagram(),
        Err(ProtocolError::MessageNotAllowedOnDelivery {
            kind: 3,
            delivery: Delivery::Datagram as u8
        })
    );
    let bytes = envelope.encode_reliable().unwrap();
    assert_eq!(MessageEnvelope::decode_reliable(&bytes).unwrap(), envelope);
    assert_eq!(
        MessageEnvelope::decode_datagram(&bytes),
        Err(ProtocolError::WrongDelivery {
            expected: Delivery::Datagram as u8,
            actual: Delivery::Reliable as u8
        })
    );
}

#[test]
fn keyframe_requires_matching_complete_terrain_chunks() {
    let revision = aetherloom_protocol::ChunkRevision {
        x: 4,
        y: -8,
        revision: 2,
    };
    let mut keyframe = SnapshotKeyframe {
        snapshot_id: SnapshotId::new(1),
        viewer: PlayerId::new(0).unwrap(),
        acknowledged_input_sequence: 0,
        entities: Vec::new(),
        terrain_revisions: vec![revision],
        terrain_chunks: Vec::new(),
    };
    assert_eq!(
        keyframe.validate(),
        Err(ValidationError::TerrainKeyframeMismatch)
    );

    keyframe.terrain_chunks.push(
        aetherloom_protocol::TerrainChunkState {
            x: revision.x,
            y: revision.y,
            revision: revision.revision,
            heights_cm: vec![
                0;
                aetherloom_protocol::TERRAIN_CELLS_PER_CHUNK - 1
            ],
        },
    );
    assert_eq!(
        keyframe.validate(),
        Err(ValidationError::InvalidTerrainCellCount {
            count: aetherloom_protocol::TERRAIN_CELLS_PER_CHUNK - 1,
            expected: aetherloom_protocol::TERRAIN_CELLS_PER_CHUNK,
        })
    );
}

#[test]
fn event_batch_rejects_duplicate_event_ids() {
    let event = GameEvent {
        event_id: 77,
        tick: 9,
        kind: EventKind::HIT,
        actor: Some(EntityId::new(1, 1).unwrap()),
        target: Some(EntityId::new(2, 1).unwrap()),
        data: [10, 20, 30, 40],
    };
    let batch = EventBatch {
        events: vec![event.clone(), event],
    };
    assert_eq!(batch.validate(), Err(ValidationError::DuplicateEvent(77)));
}

#[test]
fn event_batch_round_trips_on_both_delivery_classes() {
    let event = GameEvent {
        event_id: 77,
        tick: 9,
        kind: EventKind::HIT,
        actor: Some(EntityId::new(1, 1).unwrap()),
        target: Some(EntityId::new(2, 1).unwrap()),
        data: [10, 20, 30, 40],
    };
    let envelope =
        MessageEnvelope::new(metadata(), Message::EventBatch(EventBatch { events: vec![event] }));
    let datagram = envelope.encode_datagram().unwrap();
    assert_eq!(
        MessageEnvelope::decode_datagram(&datagram).unwrap(),
        envelope
    );
    let reliable = envelope.encode_reliable().unwrap();
    assert_eq!(
        MessageEnvelope::decode_reliable(&reliable).unwrap(),
        envelope
    );
}

#[test]
fn maximum_event_batch_fits_the_gameplay_path_mtu() {
    let events = (1..=aetherloom_protocol::MAX_EVENTS_PER_BATCH as u64)
        .map(|event_id| GameEvent {
            event_id,
            tick: 99,
            kind: EventKind::DAMAGE,
            actor: Some(EntityId::new(1, 1).unwrap()),
            target: Some(EntityId::new(2, 1).unwrap()),
            data: [20, 80, 0, 0],
        })
        .collect();
    let envelope =
        MessageEnvelope::new(metadata(), Message::EventBatch(EventBatch { events }));
    let datagram = envelope.encode_datagram().expect("bounded event datagram");
    assert!(datagram.len() <= aetherloom_protocol::MAX_DATAGRAM_BYTES);
}

#[test]
fn terrain_delta_rejects_bad_revision_cells_and_duplicates() {
    let operation = TerrainOp {
        kind: TerrainOpKind::Crater,
        cell_x: 5,
        cell_y: 7,
        height_delta_cm: -120,
        material: 2,
        flags: 0,
    };
    let bad_revision = TerrainDelta {
        chunk_x: 0,
        chunk_y: 0,
        base_revision: 4,
        new_revision: 4,
        operations: vec![operation],
    };
    assert_eq!(
        bad_revision.validate(),
        Err(ValidationError::InvalidTerrainRevision { base: 4, new: 4 })
    );

    let duplicate = TerrainDelta {
        chunk_x: 0,
        chunk_y: 0,
        base_revision: 4,
        new_revision: 5,
        operations: vec![operation, operation],
    };
    assert_eq!(
        duplicate.validate(),
        Err(ValidationError::DuplicateTerrainCell { x: 5, y: 7 })
    );

    let invalid_cell = TerrainOp {
        cell_x: 8,
        ..operation
    };
    assert_eq!(
        invalid_cell.validate(),
        Err(ValidationError::InvalidTerrainCell { x: 8, y: 7 })
    );

    let wrapped_revision = TerrainDelta {
        chunk_x: 0,
        chunk_y: 0,
        base_revision: u32::MAX,
        new_revision: 1,
        operations: vec![operation],
    };
    assert_eq!(wrapped_revision.validate(), Ok(()));

    let zero_revision = TerrainDelta {
        new_revision: 0,
        ..wrapped_revision
    };
    assert_eq!(
        zero_revision.validate(),
        Err(ValidationError::InvalidTerrainRevision {
            base: u32::MAX,
            new: 0,
        })
    );

    let too_many_operations = TerrainDelta {
        chunk_x: 0,
        chunk_y: 0,
        base_revision: 0,
        new_revision: 1,
        operations: vec![operation; 65],
    };
    assert!(matches!(
        too_many_operations.validate(),
        Err(ValidationError::TooManyItems {
            field: "terrain operations",
            count: 65,
            max: 64,
        })
    ));
}

#[test]
fn terrain_delta_round_trips_reliably() {
    let delta = TerrainDelta {
        chunk_x: -4,
        chunk_y: 8,
        base_revision: 4,
        new_revision: 5,
        operations: vec![TerrainOp {
            kind: TerrainOpKind::Crater,
            cell_x: 5,
            cell_y: 7,
            height_delta_cm: -120,
            material: 2,
            flags: 0,
        }],
    };
    let envelope = MessageEnvelope::new(metadata(), Message::TerrainDelta(delta));
    let bytes = envelope.encode_reliable().unwrap();
    assert_eq!(MessageEnvelope::decode_reliable(&bytes).unwrap(), envelope);
}

#[test]
fn match_result_is_idempotent_and_rejects_duplicate_players_and_loot() {
    let player = PlayerMatchResult {
        player_id: PlayerId::new(1).unwrap(),
        team_id: TeamId::new(0).unwrap(),
        outcome: MatchOutcome::Extracted,
        rating_delta: 12,
        score: 900,
        banked_loot: vec![LootEntry {
            item_id: 5,
            quantity: 3,
        }],
    };
    let duplicate_players = MatchResult {
        result_id: [1; 16],
        match_id: [2; 16],
        completed_tick: 50_000,
        players: vec![player.clone(), player.clone()],
    };
    assert_eq!(
        duplicate_players.validate(),
        Err(ValidationError::DuplicateResultPlayer(1))
    );

    let duplicate_loot = MatchResult {
        result_id: [1; 16],
        match_id: [2; 16],
        completed_tick: 50_000,
        players: vec![PlayerMatchResult {
            banked_loot: vec![
                LootEntry {
                    item_id: 5,
                    quantity: 1,
                },
                LootEntry {
                    item_id: 5,
                    quantity: 2,
                },
            ],
            ..player
        }],
    };
    assert_eq!(
        duplicate_loot.validate(),
        Err(ValidationError::DuplicateLootItem(5))
    );
}

#[test]
fn match_result_round_trips_reliably() {
    let result = MatchResult {
        result_id: [1; 16],
        match_id: [2; 16],
        completed_tick: 50_000,
        players: vec![PlayerMatchResult {
            player_id: PlayerId::new(1).unwrap(),
            team_id: TeamId::new(0).unwrap(),
            outcome: MatchOutcome::Extracted,
            rating_delta: 12,
            score: 900,
            banked_loot: vec![LootEntry {
                item_id: 5,
                quantity: 3,
            }],
        }],
    };
    let envelope = MessageEnvelope::new(metadata(), Message::MatchResult(result));
    let bytes = envelope.encode_reliable().unwrap();
    assert_eq!(MessageEnvelope::decode_reliable(&bytes).unwrap(), envelope);
}
