use alloc::vec::Vec;

use aetherloom_protocol::{EntityId, PlayerId, TeamId};

use crate::{Controller, EntityKind, MatchState, PlayerOutcome};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationPlayer {
    pub id: PlayerId,
    pub team: Option<TeamId>,
    pub controller: Controller,
    pub entity_id: Option<EntityId>,
    pub position_cm: [i32; 3],
    pub yaw: u16,
    pub health: u16,
    pub outcome: PlayerOutcome,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationEntity {
    pub id: EntityId,
    pub kind: EntityKind,
    pub position_cm: [i32; 3],
    pub yaw: u16,
    pub health: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PresentationFrame {
    pub tick: u64,
    pub viewer: PlayerId,
    pub alpha: f32,
    pub cosmetic_seed: u64,
    pub players: Vec<PresentationPlayer>,
    pub entities: Vec<PresentationEntity>,
}

/// Presentation extraction owns cosmetic entropy and never mutates MatchState.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentationWorld {
    cosmetic_seed: u64,
}

impl PresentationWorld {
    pub const fn new(cosmetic_seed: u64) -> Self {
        Self { cosmetic_seed }
    }

    pub const fn cosmetic_seed(&self) -> u64 {
        self.cosmetic_seed
    }

    pub fn set_cosmetic_seed(&mut self, seed: u64) {
        self.cosmetic_seed = seed;
    }

    pub fn extract(
        &self,
        state: &MatchState,
        viewer: PlayerId,
        alpha: f32,
    ) -> PresentationFrame {
        let alpha = alpha.clamp(0.0, 1.0);
        let players = state
            .players()
            .iter()
            .filter(|player| player.controller != Controller::Empty)
            .map(|player| PresentationPlayer {
                id: player.id,
                team: player.team,
                controller: player.controller,
                entity_id: player.entity_id,
                position_cm: player.position_cm,
                yaw: player.yaw,
                health: player.health,
                outcome: player.outcome,
            })
            .collect();
        let entities = state
            .entities()
            .iter()
            .map(|entity| PresentationEntity {
                id: entity.id,
                kind: entity.kind,
                position_cm: entity.position_cm,
                yaw: entity.yaw,
                health: entity.health,
            })
            .collect();
        PresentationFrame {
            tick: state.tick(),
            viewer,
            alpha,
            cosmetic_seed: self.cosmetic_seed,
            players,
            entities,
        }
    }
}

/// Graphics APIs (wgpu, a console SDK, or a test renderer) implement this
/// boundary without becoming a gameplay dependency.
pub trait RendererBackend {
    type Error;

    fn render(&mut self, frame: &PresentationFrame) -> Result<(), Self::Error>;

    fn on_device_lost(&mut self) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlatformError {
    Unavailable,
    PermissionDenied,
    StorageFull,
    CorruptData,
    IdentityChanged,
}

/// Console-, desktop-, and browser-specific services sit behind this stable
/// interface. Online credentials and saves never enter authoritative hashes.
pub trait PlatformServices {
    fn monotonic_time_nanos(&self) -> u64;

    fn account_id(&self) -> Result<[u8; 16], PlatformError>;

    fn load_blob(&self, key: &str) -> Result<Vec<u8>, PlatformError>;

    fn store_blob(&mut self, key: &str, bytes: &[u8]) -> Result<(), PlatformError>;

    fn is_suspended(&self) -> bool;
}

