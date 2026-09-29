//! Bounded CPU image preparation. No filesystem IO is performed by the renderer.

use crate::config::{Config, WallpaperMode};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};

/// Maximum width or height accepted before image decoding.
pub const MAX_DIMENSION: u32 = 8192;
/// Maximum encoded file size read into memory.
pub const MAX_ENCODED_BYTES: u64 = 16 * 1024 * 1024;
/// Per-image RGBA size and decoder allocation limit.
pub const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
/// Maximum resident RGBA bytes across distinct configured image paths.
pub const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

/// Immutable, premultiplied RGBA8 image shared by outputs using the same path.
#[derive(Debug)]
pub struct WallpaperImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

impl WallpaperImage {
    /// Native pixel dimensions, validated before decode.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    /// Premultiplied RGBA8 pixels, ready for adapter upload.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }
}

/// Prepared image and placement policy for one output.
#[derive(Debug, Clone)]
pub struct PreparedWallpaper {
    /// Shared immutable CPU resource; identity changes on each successful reload.
    pub image: Arc<WallpaperImage>,
    /// Placement in the output's full rectangle, not its reserved usable area.
    pub mode: WallpaperMode,
}

/// A complete image set prepared before any live configuration is replaced.
#[derive(Debug, Default)]
pub struct Wallpapers {
    outputs: BTreeMap<String, PreparedWallpaper>,
}

impl Wallpapers {
    /// Read and decode all selected resources once, deduplicating identical paths.
    pub fn prepare(config: &Config) -> Result<Self, String> {
        let mut images = BTreeMap::new();
        let mut remaining = MAX_TOTAL_BYTES;
        // Validate even a global resource currently replaced on every output.
        for path in config.wallpaper.path.iter().chain(
            config
                .wallpaper
                .outputs
                .values()
                .filter_map(|entry| entry.path.as_ref()),
        ) {
            if !images.contains_key(path) {
                let image = decode(path, remaining)
                    .map_err(|error| format!("wallpaper {}: {error}", path.display()))?;
                remaining -= image.rgba.len() as u64;
                images.insert(path.clone(), Arc::new(image));
            }
        }
        let outputs = config
            .outputs
            .iter()
            .filter_map(|output| {
                let (path, mode) = config.wallpaper.for_output(&output.name);
                path.map(|path| {
                    (
                        output.name.clone(),
                        PreparedWallpaper {
                            image: images[path].clone(),
                            mode,
                        },
                    )
                })
            })
            .collect();
        Ok(Self { outputs })
    }

    /// Return the prepared image for a connector; absence means solid theme background.
    pub fn for_output(&self, name: &str) -> Option<&PreparedWallpaper> {
        self.outputs.get(name)
    }

    /// Prepared output selections, used to evict stale adapter textures.
    pub fn iter(&self) -> impl Iterator<Item = &PreparedWallpaper> {
        self.outputs.values()
    }
}

fn check_size(width: u32, height: u32, decoded_bytes: u64, remaining: u64) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(format!("dimensions must be in 1..={MAX_DIMENSION}"));
    }
    let rgba_bytes = u64::from(width) * u64::from(height) * 4;
    if decoded_bytes > MAX_IMAGE_BYTES || rgba_bytes > MAX_IMAGE_BYTES {
        return Err("image exceeds the 64 MiB decoded resource limit".into());
    }
    if rgba_bytes > remaining {
        return Err("images exceed the 128 MiB aggregate RGBA limit".into());
    }
    Ok(())
}

fn decode(path: &Path, remaining: u64) -> Result<WallpaperImage, String> {
    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("expected a regular PNG or JPEG file".into());
    }
    if metadata.len() > MAX_ENCODED_BYTES {
        return Err("encoded image exceeds 16 MiB".into());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_ENCODED_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ENCODED_BYTES {
        return Err("encoded image exceeds 16 MiB".into());
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    if !matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)) {
        return Err("only PNG and JPEG wallpapers are supported".into());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_BYTES);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    check_size(width, height, decoder.total_bytes(), remaining)?;
    let mut rgba = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| e.to_string())?
        .into_rgba8()
        .into_raw();
    // Smithay's texture blending expects premultiplied alpha.
    for pixel in rgba.chunks_exact_mut(4) {
        for channel in 0..3 {
            pixel[channel] = ((u16::from(pixel[channel]) * u16::from(pixel[3]) + 127) / 255) as u8;
        }
    }
    Ok(WallpaperImage {
        width,
        height,
        rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_are_checked_before_decode() {
        assert!(check_size(8192, 2048, MAX_IMAGE_BYTES, MAX_IMAGE_BYTES).is_ok());
        assert!(check_size(8193, 1, 32772, MAX_TOTAL_BYTES).is_err());
        assert!(check_size(u32::MAX, u32::MAX, u64::MAX, MAX_TOTAL_BYTES).is_err());
        assert!(check_size(8192, 8192, MAX_IMAGE_BYTES * 4, MAX_TOTAL_BYTES).is_err());
        assert!(check_size(4096, 4096, MAX_IMAGE_BYTES + 1, MAX_TOTAL_BYTES).is_err());
        assert!(check_size(2, 2, 16, 15).is_err());
        assert!(check_size(2, 2, 16, 16).is_ok());
    }
}
