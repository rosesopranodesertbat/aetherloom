use aetherloom_client::{
    AuthoritativeFeedError, AuthoritativeFeedOutcome,
    AuthoritativeFeedReplica,
};
use aetherloom_protocol::{
    ChunkRevision, EventBatch, EventKind, GameEvent, PlayerId, SnapshotId,
    SnapshotKeyframe, TerrainChunkState, TerrainDelta, TerrainOp, TerrainOpKind,
    TERRAIN_CELLS_PER_CHUNK,
};

fn viewer() -> PlayerId {
    PlayerId::new(0).expect("viewer")
}

fn keyframe(revision: u32, height: i16) -> SnapshotKeyframe {
    SnapshotKeyframe {
        snapshot_id: SnapshotId::new(revision),
        viewer: viewer(),
        acknowledged_input_sequence: 0,
        spell_cooldown_ticks: [0; 13],
        entities: Vec::new(),
        terrain_revisions: vec![ChunkRevision {
            x: 2,
            y: -3,
            revision,
        }],
        terrain_chunks: vec![TerrainChunkState {
            x: 2,
            y: -3,
            revision,
            heights_cm: vec![height; TERRAIN_CELLS_PER_CHUNK],
        }],
    }
}

#[test]
fn full_keyframe_recovers_a_missed_terrain_revision() {
    let mut replica =
        AuthoritativeFeedReplica::with_capacities(viewer(), 2, 4).expect("limits");
    replica.apply_keyframe(&keyframe(4, 10)).expect("keyframe");
    let gap = TerrainDelta {
        chunk_x: 2,
        chunk_y: -3,
        base_revision: 5,
        new_revision: 6,
        operations: vec![TerrainOp {
            kind: TerrainOpKind::HeightDelta,
            cell_x: 1,
            cell_y: 2,
            height_delta_cm: -8,
            material: 0,
            flags: 0,
        }],
    };
    assert_eq!(
        replica.apply_terrain_delta(&gap),
        Err(AuthoritativeFeedError::TerrainRevisionGap {
            x: 2,
            y: -3,
            current: 4,
            base: 5,
            new: 6,
        })
    );
    assert!(replica.resync_requested());

    assert_eq!(
        replica.apply_keyframe(&keyframe(6, -20)),
        Ok(AuthoritativeFeedOutcome::KeyframeApplied)
    );
    assert!(!replica.resync_requested());
    let chunk = replica.terrain_chunk(2, -3).expect("recovered chunk");
    assert_eq!(chunk.revision, 6);
    assert!(chunk.heights_cm.iter().all(|height| *height == -20));
}

#[test]
fn terrain_deltas_and_event_ids_are_applied_once_with_bounded_storage() {
    let mut replica =
        AuthoritativeFeedReplica::with_capacities(viewer(), 1, 2).expect("limits");
    let delta = TerrainDelta {
        chunk_x: 0,
        chunk_y: 0,
        base_revision: 0,
        new_revision: 1,
        operations: vec![TerrainOp {
            kind: TerrainOpKind::Crater,
            cell_x: 3,
            cell_y: 4,
            height_delta_cm: -7,
            material: 0,
            flags: 0,
        }],
    };
    assert_eq!(
        replica.apply_terrain_delta(&delta),
        Ok(AuthoritativeFeedOutcome::TerrainApplied)
    );
    assert_eq!(
        replica.apply_terrain_delta(&delta),
        Ok(AuthoritativeFeedOutcome::Obsolete)
    );
    let chunk = replica.terrain_chunk(0, 0).expect("chunk");
    assert_eq!(chunk.revision, 1);
    assert_eq!(chunk.heights_cm[4 * 8 + 3], -7);

    let event = |event_id| GameEvent {
        event_id,
        tick: event_id,
        kind: EventKind::DAMAGE,
        actor: None,
        target: None,
        data: [1, 0, 0, 0],
    };
    let first = EventBatch {
        events: vec![event(1), event(2)],
    };
    assert_eq!(
        replica.apply_event_batch(&first),
        Ok(AuthoritativeFeedOutcome::EventsQueued(2))
    );
    assert_eq!(
        replica.apply_event_batch(&first),
        Ok(AuthoritativeFeedOutcome::Obsolete)
    );
    let third = EventBatch {
        events: vec![event(3)],
    };
    replica.apply_event_batch(&third).expect("third event");
    assert_eq!(replica.pending_event_count(), 2);
    assert_eq!(replica.pop_event().expect("second retained").event_id, 2);
    assert_eq!(replica.pop_event().expect("third retained").event_id, 3);
}

#[test]
fn unsupported_material_delta_cannot_advance_revision_or_create_a_chunk() {
    let mut replica = AuthoritativeFeedReplica::new(viewer());
    let material = TerrainDelta {
        chunk_x: 4,
        chunk_y: 8,
        base_revision: 0,
        new_revision: 1,
        operations: vec![TerrainOp {
            kind: TerrainOpKind::SetMaterial,
            cell_x: 0,
            cell_y: 0,
            height_delta_cm: 0,
            material: 7,
            flags: 0,
        }],
    };
    assert_eq!(
        replica.apply_terrain_delta(&material),
        Err(AuthoritativeFeedError::UnsupportedTerrainOperation(
            TerrainOpKind::SetMaterial
        ))
    );
    assert!(replica.resync_requested());
    assert!(replica.terrain().is_empty());
}
