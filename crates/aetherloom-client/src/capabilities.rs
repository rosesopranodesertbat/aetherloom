/// Rendering quality tiers are ordered from the universally portable fallback
/// to the highest-cost presentation profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum CapabilityTier {
    Minimum = 0,
    Standard = 1,
    High = 2,
    Ultra = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TextureFormat {
    Rgba8Srgb = 0,
    Bgra8Srgb = 1,
    Bc7Srgb = 2,
    Astc4x4Srgb = 3,
    Etc2Srgb = 4,
    Rgba16Float = 5,
}

/// A compact, platform-neutral set of texture formats. Backends translate
/// these values to WebGPU, Metal, D3D12, Vulkan, or vendor-native formats.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextureFormatSet(u32);

impl TextureFormatSet {
    pub const NONE: Self = Self(0);
    pub const RGBA8_SRGB: Self = Self(1 << TextureFormat::Rgba8Srgb as u8);
    pub const BGRA8_SRGB: Self = Self(1 << TextureFormat::Bgra8Srgb as u8);
    pub const BC7_SRGB: Self = Self(1 << TextureFormat::Bc7Srgb as u8);
    pub const ASTC_4X4_SRGB: Self = Self(1 << TextureFormat::Astc4x4Srgb as u8);
    pub const ETC2_SRGB: Self = Self(1 << TextureFormat::Etc2Srgb as u8);
    pub const RGBA16_FLOAT: Self = Self(1 << TextureFormat::Rgba16Float as u8);
    const KNOWN_BITS: u32 = Self::RGBA8_SRGB.0
        | Self::BGRA8_SRGB.0
        | Self::BC7_SRGB.0
        | Self::ASTC_4X4_SRGB.0
        | Self::ETC2_SRGB.0
        | Self::RGBA16_FLOAT.0;

    pub const fn from_format(format: TextureFormat) -> Self {
        Self(1 << format as u8)
    }

    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits & Self::KNOWN_BITS)
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, format: TextureFormat) -> bool {
        self.0 & (1 << format as u8) != 0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn bits(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RendererCapabilities {
    pub supports_hdr_output: bool,
    pub supports_bloom: bool,
    pub texture_formats: TextureFormatSet,
    pub max_texture_dimension_2d: u32,
    pub max_shader_storage_buffers: u16,
    pub max_storage_buffer_bytes: u64,
    /// Dedicated VRAM available to this title. Zero means unified or unknown.
    pub dedicated_video_memory_bytes: u64,
    /// Budget exposed by a unified-memory platform. Zero means dedicated or
    /// unknown. Selection conservatively uses whichever non-zero budget exists.
    pub unified_memory_budget_bytes: u64,
}

impl RendererCapabilities {
    pub const fn available_memory_bytes(self) -> u64 {
        if self.dedicated_video_memory_bytes == 0 {
            self.unified_memory_budget_bytes
        } else if self.unified_memory_budget_bytes == 0 {
            self.dedicated_video_memory_bytes
        } else if self.dedicated_video_memory_bytes < self.unified_memory_budget_bytes {
            self.dedicated_video_memory_bytes
        } else {
            self.unified_memory_budget_bytes
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectedCapabilities {
    pub tier: CapabilityTier,
    pub hdr_enabled: bool,
    pub bloom_enabled: bool,
    pub color_format: TextureFormat,
    pub texture_budget_bytes: u64,
    pub particle_budget: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityError {
    MissingSrgbColorFormat,
    TextureDimensionTooSmall,
    InsufficientShaderStorageBuffers,
    InsufficientStorageBufferSize,
    InsufficientMemoryBudget,
}

const MIB: u64 = 1024 * 1024;

/// Selects the highest fully supported tier in a fixed order. The result does
/// not depend on backend enumeration order, making fallbacks reproducible
/// across browser, desktop, and console integrations.
pub fn select_capabilities(
    capabilities: RendererCapabilities,
) -> Result<SelectedCapabilities, CapabilityError> {
    validate_minimum(capabilities)?;

    let tier = if supports_ultra(capabilities) {
        CapabilityTier::Ultra
    } else if supports_high(capabilities) {
        CapabilityTier::High
    } else if supports_standard(capabilities) {
        CapabilityTier::Standard
    } else {
        CapabilityTier::Minimum
    };

    let color_format = select_color_format(capabilities.texture_formats);
    let (hdr_enabled, bloom_enabled, texture_budget_bytes, particle_budget) = match tier {
        CapabilityTier::Minimum => (false, false, 128 * MIB, 2_048),
        CapabilityTier::Standard => (false, capabilities.supports_bloom, 256 * MIB, 8_192),
        CapabilityTier::High => (
            capabilities.supports_hdr_output
                && capabilities
                    .texture_formats
                    .contains(TextureFormat::Rgba16Float),
            capabilities.supports_bloom,
            512 * MIB,
            32_768,
        ),
        CapabilityTier::Ultra => (true, true, 1024 * MIB, 65_536),
    };

    Ok(SelectedCapabilities {
        tier,
        hdr_enabled,
        bloom_enabled,
        color_format,
        texture_budget_bytes,
        particle_budget,
    })
}

fn validate_minimum(capabilities: RendererCapabilities) -> Result<(), CapabilityError> {
    if !capabilities
        .texture_formats
        .intersects(TextureFormatSet::RGBA8_SRGB.union(TextureFormatSet::BGRA8_SRGB))
    {
        return Err(CapabilityError::MissingSrgbColorFormat);
    }
    if capabilities.max_texture_dimension_2d < 4_096 {
        return Err(CapabilityError::TextureDimensionTooSmall);
    }
    if capabilities.max_shader_storage_buffers < 4 {
        return Err(CapabilityError::InsufficientShaderStorageBuffers);
    }
    if capabilities.max_storage_buffer_bytes < 16 * MIB {
        return Err(CapabilityError::InsufficientStorageBufferSize);
    }
    if capabilities.available_memory_bytes() < 256 * MIB {
        return Err(CapabilityError::InsufficientMemoryBudget);
    }
    Ok(())
}

fn supports_standard(capabilities: RendererCapabilities) -> bool {
    capabilities.max_texture_dimension_2d >= 8_192
        && capabilities.max_shader_storage_buffers >= 6
        && capabilities.max_storage_buffer_bytes >= 32 * MIB
        && capabilities.available_memory_bytes() >= 512 * MIB
}

fn supports_high(capabilities: RendererCapabilities) -> bool {
    supports_standard(capabilities)
        && capabilities.supports_bloom
        && capabilities.max_shader_storage_buffers >= 8
        && capabilities.max_storage_buffer_bytes >= 64 * MIB
        && capabilities.available_memory_bytes() >= 1024 * MIB
        && capabilities.texture_formats.intersects(
            TextureFormatSet::BC7_SRGB
                .union(TextureFormatSet::ASTC_4X4_SRGB)
                .union(TextureFormatSet::ETC2_SRGB),
        )
}

fn supports_ultra(capabilities: RendererCapabilities) -> bool {
    supports_high(capabilities)
        && capabilities.supports_hdr_output
        && capabilities
            .texture_formats
            .contains(TextureFormat::Rgba16Float)
        && capabilities.max_texture_dimension_2d >= 16_384
        && capabilities.max_shader_storage_buffers >= 12
        && capabilities.max_storage_buffer_bytes >= 128 * MIB
        && capabilities.available_memory_bytes() >= 2_048 * MIB
}

fn select_color_format(formats: TextureFormatSet) -> TextureFormat {
    // Stable preference order. Texture compression formats are asset formats,
    // while the first available sRGB swapchain format remains the color target.
    if formats.contains(TextureFormat::Bgra8Srgb) {
        TextureFormat::Bgra8Srgb
    } else {
        TextureFormat::Rgba8Srgb
    }
}
