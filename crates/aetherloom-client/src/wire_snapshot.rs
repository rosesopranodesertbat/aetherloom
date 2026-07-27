use alloc::vec::Vec;

use aetherloom_core::{
    ChunkCoord, ClientReplica, EntityKind, InterestTier, ReplicaError,
    ReplicatedEntity, Snapshot, SnapshotKind, TerrainRevision,
};
use aetherloom_protocol::{
    EntityState, Message, MessageEnvelope, SnapshotDelta, SnapshotKeyframe,
    ValidationError,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireSnapshotError {
    NotEntitySnapshot,
    Validation(ValidationError),
    InvalidArchetype(u16),
    Replica(ReplicaError),
}

/// Converts a decoded wire keyframe or delta into the platform-neutral core
/// replica format. Envelope time is authoritative; client arrival time never
/// enters the gameplay snapshot.
fn decode_wire_snapshot(
    envelope: &MessageEnvelope,
) -> Result<Snapshot, WireSnapshotError> {
    match &envelope.message {
        Message::SnapshotKeyframe(keyframe) => {
            keyframe
                .validate()
                .map_err(WireSnapshotError::Validation)?;
            decode_keyframe(envelope.metadata.server_tick, keyframe)
        }
        Message::SnapshotDelta(delta) => {
            delta
                .validate()
                .map_err(WireSnapshotError::Validation)?;
            decode_delta(envelope.metadata.server_tick, delta)
        }
        _ => Err(WireSnapshotError::NotEntitySnapshot),
    }
}

pub fn apply_wire_snapshot(
    replica: &mut ClientReplica,
    envelope: &MessageEnvelope,
) -> Result<bool, WireSnapshotError> {
    let mut snapshot = decode_wire_snapshot(envelope)?;
    if snapshot.kind == SnapshotKind::Delta {
        // Terrain changes arrive on their own reliable revision stream.
        // Entity-only deltas must not erase the last keyframe's revision view
        // inside the core replica.
        snapshot.terrain_revisions = replica.terrain_revisions().to_vec();
    }
    replica.apply(snapshot).map_err(WireSnapshotError::Replica)
}

fn decode_keyframe(
    server_tick: u64,
    keyframe: &SnapshotKeyframe,
) -> Result<Snapshot, WireSnapshotError> {
    Ok(Snapshot {
        snapshot_id: keyframe.snapshot_id,
        baseline_id: aetherloom_protocol::SnapshotId::NONE,
        kind: SnapshotKind::Keyframe,
        viewer: keyframe.viewer,
        server_tick,
        acknowledged_input_sequence: keyframe.acknowledged_input_sequence,
        entities: decode_entities(&keyframe.entities)?,
        removed_entities: Vec::new(),
        terrain_revisions: keyframe
            .terrain_revisions
            .iter()
            .map(|revision| TerrainRevision {
                chunk: ChunkCoord {
                    x: revision.x,
                    y: revision.y,
                },
                revision: revision.revision,
            })
            .collect(),
    })
}

fn decode_delta(
    server_tick: u64,
    delta: &SnapshotDelta,
) -> Result<Snapshot, WireSnapshotError> {
    Ok(Snapshot {
        snapshot_id: delta.snapshot_id,
        baseline_id: delta.baseline_id,
        kind: SnapshotKind::Delta,
        viewer: delta.viewer,
        server_tick,
        acknowledged_input_sequence: delta.acknowledged_input_sequence,
        entities: decode_entities(&delta.entities)?,
        removed_entities: delta.removed_entities.clone(),
        terrain_revisions: Vec::new(),
    })
}

fn decode_entities(
    entities: &[EntityState],
) -> Result<Vec<ReplicatedEntity>, WireSnapshotError> {
    entities.iter().map(decode_entity).collect()
}

fn decode_entity(entity: &EntityState) -> Result<ReplicatedEntity, WireSnapshotError> {
    let kind = match entity.archetype {
        1 => EntityKind::Player,
        2 => EntityKind::Projectile,
        3 => EntityKind::Resource,
        4 => EntityKind::Objective,
        other => return Err(WireSnapshotError::InvalidArchetype(other)),
    };
    Ok(ReplicatedEntity {
        entity_id: entity.entity_id,
        kind,
        owner: entity.owner,
        team: entity.team,
        position_cm: entity.position_cm,
        velocity_cm_per_tick: entity.velocity_cm_per_tick,
        yaw: entity.yaw,
        pitch: entity.pitch,
        health: entity.health,
        flags: entity.flags,
        // Interest cadence is a server scheduling concern and is not part of
        // authoritative entity state on the wire. Treat received data as
        // immediately present; interpolation uses actual snapshot tick gaps.
        interest: InterestTier::CombatCritical128Hz,
        scheduled_hz: 128,
    })
}
