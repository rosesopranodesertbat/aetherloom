#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

#[cfg(any(test, feature = "std"))]
extern crate std;

mod capabilities;
mod authoritative_feed;
mod frame;
mod input;
mod orchestrator;
mod platform;
mod session;
mod transport;
mod wire_snapshot;
pub mod vendor;

pub use capabilities::{
    select_capabilities, CapabilityError, CapabilityTier, RendererCapabilities,
    SelectedCapabilities, TextureFormat, TextureFormatSet,
};
pub use authoritative_feed::{
    AuthoritativeFeedError, AuthoritativeFeedOutcome, AuthoritativeFeedReplica,
    ReplicaTerrainChunk, DEFAULT_EVENT_HISTORY_CAPACITY,
    DEFAULT_TERRAIN_CHUNK_CAPACITY,
};
pub use frame::{
    CameraFrame, ClientRenderFrame, HudElement, HudElementKind, HudFrame, InstanceData,
    InstanceFrame, MeshDraw, MeshFrame, ParticleData, ParticleFrame, RenderError,
    RendererBackend, PresentationExtractor, ResourceId, SafeAreaInsets, SafeAreaRect,
    TerrainDraw, TerrainFrame, Viewport,
};
pub use input::{
    ControllerButtons, ControllerInput, InputDeviceMode, InputMapper, KeyboardMouseInput,
    MappedInput, NavigationEvent, NavigationEvents,
};
pub use orchestrator::{
    ClientFrameOrchestrator, FrameCadence, OrchestratorError,
    MAX_PREDICTION_STEPS_PER_FRAME, NANOSECONDS_PER_SECOND,
};
pub use platform::{
    AchievementId, ClientLifecycle, ClientPlatformServices, DeviceState, EntitlementId,
    IdentityId, LifecycleError, OnlineSessionState, PlatformError, ResumeOutcome,
    SaveNamespace, SessionKind, StorageState,
};
pub use session::{
    ClientAuthoritativeSession, ClientSessionError, ClientSessionOutcome,
    IncomingServerMessage,
};
pub use transport::{
    ClientTransport, InputBatchScheduler, InputBatchSchedulerError,
    LoopbackTransport, QuicTransport, SnapshotJitterEstimator, TransportKind,
    TransportMarker, TransportPolicy, WebSocketTransport,
};
pub use wire_snapshot::{
    apply_wire_snapshot, WireSnapshotError,
};

pub use aetherloom_core::{ClientReplica, ReplicaError};
pub use aetherloom_protocol::{PlayerCommand, PlayerId, AUTHORITATIVE_HZ};
