//! Committed window composition shared by previews and presentation snapshots.

use super::{
    rounded::{RoundedShaders, WindowOutline},
    scene::{SceneElement, logical},
    state::Compositor,
    titlebar::{TitlebarCache, content_rect},
};
use crate::core::{Rect, WindowId};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, ContextId, Frame, Offscreen, Renderer, Texture,
            element::{
                Element, Id, Kind, RenderElement,
                surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
                texture::TextureRenderElement,
                utils::CropRenderElement,
            },
            gles::{GlesError, GlesRenderer, GlesTexture},
            utils::CommitCounter,
        },
    },
    utils::{Logical, Physical, Point, Rectangle, Transform},
};

/// Shared scratch and owned images are bounded to this edge, including oversized clients.
pub(super) const MAX_IMAGE_EDGE: i32 = 2048;

/// Appearance identity independent of the requested presentation destination.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct WindowImageIdentity {
    pub signature: Vec<(Id, CommitCounter)>,
    pub mappings: Vec<WindowImageMapping>,
    pub geometry_origin: (i32, i32),
    pub border_color: [f32; 4],
    pub source: Rect,
    pub frame: Rect,
    pub outlines: [(Rect, [f32; 4]); 2],
    pub committed_fullscreen: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct WindowImageMapping {
    geometry: Rectangle<i32, Physical>,
    source: [f64; 4],
    transform: Transform,
    alpha: f32,
}

/// Short-lived live elements; only the resulting WindowImage can outlive a client.
pub(super) struct PreparedWindowImage {
    pub identity: WindowImageIdentity,
    pub outline: WindowOutline,
    parts: Vec<SceneElement>,
}

impl PreparedWindowImage {
    /// Gather the latest committed body, subsurfaces and SSD; never include popups.
    pub fn prepare(
        state: &Compositor,
        renderer: &mut GlesRenderer,
        titlebars: &mut TitlebarCache,
        id: WindowId,
        focused: bool,
    ) -> Result<Option<Self>, GlesError> {
        let Some(entry) = state.windows.get(&id).filter(|entry| entry.mapped) else {
            return Ok(None);
        };
        let Some(top) = entry.window.toplevel() else {
            return Ok(None);
        };
        let Some(window) = state.runtime.desktop.window(id) else {
            return Ok(None);
        };
        let theme = &state.runtime.config.theme;
        let geometry = entry.window.geometry();
        let ssd = entry.uses_ssd();
        let frame = Rect::new(
            0,
            0,
            geometry.size.w.max(1),
            geometry
                .size
                .h
                .max(1)
                .saturating_add(if ssd { theme.titlebar.height } else { 0 }),
        );
        let outline = entry.outline(frame, theme);
        let source = outline.outer.rect;
        let content = content_rect(frame, ssd, theme.titlebar.height);
        let location: Point<i32, Physical> =
            (content.x - geometry.loc.x, content.y - geometry.loc.y).into();
        let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
            render_elements_from_surface_tree(
                renderer,
                top.wl_surface(),
                location,
                1.0,
                1.0,
                Kind::Unspecified,
            );
        if surfaces.is_empty() {
            return Ok(None);
        }
        let mut parts = Vec::new();
        if ssd {
            parts.extend(
                titlebars
                    .elements(
                        renderer,
                        id,
                        &window.title,
                        frame,
                        focused,
                        window.maximized,
                        frame,
                        &theme.titlebar,
                        &state.runtime.titlebar_assets,
                        &window.app_id,
                    )?
                    .into_iter()
                    .filter_map(|bar| {
                        CropRenderElement::from_element(
                            bar,
                            1.0,
                            logical(frame).to_physical_precise_round(1.0),
                        )
                    })
                    .map(SceneElement::Titlebar),
            );
        }
        parts.extend(
            surfaces
                .into_iter()
                .filter_map(|surface| {
                    CropRenderElement::from_element(
                        surface,
                        1.0,
                        logical(content).to_physical_precise_round(1.0),
                    )
                })
                .map(SceneElement::Surface),
        );
        let signature = parts
            .iter()
            .map(|part| (part.id().clone(), part.current_commit()))
            .collect();
        let mappings = parts
            .iter()
            .map(|part| {
                let source = part.src();
                WindowImageMapping {
                    geometry: part.geometry(1.0.into()),
                    source: [source.loc.x, source.loc.y, source.size.w, source.size.h],
                    transform: part.transform(),
                    alpha: part.alpha(),
                }
            })
            .collect();
        let identity = WindowImageIdentity {
            signature,
            mappings,
            geometry_origin: (geometry.loc.x, geometry.loc.y),
            border_color: if focused {
                theme.active_border
            } else {
                theme.inactive_border
            },
            source,
            frame,
            outlines: [
                (outline.outer.rect, outline.outer.radii),
                (outline.inner.rect, outline.inner.radii),
            ],
            committed_fullscreen: entry.committed_fullscreen,
        };
        Ok(Some(Self {
            identity,
            outline,
            parts,
        }))
    }
}

/// An independent GPU image with original committed frame and silhouette metadata.
/// It retains no Wayland surface, popup, client handle, backdrop, or desktop state.
#[derive(Clone)]
pub(super) struct WindowImage {
    pub texture: GlesTexture,
    pub source: Rect,
    pub frame: Rect,
    pub outline: WindowOutline,
    context_id: ContextId<GlesTexture>,
}

impl WindowImage {
    pub fn size(&self) -> (i32, i32) {
        self.texture.size().into()
    }

    pub fn bytes(&self) -> usize {
        let (width, height) = self.size();
        width as usize * height as usize * 4
    }

    /// Scale the complete source, preserving fractional origin until pixel placement.
    /// Smithay multiplies premultiplied RGB and alpha by the same opacity.
    pub fn texture_element(
        &self,
        destination: Rectangle<f64, Logical>,
        alpha: f32,
    ) -> Option<TextureRenderElement<GlesTexture>> {
        let size = destination_size(destination, alpha)?;
        let (width, height) = self.size();
        Some(TextureRenderElement::from_static_texture(
            Id::new(),
            self.context_id.clone(),
            (destination.loc.x, destination.loc.y),
            self.texture.clone(),
            1,
            Transform::Normal,
            Some(alpha),
            Some(Rectangle::from_size(
                (f64::from(width), f64::from(height)).into(),
            )),
            Some(size.into()),
            None,
            Kind::Unspecified,
        ))
    }

    /// Apply an output crop after transforming the original complete silhouette.
    pub fn element(
        &self,
        destination: Rectangle<f64, Logical>,
        alpha: f32,
        output_clip: Rect,
    ) -> Option<CropRenderElement<TextureRenderElement<GlesTexture>>> {
        CropRenderElement::from_element(
            self.texture_element(destination, alpha)?,
            1.0,
            logical(output_clip).to_physical_precise_round(1.0),
        )
    }
}

fn destination_size(destination: Rectangle<f64, Logical>, alpha: f32) -> Option<(i32, i32)> {
    if !alpha.is_finite()
        || !(0.0..=1.0).contains(&alpha)
        || alpha == 0.0
        || ![
            destination.loc.x,
            destination.loc.y,
            destination.size.w,
            destination.size.h,
        ]
        .iter()
        .all(|v| v.is_finite())
        || destination.size.w <= 0.0
        || destination.size.h <= 0.0
        || destination.size.w > f64::from(i32::MAX)
        || destination.size.h > f64::from(i32::MAX)
    {
        return None;
    }
    let width = destination.size.w.round().max(1.0);
    let height = destination.size.h.round().max(1.0);
    let left = destination.loc.x.round();
    let top = destination.loc.y.round();
    let low = f64::from(i32::MIN);
    let high = f64::from(i32::MAX);
    if left < low || top < low || left + width > high || top + height > high {
        return None;
    }
    Some((width as i32, height as i32))
}

struct SourceBuffers {
    size: (i32, i32),
    body: GlesTexture,
    masked: GlesTexture,
}

/// Two shared bounded scratch images; retained snapshots belong to each caller's cache.
#[derive(Default)]
pub(super) struct WindowImageComposer {
    buffers: Option<SourceBuffers>,
}

impl WindowImageComposer {
    pub fn clear(&mut self) {
        self.buffers = None;
    }

    /// Bake the original mask before downscaling. Reusable must no longer be sampled
    /// by any retained snapshot or outstanding render element; omission allocates anew.
    pub fn compose(
        &mut self,
        renderer: &mut GlesRenderer,
        shaders: &RoundedShaders,
        prepared: &PreparedWindowImage,
        size: (i32, i32),
        reusable: Option<GlesTexture>,
    ) -> Result<WindowImage, GlesError> {
        let source = prepared.identity.source;
        let native_size = source_size(source);
        let size = (
            size.0.clamp(1, MAX_IMAGE_EDGE),
            size.1.clamp(1, MAX_IMAGE_EDGE),
        );
        let mut texture = match reusable.filter(|texture| texture.size() == size.into()) {
            Some(texture) => texture,
            None => renderer.create_buffer(Fourcc::Abgr8888, size.into())?,
        };
        if self
            .buffers
            .as_ref()
            .is_none_or(|buffers| buffers.size.0 < native_size.0 || buffers.size.1 < native_size.1)
        {
            let capacity = self.buffers.as_ref().map_or(native_size, |buffers| {
                (
                    buffers.size.0.max(native_size.0),
                    buffers.size.1.max(native_size.1),
                )
            });
            self.buffers = Some(SourceBuffers {
                size: capacity,
                body: renderer.create_buffer(Fourcc::Abgr8888, capacity.into())?,
                masked: renderer.create_buffer(Fourcc::Abgr8888, capacity.into())?,
            });
        }
        let buffers = self.buffers.as_mut().expect("prepared source buffers");
        let sx = f64::from(native_size.0) / f64::from(source.width);
        let sy = f64::from(native_size.1) / f64::from(source.height);
        let sync = {
            let mut target = renderer.bind(&mut buffers.body)?;
            let mut frame = renderer.render(&mut target, native_size.into(), Transform::Normal)?;
            frame.clear([0.0; 4].into(), &[Rectangle::from_size(native_size.into())])?;
            for part in prepared.parts.iter().rev() {
                let geometry = part.geometry(1.0.into());
                let left = ((f64::from(geometry.loc.x) - f64::from(source.x)) * sx).round() as i32;
                let top = ((f64::from(geometry.loc.y) - f64::from(source.y)) * sy).round() as i32;
                let right = ((f64::from(geometry.loc.x) + f64::from(geometry.size.w)
                    - f64::from(source.x))
                    * sx)
                    .round() as i32;
                let bottom = ((f64::from(geometry.loc.y) + f64::from(geometry.size.h)
                    - f64::from(source.y))
                    * sy)
                    .round() as i32;
                let destination = Rectangle::new(
                    (left, top).into(),
                    ((right - left).max(1), (bottom - top).max(1)).into(),
                );
                part.draw(
                    &mut frame,
                    part.src(),
                    destination,
                    &[Rectangle::from_size(destination.size)],
                    &[],
                    None,
                )?;
            }
            frame.finish()?
        };
        renderer.wait(&sync)?;
        let body = TextureRenderElement::from_static_texture(
            Id::new(),
            renderer.context_id(),
            (0.0, 0.0),
            buffers.body.clone(),
            1,
            Transform::Normal,
            None,
            Some(Rectangle::from_size(
                (f64::from(native_size.0), f64::from(native_size.1)).into(),
            )),
            Some(native_size.into()),
            None,
            Kind::Unspecified,
        );
        let masked = shaders.mask_texture(
            body,
            prepared
                .outline
                .scaled(Rect::new(0, 0, native_size.0, native_size.1))
                .offscreen(native_size.1),
            prepared.identity.border_color,
            native_size.1,
        );
        let sync = {
            let mut target = renderer.bind(&mut buffers.masked)?;
            let mut frame = renderer.render(&mut target, native_size.into(), Transform::Normal)?;
            let area = Rectangle::from_size(native_size.into());
            frame.clear([0.0; 4].into(), &[area])?;
            masked.draw(&mut frame, masked.src(), area, &[area], &[], None)?;
            frame.finish()?
        };
        renderer.wait(&sync)?;
        let masked = TextureRenderElement::from_static_texture(
            Id::new(),
            renderer.context_id(),
            (0.0, 0.0),
            buffers.masked.clone(),
            1,
            Transform::Normal,
            None,
            Some(Rectangle::from_size(
                (f64::from(native_size.0), f64::from(native_size.1)).into(),
            )),
            Some(native_size.into()),
            None,
            Kind::Unspecified,
        );
        let sync = {
            let mut target = renderer.bind(&mut texture)?;
            let mut frame = renderer.render(&mut target, size.into(), Transform::Normal)?;
            let area = Rectangle::from_size(size.into());
            frame.clear([0.0; 4].into(), &[area])?;
            RenderElement::<GlesRenderer>::draw(
                &masked,
                &mut frame,
                masked.src(),
                area,
                &[area],
                &[],
                None,
            )?;
            frame.finish()?
        };
        renderer.wait(&sync)?;
        Ok(WindowImage {
            texture,
            source,
            frame: prepared.identity.frame,
            outline: prepared.outline,
            context_id: renderer.context_id(),
        })
    }
}

fn source_size(source: Rect) -> (i32, i32) {
    let scale = (f64::from(MAX_IMAGE_EDGE) / f64::from(source.width.max(1)))
        .min(f64::from(MAX_IMAGE_EDGE) / f64::from(source.height.max(1)))
        .min(1.0);
    (
        (f64::from(source.width) * scale).round().max(1.0) as i32,
        (f64::from(source.height) * scale).round().max(1.0) as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_keeps_the_whole_oversized_source_within_scratch_budget() {
        assert_eq!(source_size(Rect::new(-4, -4, 1600, 1000)), (1600, 1000));
        assert_eq!(source_size(Rect::new(-4, -4, 8192, 4096)), (2048, 1024));
        assert_eq!(source_size(Rect::new(-4, -4, 32, 8192)), (8, 2048));
    }

    #[test]
    fn fractional_destination_and_opacity_are_validated_at_pixel_placement() {
        let destination = Rectangle::new((11.25, -7.5).into(), (120.75, 30.25).into());
        assert_eq!(destination_size(destination, 0.5), Some((121, 30)));
        assert!(destination_size(destination, 0.0).is_none());
        assert!(destination_size(destination, f32::NAN).is_none());
        assert!(destination_size(destination, 1.01).is_none());
        assert!(
            destination_size(
                Rectangle::new((f64::NAN, 0.0).into(), (1.0, 1.0).into()),
                1.0
            )
            .is_none()
        );
        assert!(
            destination_size(Rectangle::new((1e100, 0.0).into(), (1.0, 1.0).into()), 1.0).is_none()
        );
        assert!(
            destination_size(
                Rectangle::new((f64::from(i32::MAX), 0.0).into(), (1.0, 1.0).into()),
                1.0
            )
            .is_none()
        );
        assert!(
            destination_size(Rectangle::new((0.0, 0.0).into(), (0.0, 1.0).into()), 1.0).is_none()
        );
    }

    #[test]
    fn mapping_changes_invalidate_images_without_buffer_damage() {
        let mapping = WindowImageMapping {
            geometry: Rectangle::new((12, 24).into(), (100, 80).into()),
            source: [0.0, 0.0, 100.0, 80.0],
            transform: Transform::Normal,
            alpha: 1.0,
        };
        let mut moved = mapping.clone();
        moved.geometry.loc.x += 1;
        assert_ne!(mapping, moved);
        let mut viewport = mapping.clone();
        viewport.source[0] = 0.5;
        assert_ne!(mapping, viewport);
    }
}
