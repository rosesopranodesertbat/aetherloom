#ifndef AETHERLOOM_CLIENT_H
#define AETHERLOOM_CLIENT_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define AETHERLOOM_CLIENT_ABI_VERSION 2u

#define AETHERLOOM_VENDOR_RESULT_OK 0u
#define AETHERLOOM_VENDOR_RESULT_UNAVAILABLE 1u
#define AETHERLOOM_VENDOR_RESULT_INVALID_ARGUMENT 2u
#define AETHERLOOM_VENDOR_RESULT_PERMISSION_DENIED 3u
#define AETHERLOOM_VENDOR_RESULT_RETRY 4u

#define AETHERLOOM_SERVICE_GRAPHICS 1u
#define AETHERLOOM_SERVICE_AUDIO 2u
#define AETHERLOOM_SERVICE_NETWORK 3u
#define AETHERLOOM_SERVICE_PLATFORM 4u
#define AETHERLOOM_SERVICE_CERTIFICATION 5u

#define AETHERLOOM_OP_GRAPHICS_QUERY_CAPABILITIES 0x0101u
#define AETHERLOOM_OP_GRAPHICS_CONFIGURE 0x0102u
#define AETHERLOOM_OP_GRAPHICS_SUBMIT_FRAME 0x0103u
#define AETHERLOOM_OP_GRAPHICS_RECOVER_DEVICE 0x0104u
#define AETHERLOOM_OP_AUDIO_SUBMIT_FRAME 0x0201u
#define AETHERLOOM_OP_AUDIO_SUSPEND 0x0202u
#define AETHERLOOM_OP_NETWORK_SEND_DATAGRAM 0x0301u
#define AETHERLOOM_OP_NETWORK_SEND_RELIABLE 0x0302u
#define AETHERLOOM_OP_NETWORK_POLL 0x0303u
#define AETHERLOOM_OP_PLATFORM_QUERY_IDENTITY 0x0401u
#define AETHERLOOM_OP_PLATFORM_QUERY_ENTITLEMENT 0x0402u
#define AETHERLOOM_OP_PLATFORM_LOAD_SAVE 0x0403u
#define AETHERLOOM_OP_PLATFORM_STORE_SAVE 0x0404u
#define AETHERLOOM_OP_PLATFORM_UNLOCK_ACHIEVEMENT 0x0405u
#define AETHERLOOM_OP_PLATFORM_POLL_EVENT 0x0406u
#define AETHERLOOM_OP_CERTIFICATION_QUERY_SAFE_AREA 0x0501u
#define AETHERLOOM_OP_CERTIFICATION_QUERY_USER 0x0502u

#define AETHERLOOM_EVENT_SUSPEND 1u
#define AETHERLOOM_EVENT_RESUME 2u
#define AETHERLOOM_EVENT_DEVICE_LOST 3u
#define AETHERLOOM_EVENT_IDENTITY_CHANGED 4u
#define AETHERLOOM_EVENT_STORAGE_UNAVAILABLE 5u
#define AETHERLOOM_EVENT_CONTROLLER_CONNECTED 6u
#define AETHERLOOM_EVENT_CONTROLLER_DISCONNECTED 7u

#define AETHERLOOM_INPUT_DEVICE_KEYBOARD_MOUSE 1u
#define AETHERLOOM_INPUT_DEVICE_CONTROLLER 2u

typedef struct AetherloomVendorApiHeader {
    uint32_t abi_version;
    uint32_t struct_size;
} AetherloomVendorApiHeader;

typedef struct AetherloomVendorByteSlice {
    const uint8_t *data;
    uint64_t len;
} AetherloomVendorByteSlice;

typedef struct AetherloomVendorMutableByteSlice {
    uint8_t *data;
    uint64_t capacity;
    uint64_t written;
} AetherloomVendorMutableByteSlice;

typedef struct AetherloomVendorIdentity {
    uint8_t bytes[16];
    uint32_t signed_in;
    uint32_t reserved;
} AetherloomVendorIdentity;

typedef struct AetherloomVendorSafeArea {
    uint32_t left_px;
    uint32_t top_px;
    uint32_t right_px;
    uint32_t bottom_px;
} AetherloomVendorSafeArea;

typedef struct AetherloomVendorRendererCapabilities {
    uint32_t feature_flags;
    uint32_t texture_format_bits;
    uint32_t max_texture_dimension_2d;
    uint32_t max_shader_storage_buffers;
    uint64_t max_storage_buffer_bytes;
    uint64_t dedicated_video_memory_bytes;
    uint64_t unified_memory_budget_bytes;
} AetherloomVendorRendererCapabilities;

typedef struct AetherloomVendorFrameTiming {
    uint64_t frame_index;
    uint64_t authoritative_tick;
    uint64_t monotonic_time_nanoseconds;
    uint64_t interpolation_numerator;
} AetherloomVendorFrameTiming;

typedef struct AetherloomVendorInputState {
    uint16_t player_slot;
    uint16_t action_flags;
    int16_t move_x;
    int16_t move_y;
    int16_t move_vertical;
    uint16_t look_yaw;
    int16_t look_pitch;
    uint8_t requested_spell;
    uint8_t input_device;
    uint8_t reserved[2];
} AetherloomVendorInputState;

typedef struct AetherloomVendorPlatformEvent {
    uint32_t kind;
    uint32_t flags;
    uint64_t data0;
    uint64_t data1;
} AetherloomVendorPlatformEvent;

typedef struct AetherloomVendorRenderFrameView {
    AetherloomVendorFrameTiming timing;
    AetherloomVendorByteSlice terrain;
    AetherloomVendorByteSlice meshes;
    AetherloomVendorByteSlice instances;
    AetherloomVendorByteSlice particles;
    AetherloomVendorByteSlice camera;
    AetherloomVendorByteSlice hud;
} AetherloomVendorRenderFrameView;

typedef uint32_t (*AetherloomVendorInvokeFn)(
    void *context,
    uint32_t operation,
    AetherloomVendorByteSlice request,
    AetherloomVendorMutableByteSlice *response);

typedef struct AetherloomVendorServiceApiV1 {
    AetherloomVendorApiHeader header;
    void *context;
    uint32_t service_kind;
    uint32_t reserved;
    AetherloomVendorInvokeFn invoke;
} AetherloomVendorServiceApiV1;

typedef struct AetherloomVendorClientApiV1 {
    AetherloomVendorApiHeader header;
    AetherloomVendorServiceApiV1 graphics;
    AetherloomVendorServiceApiV1 audio;
    AetherloomVendorServiceApiV1 network;
    AetherloomVendorServiceApiV1 platform;
    AetherloomVendorServiceApiV1 certification;
} AetherloomVendorClientApiV1;

#ifdef __cplusplus
}
#endif

#endif
