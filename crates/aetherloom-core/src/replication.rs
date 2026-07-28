use alloc::vec::Vec;

use aetherloom_protocol::{
    EntityId, PlayerCommand, PlayerId, SnapshotId, TeamId, HEADER_BYTES,
    MAX_DATAGRAM_BYTES,
};

use crate::entity::EntityKind;
use crate::state::{
    MatchState, PLAYER_MAX_ALTITUDE_CM, PLAYER_MIN_ALTITUDE_CM,
    PLAYER_PLANAR_SPEED_CM_PER_TICK, PLAYER_VERTICAL_SPEED_CM_PER_TICK,
};
use crate::terrain::ChunkCoord;

pub const MIN_INTERPOLATION_DELAY_TICKS: u8 = 2;
pub const MAX_INTERPOLATION_DELAY_TICKS: u8 = 6;
pub const MAX_PENDING_PREDICTION_COMMANDS: usize = 256;
pub const MAX_INTERPOLATION_SAMPLES: usize = 8;
// Wire sizes from protocol v3. Keeping the budget here prevents replication
// history from acknowledging updates that a 1,200-byte datagram cannot carry.
const DELTA_FIXED_BYTES: usize = HEADER_BYTES + 18;
const DELTA_ENTITY_BYTES: usize = 40;
const REMOVED_ENTITY_BYTES: usize = 8;
const MAX_REMOVED_PER_DELTA: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum InterestTier {
    CombatCritical128Hz = 1,
    Medium32Hz = 2,
    Distant8Hz = 3,
}

impl InterestTier {
    pub const fn update_hz(self) -> u16 {
        match self {
            Self::CombatCritical128Hz => 128,
            Self::Medium32Hz => 32,
            Self::Distant8Hz => 8,
        }
    }

    fn due(self, tick: u64) -> bool {
        match self {
            Self::CombatCritical128Hz => true,
            Self::Medium32Hz => tick % 4 == 0,
            Self::Distant8Hz => tick % 16 == 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReplicatedEntity {
    pub entity_id: EntityId,
    pub kind: EntityKind,
    pub owner: Option<PlayerId>,
    pub team: Option<TeamId>,
    pub position_cm: [i32; 3],
    pub velocity_cm_per_tick: [i16; 3],
    pub yaw: u16,
    pub pitch: i16,
    pub health: u16,
    pub flags: u16,
    pub interest: InterestTier,
    pub scheduled_hz: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainRevision {
    pub chunk: ChunkCoord,
    pub revision: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotKind {
    Keyframe,
    Delta,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub snapshot_id: SnapshotId,
    pub baseline_id: SnapshotId,
    pub kind: SnapshotKind,
    pub viewer: PlayerId,
    pub server_tick: u64,
    pub acknowledged_input_sequence: u32,
    pub entities: Vec<ReplicatedEntity>,
    pub removed_entities: Vec<EntityId>,
    pub terrain_revisions: Vec<TerrainRevision>,
}

/// A bounded, presentation-only copy of a fully applied remote snapshot.
///
/// Keeping this history in the replica allows renderers to interpolate remote
/// entities without feeding presentation timing back into authoritative
/// prediction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaSample {
    pub server_tick: u64,
    pub entities: Vec<ReplicatedEntity>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredSnapshot {
    pub id: SnapshotId,
    pub viewer: PlayerId,
    /// State the client knows after applying this snapshot, not an
    /// unscheduled copy of server state.
    pub entities: Vec<ReplicatedEntity>,
    /// First sorted entity index considered by the next bounded delta. Keeping
    /// this in the viewer baseline makes budget fairness deterministic even
    /// when clients acknowledge at different rates.
    pub next_entity_cursor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicationError {
    UnknownViewer(PlayerId),
    EmptyViewer(PlayerId),
    BaselineMissing {
        requested: SnapshotId,
        resync_required: bool,
    },
}

impl MatchState {
    /// Produces a viewer-specific keyframe or delta.
    ///
    /// A non-zero baseline must still be present in the bounded history and
    /// must belong to the same viewer. Missing baselines explicitly request a
    /// reliable resynchronization instead of guessing.
    pub fn replicate(
        &mut self,
        viewer: PlayerId,
        baseline_id: SnapshotId,
    ) -> Result<Snapshot, ReplicationError> {
        let Some(viewer_state) = self.players.get(viewer.index()).copied() else {
            return Err(ReplicationError::UnknownViewer(viewer));
        };
        let Some(viewer_entity_id) = viewer_state.entity_id else {
            return Err(ReplicationError::EmptyViewer(viewer));
        };
        let Some(viewer_entity) = self.entities.get(viewer_entity_id).copied() else {
            return Err(ReplicationError::EmptyViewer(viewer));
        };

        let baseline = if baseline_id.is_none() {
            None
        } else {
            Some(
                self.snapshot_history
                    .iter()
                    .find(|snapshot| snapshot.id == baseline_id && snapshot.viewer == viewer)
                    .cloned()
                    .ok_or(ReplicationError::BaselineMissing {
                        requested: baseline_id,
                        resync_required: true,
                    })?,
            )
        };

        let mut current: Vec<ReplicatedEntity> = self
            .entities
            .iter()
            .map(|entity| {
                let dx =
                    i128::from(entity.position_cm[0]) - i128::from(viewer_entity.position_cm[0]);
                let dy =
                    i128::from(entity.position_cm[1]) - i128::from(viewer_entity.position_cm[1]);
                let dz =
                    i128::from(entity.position_cm[2]) - i128::from(viewer_entity.position_cm[2]);
                let distance_squared = dx * dx + dy * dy + dz * dz;
                let interest = if entity.id == viewer_entity_id
                    || distance_squared <= 2_000_i128.pow(2)
                {
                    InterestTier::CombatCritical128Hz
                } else if distance_squared <= 8_000_i128.pow(2) {
                    InterestTier::Medium32Hz
                } else {
                    InterestTier::Distant8Hz
                };
                ReplicatedEntity {
                    entity_id: entity.id,
                    kind: entity.kind,
                    owner: entity.owner,
                    team: entity.team,
                    position_cm: entity.position_cm,
                    velocity_cm_per_tick: entity.velocity_cm_per_tick,
                    yaw: entity.yaw,
                    pitch: entity.pitch,
                    health: entity.health,
                    flags: entity.flags,
                    interest,
                    scheduled_hz: interest.update_hz(),
                }
            })
            .collect();
        current.sort_unstable_by_key(|entity| entity.entity_id);

        let (kind, changed, removed, known_after, next_entity_cursor) =
            if let Some(baseline) = baseline {
            let mut changed = Vec::new();
            let mut removed = Vec::new();
            let mut known_after = baseline.entities.clone();

            for old in &baseline.entities {
                if current
                    .binary_search_by_key(&old.entity_id, |entity| entity.entity_id)
                    .is_err()
                    && removed.len() < MAX_REMOVED_PER_DELTA
                {
                    removed.push(old.entity_id);
                }
            }
            known_after.retain(|entity| !removed.contains(&entity.entity_id));
            let entity_budget = MAX_DATAGRAM_BYTES
                .saturating_sub(
                    DELTA_FIXED_BYTES + removed.len() * REMOVED_ENTITY_BYTES,
                )
                / DELTA_ENTITY_BYTES;

            // Local reconciliation is never displaced by a crowded interest
            // set. Remaining bandwidth rotates through stable entity order so
            // continuously moving low IDs cannot starve higher IDs forever.
            if entity_budget > 0 {
                if let Some(local) = current
                    .iter()
                    .find(|entity| entity.entity_id == viewer_entity_id)
                    .copied()
                {
                    let old = baseline
                        .entities
                        .binary_search_by_key(&local.entity_id, |entry| entry.entity_id)
                        .ok()
                        .map(|index| baseline.entities[index]);
                    if old != Some(local) && local.interest.due(self.tick) {
                        changed.push(local);
                        update_known_entity(&mut known_after, local);
                    }
                }
            }

            let mut next_cursor = baseline.next_entity_cursor;
            if !current.is_empty() && changed.len() < entity_budget {
                let start = baseline.next_entity_cursor % current.len();
                for offset in 0..current.len() {
                    let index = (start + offset) % current.len();
                    let entity = current[index];
                    next_cursor = (index + 1) % current.len();
                    if entity.entity_id == viewer_entity_id {
                        continue;
                    }
                    let old = baseline
                        .entities
                        .binary_search_by_key(&entity.entity_id, |entry| entry.entity_id)
                        .ok()
                        .map(|old_index| baseline.entities[old_index]);
                    if old != Some(entity) && entity.interest.due(self.tick) {
                        changed.push(entity);
                        update_known_entity(&mut known_after, entity);
                        if changed.len() == entity_budget {
                            break;
                        }
                    }
                }
            }
            (
                SnapshotKind::Delta,
                changed,
                removed,
                known_after,
                next_cursor,
            )
        } else {
            (
                SnapshotKind::Keyframe,
                current.clone(),
                Vec::new(),
                current,
                0,
            )
        };

        let snapshot_id = self.snapshot_id();
        let acknowledged_input_sequence = viewer_state.last_sequence.unwrap_or(0);
        let terrain_revisions = self
            .terrain
            .iter()
            .map(|chunk| TerrainRevision {
                chunk: chunk.coord,
                revision: chunk.revision,
            })
            .collect();
        let snapshot = Snapshot {
            snapshot_id,
            baseline_id,
            kind,
            viewer,
            server_tick: self.tick,
            acknowledged_input_sequence,
            entities: changed,
            removed_entities: removed,
            terrain_revisions,
        };
        let viewer_history = self
            .snapshot_history
            .iter()
            .filter(|stored| stored.viewer == viewer)
            .count();
        if viewer_history == self.config.snapshot_history_capacity as usize {
            if let Some(oldest) = self
                .snapshot_history
                .iter()
                .position(|stored| stored.viewer == viewer)
            {
                self.snapshot_history.remove(oldest);
            }
        }
        self.snapshot_history.push(StoredSnapshot {
            id: snapshot_id,
            viewer,
            entities: known_after,
            next_entity_cursor,
        });
        Ok(snapshot)
    }
}

fn update_known_entity(known: &mut Vec<ReplicatedEntity>, entity: ReplicatedEntity) {
    match known.binary_search_by_key(&entity.entity_id, |entry| entry.entity_id) {
        Ok(index) => known[index] = entity,
        Err(index) => known.insert(index, entity),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplicaError {
    WrongViewer {
        expected: PlayerId,
        received: PlayerId,
    },
    BaselineMismatch {
        current: SnapshotId,
        received: SnapshotId,
    },
    MissingBaseline {
        snapshot: SnapshotId,
    },
    ServerTickRegression {
        current: u64,
        received: u64,
    },
}

/// Client-side replicated state with prediction/reconciliation hooks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientReplica {
    viewer: PlayerId,
    snapshot_id: SnapshotId,
    server_tick: u64,
    entities: Vec<ReplicatedEntity>,
    terrain_revisions: Vec<TerrainRevision>,
    resync_requested: bool,
    interpolation_delay_ticks: u8,
    interpolation_samples: Vec<ReplicaSample>,
    predicted_position_cm: Option<[i32; 3]>,
    pending_commands: Vec<PlayerCommand>,
    /// Newest locally predicted command discarded because the bounded
    /// history filled. Reconciliation is incomplete until the server
    /// acknowledges this command (or a newer one).
    dropped_prediction_through: Option<u32>,
}

impl ClientReplica {
    pub fn new(viewer: PlayerId) -> Self {
        Self {
            viewer,
            snapshot_id: SnapshotId::NONE,
            server_tick: 0,
            entities: Vec::new(),
            terrain_revisions: Vec::new(),
            resync_requested: false,
            interpolation_delay_ticks: MIN_INTERPOLATION_DELAY_TICKS,
            interpolation_samples: Vec::new(),
            predicted_position_cm: None,
            pending_commands: Vec::new(),
            dropped_prediction_through: None,
        }
    }

    pub const fn viewer(&self) -> PlayerId {
        self.viewer
    }

    pub const fn snapshot_id(&self) -> SnapshotId {
        self.snapshot_id
    }

    pub const fn server_tick(&self) -> u64 {
        self.server_tick
    }

    pub fn entities(&self) -> &[ReplicatedEntity] {
        &self.entities
    }

    pub fn terrain_revisions(&self) -> &[TerrainRevision] {
        &self.terrain_revisions
    }

    pub fn interpolation_samples(&self) -> &[ReplicaSample] {
        &self.interpolation_samples
    }

    pub const fn resync_requested(&self) -> bool {
        self.resync_requested
    }

    pub const fn interpolation_delay_ticks(&self) -> u8 {
        self.interpolation_delay_ticks
    }

    /// Clamps adaptive interpolation to the required 2..=6 tick window.
    pub fn set_interpolation_delay_ticks(&mut self, ticks: u8) {
        self.interpolation_delay_ticks =
            ticks.clamp(MIN_INTERPOLATION_DELAY_TICKS, MAX_INTERPOLATION_DELAY_TICKS);
    }

    pub const fn predicted_position_cm(&self) -> Option<[i32; 3]> {
        self.predicted_position_cm
    }

    pub fn predict_local(&mut self, command: PlayerCommand) {
        if self.pending_commands.len() == MAX_PENDING_PREDICTION_COMMANDS {
            let dropped = self.pending_commands.remove(0).sequence();
            if self
                .dropped_prediction_through
                .map_or(true, |previous| sequence_is_newer(dropped, previous))
            {
                self.dropped_prediction_through = Some(dropped);
            }
            self.resync_requested = true;
        }
        self.pending_commands.push(command);
        if let Some(position) = &mut self.predicted_position_cm {
            apply_predicted_movement(position, &command);
        }
    }

    /// Returns `false` for an obsolete snapshot that was safely discarded.
    pub fn apply(&mut self, snapshot: Snapshot) -> Result<bool, ReplicaError> {
        if snapshot.viewer != self.viewer {
            return Err(ReplicaError::WrongViewer {
                expected: self.viewer,
                received: snapshot.viewer,
            });
        }
        if !self.snapshot_id.is_none()
            && !sequence_is_newer(
                snapshot.snapshot_id.get(),
                self.snapshot_id.get(),
            )
        {
            return Ok(false);
        }
        if !self.snapshot_id.is_none() && snapshot.server_tick < self.server_tick {
            self.resync_requested = true;
            return Err(ReplicaError::ServerTickRegression {
                current: self.server_tick,
                received: snapshot.server_tick,
            });
        }
        if snapshot.kind == SnapshotKind::Delta && snapshot.baseline_id.is_none() {
            self.resync_requested = true;
            return Err(ReplicaError::MissingBaseline {
                snapshot: snapshot.snapshot_id,
            });
        }
        if snapshot.kind == SnapshotKind::Delta && snapshot.baseline_id != self.snapshot_id {
            self.resync_requested = true;
            return Err(ReplicaError::BaselineMismatch {
                current: self.snapshot_id,
                received: snapshot.baseline_id,
            });
        }

        if snapshot.kind == SnapshotKind::Keyframe {
            self.entities = snapshot.entities.clone();
        } else {
            for removed in &snapshot.removed_entities {
                if let Ok(index) = self
                    .entities
                    .binary_search_by_key(removed, |entity| entity.entity_id)
                {
                    self.entities.remove(index);
                }
            }
            for changed in &snapshot.entities {
                match self
                    .entities
                    .binary_search_by_key(&changed.entity_id, |entity| entity.entity_id)
                {
                    Ok(index) => self.entities[index] = *changed,
                    Err(index) => self.entities.insert(index, *changed),
                }
            }
        }
        self.entities
            .sort_unstable_by_key(|entity| entity.entity_id);
        self.terrain_revisions = snapshot.terrain_revisions;
        self.snapshot_id = snapshot.snapshot_id;
        self.server_tick = snapshot.server_tick;
        let sample = ReplicaSample {
            server_tick: self.server_tick,
            entities: self.entities.clone(),
        };
        if self
            .interpolation_samples
            .last()
            .is_some_and(|previous| previous.server_tick == sample.server_tick)
        {
            let last = self
                .interpolation_samples
                .last_mut()
                .expect("last sample was just observed");
            *last = sample;
        } else {
            if self.interpolation_samples.len() == MAX_INTERPOLATION_SAMPLES {
                self.interpolation_samples.remove(0);
            }
            self.interpolation_samples.push(sample);
        }

        let acknowledged = snapshot.acknowledged_input_sequence;
        if self.dropped_prediction_through.is_some_and(|dropped| {
            acknowledged != 0
                && (acknowledged == dropped
                    || sequence_is_newer(acknowledged, dropped))
        }) {
            self.dropped_prediction_through = None;
        }
        self.resync_requested = self.dropped_prediction_through.is_some();

        // Sequence zero means that the server has not processed any input.
        // It is a sentinel, not a serial number preceding only small values;
        // pre-wrap commands such as u32::MAX must therefore remain pending.
        if acknowledged != 0 {
            self.pending_commands
                .retain(|command| sequence_is_newer(command.sequence(), acknowledged));
        }
        self.predicted_position_cm = self
            .entities
            .iter()
            .find(|entity| entity.owner == Some(self.viewer))
            .map(|entity| entity.position_cm);
        if let Some(position) = &mut self.predicted_position_cm {
            for command in &self.pending_commands {
                apply_predicted_movement(position, command);
            }
        }
        Ok(true)
    }

    pub fn pending_prediction_count(&self) -> usize {
        self.pending_commands.len()
    }

    /// Interpolates a remote entity at the current adaptive presentation
    /// target. `subtick_numerator` is a 16-bit fraction of one authoritative
    /// tick. The local predicted player should use `predicted_position_cm`
    /// instead.
    pub fn interpolated_position_cm(
        &self,
        entity_id: EntityId,
        subtick_numerator: u16,
    ) -> Option<[i32; 3]> {
        let target_tick = self
            .server_tick
            .saturating_sub(self.interpolation_delay_ticks as u64);
        let target = (target_tick as u128)
            .saturating_mul(1_u128 << 16)
            .saturating_add(subtick_numerator as u128);
        let older = self
            .interpolation_samples
            .iter()
            .rev()
            .find(|sample| ((sample.server_tick as u128) << 16) <= target)
            .or_else(|| self.interpolation_samples.first())?;
        let newer = self
            .interpolation_samples
            .iter()
            .find(|sample| ((sample.server_tick as u128) << 16) >= target)
            .or_else(|| self.interpolation_samples.last())?;
        let older_position = sample_position(older, entity_id);
        let newer_position = sample_position(newer, entity_id);
        match (older_position, newer_position) {
            (Some(position), None) | (None, Some(position)) => Some(position),
            (None, None) => None,
            (Some(from), Some(to)) => {
                let from_time = (older.server_tick as u128) << 16;
                let to_time = (newer.server_tick as u128) << 16;
                if to_time <= from_time || target <= from_time {
                    return Some(from);
                }
                if target >= to_time {
                    return Some(to);
                }
                let numerator = target - from_time;
                let denominator = to_time - from_time;
                Some([
                    interpolate_axis(from[0], to[0], numerator, denominator),
                    interpolate_axis(from[1], to[1], numerator, denominator),
                    interpolate_axis(from[2], to[2], numerator, denominator),
                ])
            }
        }
    }
}

fn sample_position(sample: &ReplicaSample, entity_id: EntityId) -> Option<[i32; 3]> {
    sample
        .entities
        .binary_search_by_key(&entity_id, |entity| entity.entity_id)
        .ok()
        .map(|index| sample.entities[index].position_cm)
}

fn interpolate_axis(from: i32, to: i32, numerator: u128, denominator: u128) -> i32 {
    let delta = i64::from(to) - i64::from(from);
    let scaled = if delta >= 0 {
        (delta as u128)
            .saturating_mul(numerator)
            .checked_div(denominator)
            .unwrap_or(0) as i64
    } else {
        -((delta.unsigned_abs() as u128)
            .saturating_mul(numerator)
            .checked_div(denominator)
            .unwrap_or(0) as i64)
    };
    i64::from(from)
        .saturating_add(scaled)
        .clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn apply_predicted_movement(position: &mut [i32; 3], command: &PlayerCommand) {
    position[0] = position[0].saturating_add(
        command.move_x() as i32 * PLAYER_PLANAR_SPEED_CM_PER_TICK
            / i32::from(aetherloom_protocol::MAX_MOVE_AXIS),
    );
    position[1] = position[1]
        .saturating_add(
            command.move_vertical() as i32 * PLAYER_VERTICAL_SPEED_CM_PER_TICK
                / i32::from(aetherloom_protocol::MAX_MOVE_AXIS),
        )
        .clamp(PLAYER_MIN_ALTITUDE_CM, PLAYER_MAX_ALTITUDE_CM);
    position[2] = position[2].saturating_add(
        command.move_y() as i32 * PLAYER_PLANAR_SPEED_CM_PER_TICK
            / i32::from(aetherloom_protocol::MAX_MOVE_AXIS),
    );
}

fn sequence_is_newer(received: u32, previous: u32) -> bool {
    let distance = received.wrapping_sub(previous);
    distance != 0 && distance < (1_u32 << 31)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Controller, MatchConfig, TeamId, WorldSeed};

    #[test]
    fn extreme_checkpoint_positions_cannot_overflow_interest_distance() {
        let mut state = MatchState::new(
            MatchConfig::new(2, 8, 1).expect("config"),
            WorldSeed::new(1),
        );
        let first = PlayerId::new(0).expect("player");
        let second = PlayerId::new(1).expect("player");
        let first_entity = state
            .add_player(first, TeamId::new(0).expect("team"), Controller::Human)
            .expect("first entity");
        let second_entity = state
            .add_player(second, TeamId::new(1).expect("team"), Controller::Human)
            .expect("second entity");
        state.players[first.index()].position_cm = [i32::MIN, 0, i32::MIN];
        state.players[second.index()].position_cm = [i32::MAX, 0, i32::MAX];
        state
            .entities
            .get_mut(first_entity)
            .expect("first")
            .position_cm = [i32::MIN, 0, i32::MIN];
        state
            .entities
            .get_mut(second_entity)
            .expect("second")
            .position_cm = [i32::MAX, 0, i32::MAX];

        let snapshot = state.replicate(first, SnapshotId::NONE).expect("snapshot");
        let remote = snapshot
            .entities
            .iter()
            .find(|entity| entity.entity_id == second_entity)
            .expect("remote entity");
        assert_eq!(remote.interest, InterestTier::Distant8Hz);
    }
}
