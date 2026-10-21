//! Nested complete-frame presentation: no output repair history or retained game image.

use super::{
    scene::{Scene, SceneElement},
    state::Compositor,
};
use crate::core::{FullscreenMode, OutputId};
use smithay::{
    backend::renderer::{
        Frame, Renderer,
        element::{
            Element, RenderElement,
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            utils::CropRenderElement,
        },
        gles::{GlesError, GlesRenderer, GlesTarget},
    },
    desktop::{PopupManager, layer_map_for_output},
    utils::{Physical, Rectangle, Size, Transform},
    wayland::shell::wlr_layer::Layer,
};

impl Compositor {
    /// Selected low-latency policy, even when a required overlay interrupts its presenter.
    pub fn requested_low_latency_output(&self) -> Option<OutputId> {
        self.runtime
            .desktop
            .fullscreen_slot()
            .filter(|s| s.active && s.mode == FullscreenMode::LowLatency)
            .and_then(|s| s.output)
    }

    /// Effective nested path and a bounded diagnostic reason; never changes requested mode.
    pub fn fullscreen_path(&self) -> (&'static str, &'static str) {
        let Some(slot) = self.runtime.desktop.fullscreen_slot() else {
            return ("normal", "empty");
        };
        if slot.mode != FullscreenMode::LowLatency {
            return ("normal", "desktop");
        }
        let Some(output) = slot.output else {
            return ("normal-overlay", "headless");
        };
        if !slot.active {
            return ("normal-overlay", "dormant");
        }
        if self.overview_present() {
            return ("normal-overlay", "overview");
        }
        if self.runtime.switcher.is_some() {
            return ("normal-overlay", "switcher");
        }
        if self.shell_previews_pending() {
            return ("normal-overlay", "shell-preview");
        }
        if self
            .placements
            .iter()
            .any(|p| self.runtime.desktop.is_fullscreen_transient(p.window))
        {
            return ("normal-overlay", "dialog");
        }
        let Some(region) = self.outputs.iter().find(|r| r.id == output) else {
            return ("normal-overlay", "headless");
        };
        if layer_map_for_output(&region.output)
            .layers_on(Layer::Overlay)
            .next()
            .is_some()
        {
            return ("normal-overlay", "overlay");
        }
        if !self
            .windows
            .get(&slot.window)
            .is_some_and(|w| w.mapped && w.committed_fullscreen)
        {
            return ("normal-overlay", "pending-commit");
        }
        ("full-frame-composed", "nested")
    }

    pub fn low_latency_output(&self) -> Option<OutputId> {
        (self.fullscreen_path().0 == "full-frame-composed")
            .then(|| self.requested_low_latency_output())
            .flatten()
    }

    /// Only the current committed application tree and its popups. The host clear is backing.
    pub fn low_latency_scene(&mut self, renderer: &mut GlesRenderer, output: OutputId) -> Scene {
        let mut scene = Scene {
            elements: Vec::new(),
            groups: Vec::new(),
        };
        let Some(slot) = self.runtime.desktop.fullscreen_slot() else {
            return scene;
        };
        let Some(region) = self.outputs.iter().find(|o| o.id == output) else {
            return scene;
        };
        let Some(entry) = self.windows.get(&slot.window) else {
            return scene;
        };
        let Some(top) = entry.window.toplevel() else {
            return scene;
        };
        let geometry = entry.window.geometry();
        let origin = (
            region.rect.x - geometry.loc.x,
            region.rect.y - geometry.loc.y,
        );
        // Popups use the committed XDG geometry just as ordinary rendering and hit testing do.
        for (surface, location) in PopupManager::popups_for_surface(top.wl_surface())
            .map(|(popup, offset)| {
                let location = (
                    region.rect.x + offset.x - popup.geometry().loc.x,
                    region.rect.y + offset.y - popup.geometry().loc.y,
                );
                (popup.wl_surface().clone(), location)
            })
            .chain(std::iter::once((top.wl_surface().clone(), origin)))
        {
            let parts: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(
                    renderer,
                    &surface,
                    location,
                    1.0,
                    1.0,
                    smithay::backend::renderer::element::Kind::Unspecified,
                );
            scene.elements.extend(parts.into_iter().filter_map(|part| {
                CropRenderElement::from_element(
                    part,
                    1.0,
                    Rectangle::new(
                        (region.rect.x, region.rect.y).into(),
                        (region.rect.width, region.rect.height).into(),
                    ),
                )
                .map(SceneElement::Surface)
            }));
        }
        // Opaque black backing protects transparent, undersized and offset content.
        if self
            .fullscreen_backing
            .as_ref()
            .is_none_or(|(rect, _)| *rect != region.rect)
        {
            self.fullscreen_backing = Some((
                region.rect,
                smithay::backend::renderer::element::solid::SolidColorBuffer::new(
                    (region.rect.width, region.rect.height),
                    [0.0, 0.0, 0.0, 1.0],
                ),
            ));
        }
        let buffer = &self.fullscreen_backing.as_ref().unwrap().1;
        scene.elements.push(SceneElement::Border(
            smithay::backend::renderer::element::solid::SolidColorRenderElement::from_buffer(
                buffer,
                (region.rect.x, region.rect.y),
                1.0,
                1.0,
                smithay::backend::renderer::element::Kind::Unspecified,
            ),
        ));
        scene
    }
}

/// Clear and paint every element directly. Does not query age, element damage, or repair state.
pub(super) fn render_full_frame(
    renderer: &mut GlesRenderer,
    target: &mut GlesTarget<'_>,
    size: Size<i32, Physical>,
    elements: &[SceneElement],
    background: [f32; 4],
) -> Result<(), GlesError> {
    let bounds = Rectangle::from_size(size);
    let mut frame = renderer.render(target, size, Transform::Flipped180)?;
    frame.clear(background.into(), &[bounds])?;
    for element in elements.iter().rev() {
        let geometry = element.geometry(1.0.into());
        if let Some(mut clip) = geometry.intersection(bounds) {
            clip.loc -= geometry.loc;
            element.draw(&mut frame, element.src(), geometry, &[clip], &[], None)?;
        }
    }
    // Same-context EGL swap orders this render; no CPU fence wait is needed.
    let _sync = frame.finish()?;
    Ok(())
}
