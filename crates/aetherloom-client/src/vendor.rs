use core::ffi::c_void;
use core::mem::size_of;

pub const AETHERLOOM_CLIENT_ABI_VERSION: u32 = 2;

pub const VENDOR_RESULT_OK: u32 = 0;
pub const VENDOR_RESULT_UNAVAILABLE: u32 = 1;
pub const VENDOR_RESULT_INVALID_ARGUMENT: u32 = 2;
pub const VENDOR_RESULT_PERMISSION_DENIED: u32 = 3;
pub const VENDOR_RESULT_RETRY: u32 = 4;

pub const SERVICE_GRAPHICS: u32 = 1;
pub const SERVICE_AUDIO: u32 = 2;
pub const SERVICE_NETWORK: u32 = 3;
pub const SERVICE_PLATFORM: u32 = 4;
pub const SERVICE_CERTIFICATION: u32 = 5;

pub const OP_GRAPHICS_QUERY_CAPABILITIES: u32 = 0x0101;
pub const OP_GRAPHICS_CONFIGURE: u32 = 0x0102;
pub const OP_GRAPHICS_SUBMIT_FRAME: u32 = 0x0103;
pub const OP_GRAPHICS_RECOVER_DEVICE: u32 = 0x0104;
pub const OP_AUDIO_SUBMIT_FRAME: u32 = 0x0201;
pub const OP_AUDIO_SUSPEND: u32 = 0x0202;
pub const OP_NETWORK_SEND_DATAGRAM: u32 = 0x0301;
pub const OP_NETWORK_SEND_RELIABLE: u32 = 0x0302;
pub const OP_NETWORK_POLL: u32 = 0x0303;
pub const OP_PLATFORM_QUERY_IDENTITY: u32 = 0x0401;
pub const OP_PLATFORM_QUERY_ENTITLEMENT: u32 = 0x0402;
pub const OP_PLATFORM_LOAD_SAVE: u32 = 0x0403;
pub const OP_PLATFORM_STORE_SAVE: u32 = 0x0404;
pub const OP_PLATFORM_UNLOCK_ACHIEVEMENT: u32 = 0x0405;
pub const OP_PLATFORM_POLL_EVENT: u32 = 0x0406;
pub const OP_CERTIFICATION_QUERY_SAFE_AREA: u32 = 0x0501;
pub const OP_CERTIFICATION_QUERY_USER: u32 = 0x0502;

pub const EVENT_SUSPEND: u32 = 1;
pub const EVENT_RESUME: u32 = 2;
pub const EVENT_DEVICE_LOST: u32 = 3;
pub const EVENT_IDENTITY_CHANGED: u32 = 4;
pub const EVENT_STORAGE_UNAVAILABLE: u32 = 5;
pub const EVENT_CONTROLLER_CONNECTED: u32 = 6;
pub const EVENT_CONTROLLER_DISCONNECTED: u32 = 7;

pub const INPUT_DEVICE_KEYBOARD_MOUSE: u8 = 1;
pub const INPUT_DEVICE_CONTROLLER: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct VendorApiHeader {
    pub abi_version: u32,
    pub struct_size: u32,
}

impl VendorApiHeader {
    pub const fn for_type<T>() -> Self {
        Self {
            abi_version: AETHERLOOM_CLIENT_ABI_VERSION,
            struct_size: size_of::<T>() as u32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct VendorByteSlice {
    pub data: *const u8,
    pub len: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct VendorMutableByteSlice {
    pub data: *mut u8,
    pub capacity: u64,
    pub written: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct VendorIdentity {
    pub bytes: [u8; 16],
    pub signed_in: u32,
    pub reserved: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct VendorSafeArea {
    pub left_px: u32,
    pub top_px: u32,
    pub right_px: u32,
    pub bottom_px: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct VendorRendererCapabilities {
    /// Bit 0: HDR output. Bit 1: bloom.
    pub feature_flags: u32,
    pub texture_format_bits: u32,
    pub max_texture_dimension_2d: u32,
    pub max_shader_storage_buffers: u32,
    pub max_storage_buffer_bytes: u64,
    pub dedicated_video_memory_bytes: u64,
    pub unified_memory_budget_bytes: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct VendorFrameTiming {
    pub frame_index: u64,
    pub authoritative_tick: u64,
    pub monotonic_time_nanoseconds: u64,
    pub interpolation_numerator: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct VendorInputState {
    pub player_slot: u16,
    pub action_flags: u16,
    pub move_x: i16,
    pub move_y: i16,
    pub move_vertical: i16,
    pub look_yaw: u16,
    pub look_pitch: i16,
    pub requested_spell: u8,
    pub input_device: u8,
    pub reserved: [u8; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct VendorPlatformEvent {
    pub kind: u32,
    pub flags: u32,
    pub data0: u64,
    pub data1: u64,
}

/// Presentation sections are backend-neutral encoded packets owned by the
/// shared client for the duration of a submit call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct VendorRenderFrameView {
    pub timing: VendorFrameTiming,
    pub terrain: VendorByteSlice,
    pub meshes: VendorByteSlice,
    pub instances: VendorByteSlice,
    pub particles: VendorByteSlice,
    pub camera: VendorByteSlice,
    pub hud: VendorByteSlice,
}

pub type VendorInvokeFn = Option<
    extern "C" fn(
        context: *mut c_void,
        operation: u32,
        request: VendorByteSlice,
        response: *mut VendorMutableByteSlice,
    ) -> u32,
>;

/// One service table. Each table is independently versioned and may be absent
/// (`invoke == None`) when a platform does not provide that service.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct VendorServiceApiV1 {
    pub header: VendorApiHeader,
    pub context: *mut c_void,
    pub service_kind: u32,
    pub reserved: u32,
    pub invoke: VendorInvokeFn,
}

/// Root table supplied by a proprietary platform adapter to the shared static
/// library. It is data-only: this crate does not pretend to implement a vendor
/// graphics, audio, network, storage, identity, or certification SDK.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct VendorClientApiV1 {
    pub header: VendorApiHeader,
    pub graphics: VendorServiceApiV1,
    pub audio: VendorServiceApiV1,
    pub network: VendorServiceApiV1,
    pub platform: VendorServiceApiV1,
    pub certification: VendorServiceApiV1,
}
