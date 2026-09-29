//! Backend-owned immutable textures; wallpaper never participates in hit-testing.

use crate::{
    config::WallpaperMode,
    core::Rect,
    runtime::wallpaper::{WallpaperImage, Wallpapers},
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem,
            element::{
                Kind,
                texture::{TextureBuffer, TextureRenderElement},
            },
            gles::{GlesRenderer, GlesTexture},
        },
    },
    utils::{Logical, Rectangle, Transform},
};
use std::{collections::BTreeMap, sync::Arc};

struct CachedImage {
    image: Arc<WallpaperImage>,
    // Remember failed imports too: warn once, retry only after resource reload.
    texture: Option<GlesTexture>,
    // Each output needs a distinct element identity for damage tracking, even
    // when several output buffers share one GPU texture.
    outputs: BTreeMap<String, TextureBuffer<GlesTexture>>,
}

/// Lives with the backend renderer, not compositor/protocol state.
#[derive(Default)]
pub(super) struct WallpaperCache {
    images: Vec<CachedImage>,
}

impl WallpaperCache {
    pub fn retain(&mut self, wallpapers: &Wallpapers) {
        self.images.retain(|cached| {
            wallpapers
                .iter()
                .any(|entry| Arc::ptr_eq(&entry.image, &cached.image))
        });
    }

    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        wallpapers: &Wallpapers,
        output: &str,
        rect: Rect,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        let prepared = wallpapers.for_output(output)?;
        let index = self.images.iter().position(|cached| Arc::ptr_eq(&cached.image, &prepared.image)).unwrap_or_else(|| {
            let (width, height) = prepared.image.size();
            let texture = match renderer.import_memory(prepared.image.rgba(), Fourcc::Abgr8888, (width as i32, height as i32).into(), false) {
                Ok(texture) => Some(texture),
                Err(error) => {
                    eprintln!("clear: wallpaper upload for {output:?} failed: {error}; using solid theme background until reload");
                    None
                }
            };
            self.images.push(CachedImage { image: prepared.image.clone(), texture, outputs: BTreeMap::new() });
            self.images.len() - 1
        });
        let cached = &mut self.images[index];
        let texture = cached.texture.as_ref()?;
        let buffer = cached.outputs.entry(output.to_owned()).or_insert_with(|| {
            TextureBuffer::from_texture(renderer, texture.clone(), 1, Transform::Normal, None)
        });
        let (source, destination) = placement(prepared.image.size(), rect, prepared.mode);
        Some(TextureRenderElement::from_texture_buffer(
            (f64::from(destination.x), f64::from(destination.y)),
            buffer,
            None,
            Some(source),
            Some((destination.width, destination.height).into()),
            Kind::Unspecified,
        ))
    }
}

// Compute a source crop rather than an oversized destination: all four modes
// stay within this output, including for giant images and off-origin outputs.
fn placement(
    image: (u32, u32),
    output: Rect,
    mode: WallpaperMode,
) -> (Rectangle<f64, Logical>, Rect) {
    let (width, height) = (f64::from(image.0), f64::from(image.1));
    let (ow, oh) = (f64::from(output.width), f64::from(output.height));
    let mut source = Rectangle::from_size((width, height).into());
    let mut destination = output;
    match mode {
        WallpaperMode::Fill => {
            let scale = (ow / width).max(oh / height);
            source.size = (ow / scale, oh / scale).into();
            source.loc = (
                (width - source.size.w) / 2.0,
                (height - source.size.h) / 2.0,
            )
                .into();
        }
        WallpaperMode::Fit => {
            let scale = (ow / width).min(oh / height);
            destination.width = ((width * scale).round() as i32).clamp(1, output.width);
            destination.height = ((height * scale).round() as i32).clamp(1, output.height);
            destination.x += (output.width - destination.width) / 2;
            destination.y += (output.height - destination.height) / 2;
        }
        WallpaperMode::Stretch => {}
        WallpaperMode::Center => {
            destination.width = output.width.min(image.0 as i32);
            destination.height = output.height.min(image.1 as i32);
            destination.x += (output.width - destination.width) / 2;
            destination.y += (output.height - destination.height) / 2;
            source.size = (f64::from(destination.width), f64::from(destination.height)).into();
            source.loc = (
                (width - source.size.w) / 2.0,
                (height - source.size.h) / 2.0,
            )
                .into();
        }
    }
    (source, destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_crops_symmetrically_and_uses_full_offset_output() {
        let output = Rect::new(-800, 30, 800, 600);
        let (source, destination) = placement((1600, 600), output, WallpaperMode::Fill);
        assert_eq!(destination, output);
        assert_eq!(
            source,
            Rectangle::new((400.0, 0.0).into(), (800.0, 600.0).into())
        );
        let (source, destination) = placement((400, 1200), output, WallpaperMode::Fill);
        assert_eq!(destination, output);
        assert_eq!(
            source,
            Rectangle::new((0.0, 450.0).into(), (400.0, 300.0).into())
        );
    }

    #[test]
    fn fit_stretch_and_center_preserve_expected_source_and_destination() {
        let output = Rect::new(800, 40, 800, 600);
        let full = Rectangle::from_size((1600.0, 600.0).into());
        assert_eq!(
            placement((1600, 600), output, WallpaperMode::Fit),
            (full, Rect::new(800, 190, 800, 300))
        );
        assert_eq!(
            placement((1600, 600), output, WallpaperMode::Stretch),
            (full, output)
        );
        assert_eq!(
            placement((400, 200), output, WallpaperMode::Center),
            (
                Rectangle::from_size((400.0, 200.0).into()),
                Rect::new(1000, 240, 400, 200)
            )
        );
        assert_eq!(
            placement((1600, 1200), output, WallpaperMode::Center),
            (
                Rectangle::new((400.0, 300.0).into(), (800.0, 600.0).into()),
                output
            )
        );
    }

    #[test]
    fn every_destination_is_bounded_by_its_output_not_a_spanning_group() {
        for output in [Rect::new(-37, 19, 801, 599), Rect::new(900, 5, 1, 1)] {
            for image in [(8192, 1), (1, 8192), (1, 1), (401, 301)] {
                for mode in [
                    WallpaperMode::Fill,
                    WallpaperMode::Fit,
                    WallpaperMode::Stretch,
                    WallpaperMode::Center,
                ] {
                    let (source, destination) = placement(image, output, mode);
                    assert!(destination.width > 0 && destination.height > 0);
                    assert!(destination.x >= output.x && destination.y >= output.y);
                    assert!(destination.x + destination.width <= output.x + output.width);
                    assert!(destination.y + destination.height <= output.y + output.height);
                    assert!(source.loc.x >= 0.0 && source.loc.y >= 0.0);
                    assert!(source.loc.x + source.size.w <= f64::from(image.0) + 1e-9);
                    assert!(source.loc.y + source.size.h <= f64::from(image.1) + 1e-9);
                }
            }
        }
    }
}
