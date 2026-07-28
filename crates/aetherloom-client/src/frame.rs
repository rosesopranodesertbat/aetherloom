use alloc::vec::Vec;

use aetherloom_core::{ChunkCoord, ClientReplica};
use aetherloom_protocol::{EntityId, PlayerId};

use crate::{RendererCapabilities, SelectedCapabilities};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceId(pub u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Viewport {
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SafeAreaInsets {
    pub left_px: u32,
    pub top_px: u32,
    pub right_px: u32,
    pub bottom_px: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SafeAreaRect {
    pub x_px: u32,
    pub y_px: u32,
    pub width_px: u32,
    pub height_px: u32,
}

impl SafeAreaRect {
    pub fn layout(viewport: Viewport, insets: SafeAreaInsets) -> Self {
        let left = insets.left_px.min(viewport.width_px);
        let right = insets
            .right_px
            .min(viewport.width_px.saturating_sub(left));
        let top = insets.top_px.min(viewport.height_px);
        let bottom = insets
            .bottom_px
            .min(viewport.height_px.saturating_sub(top));
        Self {
            x_px: left,
            y_px: top,
            width_px: viewport.width_px.saturating_sub(left + right),
            height_px: viewport.height_px.saturating_sub(top + bottom),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerrainDraw {
    pub chunk: ChunkCoord,
    pub revision: u32,
    pub mesh: ResourceId,
    pub material: ResourceId,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TerrainFrame {
    pub draws: Vec<TerrainDraw>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshDraw {
    pub entity: Option<EntityId>,
    pub mesh: ResourceId,
    pub material: ResourceId,
    pub transform: [f32; 16],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshFrame {
    pub draws: Vec<MeshDraw>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceData {
    pub entity: Option<EntityId>,
    pub transform: [f32; 16],
    pub color_rgba: [f32; 4],
    pub custom: [f32; 4],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstanceFrame {
    pub mesh: ResourceId,
    pub material: ResourceId,
    pub instances: Vec<InstanceData>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleData {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub color_rgba: [f32; 4],
    pub size: f32,
    pub rotation: f32,
    pub age_seconds: f32,
    pub lifetime_seconds: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParticleFrame {
    pub atlas: ResourceId,
    pub particles: Vec<ParticleData>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraFrame {
    pub view: [f32; 16],
    pub projection: [f32; 16],
    pub position: [f32; 3],
    pub exposure: f32,
    pub viewport: Viewport,
    pub safe_area: SafeAreaRect,
}

impl Default for CameraFrame {
    fn default() -> Self {
        Self {
            view: identity_matrix(),
            projection: identity_matrix(),
            position: [0.0; 3],
            exposure: 1.0,
            viewport: Viewport::default(),
            safe_area: SafeAreaRect::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HudElementKind {
    Health = 1,
    Cooldown = 2,
    Inventory = 3,
    Objective = 4,
    Reticle = 5,
    NavigationFocus = 6,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudElement {
    pub kind: HudElementKind,
    pub owner: Option<PlayerId>,
    pub rect_normalized: [f32; 4],
    pub value: f32,
    pub style: u16,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudFrame {
    pub safe_area: SafeAreaRect,
    pub focused_element: Option<u32>,
    pub elements: Vec<HudElement>,
}

/// A complete presentation-only frame. None of these fields may be fed back
/// into authoritative simulation or authoritative hashes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClientRenderFrame {
    pub authoritative_tick: u64,
    pub interpolation_alpha: f32,
    pub terrain: TerrainFrame,
    pub meshes: MeshFrame,
    pub instance_batches: Vec<InstanceFrame>,
    pub particles: ParticleFrame,
    pub camera: CameraFrame,
    pub hud: HudFrame,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderError<E> {
    Backend(E),
    DeviceLost,
    InvalidFrame,
}

/// Implemented by wgpu on web/desktop or by a proprietary console adapter.
/// The simulation and frame extraction crates never import a graphics API.
pub trait RendererBackend {
    type Error;

    fn capabilities(&self) -> RendererCapabilities;

    fn configure(&mut self, selected: SelectedCapabilities) -> Result<(), Self::Error>;

    fn render(&mut self, frame: &ClientRenderFrame) -> Result<(), RenderError<Self::Error>>;

    fn recover_device(&mut self) -> Result<(), Self::Error>;
}

/// Converts replicated gameplay state into presentation-only data. A wgpu or
/// console backend consumes the result but cannot reach back into prediction.
pub trait PresentationExtractor {
    type Error;

    fn extract(
        &mut self,
        replica: &ClientReplica,
        interpolation_alpha: f32,
        viewport: Viewport,
        safe_area: SafeAreaRect,
    ) -> Result<ClientRenderFrame, Self::Error>;
}

const fn identity_matrix() -> [f32; 16] {
    [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ]
}
