use alloc::collections::VecDeque;
use alloc::vec;
use alloc::vec::Vec;

use aetherloom_protocol::{
    EventBatch, GameEvent, Message, PlayerId, SnapshotKeyframe, TerrainDelta,
    TerrainOpKind, ValidationError, TERRAIN_CELLS_PER_CHUNK,
    TERRAIN_CHUNK_SIDE,
};

pub const DEFAULT_TERRAIN_CHUNK_CAPACITY: usize = 4_096;
pub const DEFAULT_EVENT_HISTORY_CAPACITY: usize = 1_024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaTerrainChunk {
    pub x: i32,
    pub y: i32,
    pub revision: u32,
    pub heights_cm: Vec<i16>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthoritativeFeedOutcome {
    Ignored,
    KeyframeApplied,
    TerrainApplied,
    EventsQueued(usize),
    Obsolete,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthoritativeFeedError {
    ZeroCapacity,
    Validation(ValidationError),
    WrongViewer {
        expected: PlayerId,
        received: PlayerId,
    },
    TerrainCapacity {
        maximum: usize,
    },
    TerrainRevisionGap {
        x: i32,
        y: i32,
        current: u32,
        base: u32,
        new: u32,
    },
    UnsupportedTerrainOperation(TerrainOpKind),
}

/// Bounded client cache for authoritative non-entity messages.
///
/// Entity snapshots continue through `ClientReplica`. This companion consumes
/// the full terrain state in reliable keyframes, revision-checked terrain
/// deltas, and de-duplicated gameplay events without allowing a hostile or
/// stalled stream to grow client memory without bound.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthoritativeFeedReplica {
    viewer: PlayerId,
    terrain_capacity: usize,
    event_capacity: usize,
    terrain: Vec<ReplicaTerrainChunk>,
    recent_event_ids: VecDeque<u64>,
    pending_events: VecDeque<GameEvent>,
    resync_requested: bool,
}

impl AuthoritativeFeedReplica {
    pub fn new(viewer: PlayerId) -> Self {
        Self {
            viewer,
            terrain_capacity: DEFAULT_TERRAIN_CHUNK_CAPACITY,
            event_capacity: DEFAULT_EVENT_HISTORY_CAPACITY,
            terrain: Vec::new(),
            recent_event_ids: VecDeque::new(),
            pending_events: VecDeque::new(),
            resync_requested: false,
        }
    }

    pub fn with_capacities(
        viewer: PlayerId,
        terrain_capacity: usize,
        event_capacity: usize,
    ) -> Result<Self, AuthoritativeFeedError> {
        if terrain_capacity == 0 || event_capacity == 0 {
            return Err(AuthoritativeFeedError::ZeroCapacity);
        }
        Ok(Self {
            viewer,
            terrain_capacity,
            event_capacity,
            terrain: Vec::new(),
            recent_event_ids: VecDeque::new(),
            pending_events: VecDeque::new(),
            resync_requested: false,
        })
    }

    pub const fn viewer(&self) -> PlayerId {
        self.viewer
    }

    pub fn terrain(&self) -> &[ReplicaTerrainChunk] {
        &self.terrain
    }

    pub fn terrain_chunk(&self, x: i32, y: i32) -> Option<&ReplicaTerrainChunk> {
        self.terrain
            .binary_search_by_key(&(x, y), |chunk| (chunk.x, chunk.y))
            .ok()
            .map(|index| &self.terrain[index])
    }

    pub const fn resync_requested(&self) -> bool {
        self.resync_requested
    }

    pub fn pending_event_count(&self) -> usize {
        self.pending_events.len()
    }

    pub fn pop_event(&mut self) -> Option<GameEvent> {
        self.pending_events.pop_front()
    }

    pub fn apply(
        &mut self,
        message: &Message,
    ) -> Result<AuthoritativeFeedOutcome, AuthoritativeFeedError> {
        match message {
            Message::SnapshotKeyframe(keyframe) => self.apply_keyframe(keyframe),
            Message::TerrainDelta(delta) => self.apply_terrain_delta(delta),
            Message::EventBatch(batch) => self.apply_event_batch(batch),
            Message::InputBatch(_)
            | Message::SnapshotDelta(_)
            | Message::MatchResult(_) => Ok(AuthoritativeFeedOutcome::Ignored),
        }
    }

    pub fn apply_keyframe(
        &mut self,
        keyframe: &SnapshotKeyframe,
    ) -> Result<AuthoritativeFeedOutcome, AuthoritativeFeedError> {
        keyframe
            .validate()
            .map_err(AuthoritativeFeedError::Validation)?;
        if keyframe.viewer != self.viewer {
            return Err(AuthoritativeFeedError::WrongViewer {
                expected: self.viewer,
                received: keyframe.viewer,
            });
        }
        if keyframe.terrain_chunks.len() > self.terrain_capacity {
            self.resync_requested = true;
            return Err(AuthoritativeFeedError::TerrainCapacity {
                maximum: self.terrain_capacity,
            });
        }

        let mut replacement: Vec<ReplicaTerrainChunk> = keyframe
            .terrain_chunks
            .iter()
            .map(|chunk| ReplicaTerrainChunk {
                x: chunk.x,
                y: chunk.y,
                revision: chunk.revision,
                heights_cm: chunk.heights_cm.clone(),
            })
            .collect();
        replacement.sort_unstable_by_key(|chunk| (chunk.x, chunk.y));
        self.terrain = replacement;
        self.resync_requested = false;
        Ok(AuthoritativeFeedOutcome::KeyframeApplied)
    }

    pub fn apply_terrain_delta(
        &mut self,
        delta: &TerrainDelta,
    ) -> Result<AuthoritativeFeedOutcome, AuthoritativeFeedError> {
        delta
            .validate()
            .map_err(AuthoritativeFeedError::Validation)?;
        if let Some(operation) = delta
            .operations
            .iter()
            .find(|operation| operation.kind == TerrainOpKind::SetMaterial)
        {
            self.resync_requested = true;
            return Err(AuthoritativeFeedError::UnsupportedTerrainOperation(
                operation.kind,
            ));
        }
        let key = (delta.chunk_x, delta.chunk_y);
        let index = match self
            .terrain
            .binary_search_by_key(&key, |chunk| (chunk.x, chunk.y))
        {
            Ok(index) => {
                let current = self.terrain[index].revision;
                if delta.new_revision == current
                    || !serial_is_newer(delta.new_revision, current)
                {
                    return Ok(AuthoritativeFeedOutcome::Obsolete);
                }
                if delta.base_revision != current {
                    self.resync_requested = true;
                    return Err(AuthoritativeFeedError::TerrainRevisionGap {
                        x: delta.chunk_x,
                        y: delta.chunk_y,
                        current,
                        base: delta.base_revision,
                        new: delta.new_revision,
                    });
                }
                index
            }
            Err(index) => {
                if delta.base_revision != 0 {
                    self.resync_requested = true;
                    return Err(AuthoritativeFeedError::TerrainRevisionGap {
                        x: delta.chunk_x,
                        y: delta.chunk_y,
                        current: 0,
                        base: delta.base_revision,
                        new: delta.new_revision,
                    });
                }
                if self.terrain.len() == self.terrain_capacity {
                    self.resync_requested = true;
                    return Err(AuthoritativeFeedError::TerrainCapacity {
                        maximum: self.terrain_capacity,
                    });
                }
                self.terrain.insert(
                    index,
                    ReplicaTerrainChunk {
                        x: delta.chunk_x,
                        y: delta.chunk_y,
                        revision: 0,
                        heights_cm: vec![0; TERRAIN_CELLS_PER_CHUNK],
                    },
                );
                index
            }
        };

        let chunk = &mut self.terrain[index];
        for operation in &delta.operations {
            let cell = operation.cell_y as usize
                * TERRAIN_CHUNK_SIDE as usize
                + operation.cell_x as usize;
            match operation.kind {
                TerrainOpKind::HeightDelta | TerrainOpKind::Crater => {
                    chunk.heights_cm[cell] = chunk.heights_cm[cell]
                        .saturating_add(operation.height_delta_cm);
                }
                TerrainOpKind::SetMaterial => {
                    unreachable!("unsupported operations are rejected before mutation")
                }
            }
        }
        chunk.revision = delta.new_revision;
        Ok(AuthoritativeFeedOutcome::TerrainApplied)
    }

    pub fn apply_event_batch(
        &mut self,
        batch: &EventBatch,
    ) -> Result<AuthoritativeFeedOutcome, AuthoritativeFeedError> {
        batch
            .validate()
            .map_err(AuthoritativeFeedError::Validation)?;
        let mut queued = 0;
        for event in &batch.events {
            if self.recent_event_ids.contains(&event.event_id) {
                continue;
            }
            if self.recent_event_ids.len() == self.event_capacity {
                self.recent_event_ids.pop_front();
            }
            self.recent_event_ids.push_back(event.event_id);
            if self.pending_events.len() == self.event_capacity {
                self.pending_events.pop_front();
            }
            self.pending_events.push_back(event.clone());
            queued += 1;
        }
        if queued == 0 {
            Ok(AuthoritativeFeedOutcome::Obsolete)
        } else {
            Ok(AuthoritativeFeedOutcome::EventsQueued(queued))
        }
    }
}

fn serial_is_newer(received: u32, previous: u32) -> bool {
    let distance = received.wrapping_sub(previous);
    distance != 0 && distance < (1_u32 << 31)
}
