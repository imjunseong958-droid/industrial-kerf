//! What a texel is. A texture here is a buffer of its texels, row after row with nothing
//! between, so a kernel finds one by its place and reads it by its kind.

/// The kinds of texel Kerf's kernels read and write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Texel {
    Rgba32Float,
    R32Float,
    Rg32Float,
    Rgba8Unorm,
    Rgba16Uint,
    R8Uint,
    R16Uint,
}

impl Texel {
    /// The kind a texture of `format` holds, where it is one of these.
    pub fn of(format: wgpu::TextureFormat) -> Option<Self> {
        Some(match format {
            wgpu::TextureFormat::Rgba32Float => Self::Rgba32Float,
            wgpu::TextureFormat::R32Float => Self::R32Float,
            wgpu::TextureFormat::Rg32Float => Self::Rg32Float,
            wgpu::TextureFormat::Rgba8Unorm => Self::Rgba8Unorm,
            wgpu::TextureFormat::Rgba16Uint => Self::Rgba16Uint,
            wgpu::TextureFormat::R8Uint => Self::R8Uint,
            wgpu::TextureFormat::R16Uint => Self::R16Uint,
            _ => return None,
        })
    }

    /// How many bytes one is.
    pub fn bytes(self) -> usize {
        match self {
            Self::Rgba32Float => 16,
            Self::R32Float | Self::Rgba8Unorm => 4,
            Self::Rg32Float | Self::Rgba16Uint => 8,
            Self::R8Uint => 1,
            Self::R16Uint => 2,
        }
    }

    /// Its number in `prelude.cl`.
    pub fn code(self) -> i32 {
        match self {
            Self::Rgba32Float => 0,
            Self::R32Float => 1,
            Self::Rg32Float => 2,
            Self::Rgba8Unorm => 3,
            Self::Rgba16Uint => 4,
            Self::R8Uint => 5,
            Self::R16Uint => 6,
        }
    }
}
