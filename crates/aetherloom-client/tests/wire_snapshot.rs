use aetherloom_client::{
    apply_wire_snapshot, ClientReplica, WireSnapshotError,
};
use aetherloom_protocol::{
    EntityId, EntityState, EnvelopeMetadata, Message, MessageEnvelope, PlayerId,
    SnapshotDelta, SnapshotId, SnapshotKeyframe,
};

fn viewer() -> PlayerId {
    PlayerId::new(0).expect("viewer")
}

fn metadata(server_tick: u64) -> EnvelopeMetadata {
    EnvelopeMetadata::new([9; 16], 4, server_tick as u32, server_tick, 0)
}

fn entity(archetype: u16, x: i32) -> EntityState {
    EntityState {
        entity_id: EntityId::new(3, 1).expect("entity"),
        archetype,
        owner: Some(viewer()),
        team: None,
        position_cm: [x, 0, 20],
        velocity_cm_per_tick: [1, 0, 0],
        yaw: 7,
        pitch: -2,
        health: 90,
        flags: 5,
    }
}

#[test]
fn wire_keyframes_and_deltas_drive_the_core_replica_with_server_tick() {
    let mut replica = ClientReplica::new(viewer());
    let keyframe = MessageEnvelope::new(
        metadata(40),
        Message::SnapshotKeyframe(SnapshotKeyframe {
            snapshot_id: SnapshotId::new(1),
            viewer: viewer(),
            acknowledged_input_sequence: 7,
            spell_cooldown_ticks: [0, 0, 0, 0, 0, 0, 0, 44, 0, 0, 0, 0, 0],
            entities: vec![entity(1, 100)],
            terrain_revisions: Vec::new(),
            terrain_chunks: Vec::new(),
        }),
    );
    assert_eq!(apply_wire_snapshot(&mut replica, &keyframe), Ok(true));
    assert_eq!(replica.server_tick(), 40);
    assert_eq!(replica.snapshot_id(), SnapshotId::new(1));
    assert_eq!(replica.entities()[0].position_cm, [100, 0, 20]);
    assert_eq!(replica.spell_cooldown_ticks()[7], 44);

    let delta = MessageEnvelope::new(
        metadata(41),
        Message::SnapshotDelta(SnapshotDelta {
            snapshot_id: SnapshotId::new(2),
            baseline_id: SnapshotId::new(1),
            viewer: viewer(),
            acknowledged_input_sequence: 8,
            spell_cooldown_ticks: [9, 0, 0, 0, 0, 0, 0, 43, 0, 0, 0, 0, 0],
            entities: vec![entity(1, 125)],
            removed_entities: Vec::new(),
        }),
    );
    assert_eq!(apply_wire_snapshot(&mut replica, &delta), Ok(true));
    assert_eq!(replica.server_tick(), 41);
    assert_eq!(replica.entities()[0].position_cm, [125, 0, 20]);
    assert_eq!(replica.spell_cooldown_ticks()[0], 9);
    assert_eq!(replica.spell_cooldown_ticks()[7], 43);
}

#[test]
fn wire_bridge_rejects_unknown_archetypes_before_mutating_replica() {
    let envelope = MessageEnvelope::new(
        metadata(1),
        Message::SnapshotKeyframe(SnapshotKeyframe {
            snapshot_id: SnapshotId::new(1),
            viewer: viewer(),
            acknowledged_input_sequence: 0,
            spell_cooldown_ticks: [0; 13],
            entities: vec![entity(99, 0)],
            terrain_revisions: Vec::new(),
            terrain_chunks: Vec::new(),
        }),
    );
    let mut replica = ClientReplica::new(viewer());
    assert_eq!(
        apply_wire_snapshot(&mut replica, &envelope),
        Err(WireSnapshotError::InvalidArchetype(99))
    );
    assert!(replica.entities().is_empty());
}
