use super::{
    blur::BlurMask,
    rounded::{RoundedShaders, RoundedShape, RoundedSurface, WindowOutline},
    state::Compositor,
    titlebar::{TitlebarCache, TitlebarPart, content_rect, titlebar_hit},
    wallpaper::WallpaperCache,
};
use crate::core::{Desktop, OutputId, Placement, Rect, WindowId, WindowRole};
use smithay::{
    backend::renderer::{
        element::{
            Element, Kind,
            solid::{SolidColorBuffer, SolidColorRenderElement},
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            texture::TextureRenderElement,
            utils::CropRenderElement,
        },
        gles::{GlesError, GlesRenderer, GlesTexture},
    },
    desktop::{
        PopupManager, WindowSurfaceType, layer_map_for_output,
        space::SpaceElement,
        utils::{output_update, send_frames_surface_tree},
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Physical, Point, Rectangle, SERIAL_COUNTER},
    wayland::shell::wlr_layer::Layer,
};
use std::{collections::BTreeSet, time::Duration};

smithay::backend::renderer::element::render_elements! {
    pub(super) SceneElement<=GlesRenderer>;
    Surface=CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Border=SolidColorRenderElement,
    Titlebar=CropRenderElement<TextureRenderElement<GlesTexture>>,
    RoundedSurface=RoundedSurface,
    Wallpaper=TextureRenderElement<GlesTexture>,
}

/// Elements and logical trees are both front-to-back. A tree is blurred only once.
pub(super) struct Scene {
    pub elements: Vec<SceneElement>,
    pub groups: Vec<SceneGroup>,
}

pub(super) struct SceneGroup {
    pub elements: std::ops::Range<usize>,
    pub clip: Rectangle<i32, Physical>,
    pub mask: Option<BlurMask>,
}

// Optical bounds come from the original tree, never an output-dependent crop.
fn client_tree_shape(elements: &[WaylandSurfaceRenderElement<GlesRenderer>]) -> Option<Rect> {
    elements
        .iter()
        .map(|e| e.geometry(1.0.into()))
        .reduce(|a, b| a.merge(b))
        .map(|r| Rect::new(r.loc.x, r.loc.y, r.size.w, r.size.h))
}

fn glass_layer_shape(rect: Rect, namespace: &str, root: bool) -> Option<RoundedShape> {
    if !root {
        return None;
    }
    let fit = |radius: f32| radius.min(rect.width.min(rect.height).max(0) as f32 / 2.0);
    let radius = if namespace.starts_with("clear-glass-pill-") {
        fit(rect.height.min(rect.width).max(0) as f32 / 2.0)
    } else if namespace.starts_with("clear-glass-rounded-") {
        fit(24.0)
    } else {
        return None;
    };
    Some(RoundedShape {
        rect,
        radii: [radius; 4],
    })
}

pub(super) fn logical(rect: Rect) -> Rectangle<i32, Logical> {
    Rectangle::new((rect.x, rect.y).into(), (rect.width, rect.height).into())
}
fn physical(rect: Rect) -> Rectangle<i32, Physical> {
    Rectangle::new((rect.x, rect.y).into(), (rect.width, rect.height).into())
}
fn contains(rect: Rect, point: Point<f64, Logical>) -> bool {
    logical(rect).contains(point.to_i32_floor())
}

pub(super) enum HitOwner {
    Window(WindowId),
    Decoration(WindowId, TitlebarPart),
    Layer(WlSurface),
}

pub(super) struct SurfaceHit {
    pub surface: WlSurface,
    pub origin: Point<f64, Logical>,
    pub owner: HitOwner,
    pub scale: Point<f64, Logical>,
}

impl Compositor {
    pub fn reconcile(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.shell_dirty = true;
        self.runtime.refresh_overview();
        if self.runtime.overview.is_none() {
            self.overview_press = None;
        }
        if self.overview_drag.as_ref().is_some_and(|drag| {
            self.runtime.overview.as_ref().is_none_or(|session| {
                session.workspace != drag.workspace
                    || session.output != drag.output
                    || !session.windows.contains(&drag.window)
            }) || self
                .outputs
                .iter()
                .find(|o| o.id == drag.output)
                .is_none_or(|o| o.rect != drag.output_rect)
        }) {
            self.overview_drag = None;
            self.overview_press = None;
        }
        self.refresh_layers();
        let layer_focus = self.layer_keyboard_focus();
        for region in &self.outputs {
            let mut layers = layer_map_for_output(&region.output);
            layers.arrange();
            let area = layers.non_exclusive_zone();
            let usable = Rect::new(
                region.rect.x + area.loc.x,
                region.rect.y + area.loc.y,
                area.size.w,
                area.size.h,
            );
            if self
                .runtime
                .desktop
                .output(region.id)
                .is_some_and(|o| o.area != usable)
            {
                self.runtime.desktop.set_output_area(region.id, usable);
            }
        }
        if self.drag.as_ref().is_some_and(|drag| {
            self.runtime
                .desktop
                .window(drag.window)
                .is_none_or(|w| w.minimized || w.maximized || w.fullscreen)
        }) {
            self.end_drag();
        }
        if self.titlebar_drag.as_ref().is_some_and(|(id, _)| {
            self.windows.get(id).is_none_or(|entry| !entry.uses_ssd())
                || self
                    .runtime
                    .desktop
                    .window(*id)
                    .is_none_or(|window| window.minimized || window.maximized)
        }) {
            self.titlebar_drag = None;
        }
        // Launcher frames are client-sized. Recalculate the inset before placement
        // after a theme reload, even if the client has not committed another buffer.
        let titlebar_height = self.runtime.config.theme.titlebar.height;
        for (id, entry) in &self.windows {
            if entry.mapped {
                let size = entry.window.geometry().size;
                self.runtime.desktop.set_window_committed_size(
                    *id,
                    size.w,
                    size.h
                        .saturating_add(if entry.uses_ssd() { titlebar_height } else { 0 }),
                );
            }
        }
        let placements = self.runtime.placements();
        let overview_live = self.overview_live_windows();
        for (id, entry) in &self.windows {
            if let Some(window) = self.runtime.desktop.window(*id)
                && let Some(top) = entry.window.toplevel()
            {
                top.with_pending_state(|pending| {
                    if window.maximized {
                        pending.states.set(xdg_toplevel::State::Maximized);
                    } else {
                        pending.states.unset(xdg_toplevel::State::Maximized);
                    }
                    if window.fullscreen {
                        pending.states.set(xdg_toplevel::State::Fullscreen);
                    } else {
                        pending.states.unset(xdg_toplevel::State::Fullscreen);
                    }
                    if window.minimized && !overview_live.contains(id) {
                        pending.states.set(xdg_toplevel::State::Suspended);
                    } else {
                        pending.states.unset(xdg_toplevel::State::Suspended);
                    }
                });
            }
            if !placements.iter().any(|p| p.window == *id) {
                self.space.unmap_elem(&entry.window);
                entry.window.set_activated(false);
                if let Some(top) = entry.window.toplevel()
                    && top.is_initial_configure_sent()
                {
                    top.send_pending_configure();
                }
            }
        }
        for placement in &placements {
            let Some(entry) = self.windows.get(&placement.window) else {
                continue;
            };
            let window = &entry.window;
            entry.remember_frame(placement.rect);
            entry.prepare_decoration_configure();
            let requested_content = content_rect(
                placement.rect,
                entry.pending_ssd()
                    && !self
                        .runtime
                        .desktop
                        .window(placement.window)
                        .is_some_and(|w| w.fullscreen),
                titlebar_height,
            );
            let content = content_rect(placement.rect, entry.uses_ssd(), titlebar_height);
            if let Some(top) = window.toplevel() {
                top.with_pending_state(|pending| {
                    // Launcher geometry is client-owned; a size hint here creates
                    // a configure/commit feedback loop when the client resizes.
                    let launcher = self
                        .runtime
                        .desktop
                        .window(placement.window)
                        .is_some_and(|window| window.role == WindowRole::Launcher);
                    pending.size = (!launcher)
                        .then_some((requested_content.width, requested_content.height).into());
                    for tiled in [
                        xdg_toplevel::State::TiledLeft,
                        xdg_toplevel::State::TiledRight,
                        xdg_toplevel::State::TiledTop,
                        xdg_toplevel::State::TiledBottom,
                    ] {
                        if placement.tiled {
                            pending.states.set(tiled);
                        } else {
                            pending.states.unset(tiled);
                        }
                    }
                });
                window.set_activated(
                    placement.focused
                        && self.host_focused
                        && layer_focus.is_none()
                        && self.runtime.overview.is_none(),
                );
                top.send_pending_configure();
            }
            self.space
                .map_element(window.clone(), (content.x, content.y), false);
            self.space.raise_element(window, false);
        }
        self.placements = placements;
        self.refresh_animation_targets();
        let focus = layer_focus
            .or_else(|| {
                self.runtime
                    .desktop
                    .focused_window()
                    .and_then(|id| self.windows.get(&id))
                    .and_then(|w| w.window.toplevel())
                    .map(|t| t.wl_surface().clone())
            })
            .filter(|_| self.host_focused && self.runtime.overview.is_none());
        let keyboard = self.seat.get_keyboard().expect("keyboard is initialized");
        if !keyboard.is_grabbed() && keyboard.current_focus() != focus {
            keyboard.set_focus(self, focus, SERIAL_COUNTER.next_serial());
        }
        // Workspace switches must also remove pointer focus from now-hidden surfaces.
        if self.drag.is_none() {
            let pointer = self.seat.get_pointer().expect("pointer is initialized");
            if !pointer.is_grabbed() {
                let location = pointer.current_location();
                let target = self.surface_under(location);
                pointer.motion(
                    self,
                    target,
                    &smithay::input::pointer::MotionEvent {
                        location,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: self.start.elapsed().as_millis() as u32,
                    },
                );
                pointer.frame(self);
            }
        }
    }

    fn layer_under(&self, point: Point<f64, Logical>, kinds: &[Layer]) -> Option<SurfaceHit> {
        for region in &self.outputs {
            if !contains(region.rect, point) {
                continue;
            }
            let origin: Point<i32, Logical> = (region.rect.x, region.rect.y).into();
            let layers = layer_map_for_output(&region.output);
            for kind in kinds {
                // A surface with an empty input region must not obscure the
                // next layer, even when its bounding box contains the pointer.
                for layer in layers.layers_on(*kind).rev() {
                    let Some(geometry) = layers.layer_geometry(layer) else {
                        continue;
                    };
                    if let Some((surface, offset)) = layer.surface_under(
                        point - (origin + geometry.loc).to_f64(),
                        WindowSurfaceType::ALL,
                    ) {
                        return Some(SurfaceHit {
                            surface,
                            origin: (origin + geometry.loc + offset).to_f64(),
                            owner: HitOwner::Layer(layer.wl_surface().clone()),
                            scale: (1.0, 1.0).into(),
                        });
                    }
                }
            }
        }
        None
    }

    pub fn surface_under(
        &self,
        point: Point<f64, Logical>,
    ) -> Option<(super::presentation_input::PointerFocus, Point<f64, Logical>)> {
        self.hit_test(point).and_then(|hit| {
            (!matches!(hit.owner, HitOwner::Decoration(..))).then_some((
                super::presentation_input::PointerFocus::new(hit.surface, hit.scale),
                hit.origin,
            ))
        })
    }

    fn fullscreen_outputs(&self) -> BTreeSet<OutputId> {
        self.placements
            .iter()
            .filter_map(|p| self.runtime.desktop.window(p.window))
            .filter(|w| w.fullscreen)
            .filter_map(|w| w.output)
            .collect()
    }

    fn elevated_window(&self, id: WindowId, fullscreen_outputs: &BTreeSet<OutputId>) -> bool {
        self.runtime.desktop.window(id).is_some_and(|w| {
            w.fullscreen
                || (w.role == WindowRole::Launcher
                    && w.output.is_some_and(|o| fullscreen_outputs.contains(&o)))
        })
    }

    pub fn hit_test(&self, point: Point<f64, Logical>) -> Option<SurfaceHit> {
        if self.runtime.overview.is_some() {
            return None;
        }
        if let Some(layer) = self.layer_under(point, &[Layer::Overlay]) {
            return Some(layer);
        }
        let full_outputs = self.fullscreen_outputs();
        let over_fullscreen_output = self
            .outputs
            .iter()
            .any(|region| full_outputs.contains(&region.id) && contains(region.rect, point));
        if !over_fullscreen_output && let Some(layer) = self.layer_under(point, &[Layer::Top]) {
            return Some(layer);
        }
        if let Some(hit) = self.window_hit(point, true, &full_outputs) {
            return Some(hit);
        }
        if let Some(layer) = self.layer_under(point, &[Layer::Top]) {
            return Some(layer);
        }
        self.window_hit(point, false, &full_outputs)
            .or_else(|| self.layer_under(point, &[Layer::Bottom, Layer::Background]))
    }

    fn window_hit(
        &self,
        point: Point<f64, Logical>,
        elevated: bool,
        fullscreen_outputs: &BTreeSet<OutputId>,
    ) -> Option<SurfaceHit> {
        for p in self.placements.iter().rev() {
            if self.elevated_window(p.window, fullscreen_outputs) != elevated {
                continue;
            }
            if !self
                .placement_clips(p)
                .iter()
                .any(|clip| contains(*clip, point))
            {
                continue;
            }
            let Some(entry) = self.windows.get(&p.window) else {
                continue;
            };
            let ssd = entry.uses_ssd();
            let content = content_rect(p.rect, ssd, self.runtime.config.theme.titlebar.height);
            let origin: Point<i32, Logical> = (content.x, content.y).into();
            let surface_origin = origin - entry.window.geometry().loc;
            if let Some((surface, offset)) = entry.window.surface_under(
                point - surface_origin.to_f64(),
                if ((p.tiled || ssd || entry.committed_fullscreen) && !contains(content, point))
                    || (!entry.committed_fullscreen
                        && self.runtime.config.theme.corner_radius.is_rounded()
                        && !entry
                            .outline(p.rect, &self.runtime.config.theme)
                            .inner
                            .contains(point))
                {
                    WindowSurfaceType::POPUP
                } else {
                    WindowSurfaceType::ALL
                },
            ) {
                return Some(SurfaceHit {
                    surface,
                    origin: (surface_origin + offset).to_f64(),
                    owner: HitOwner::Window(p.window),
                    scale: (1.0, 1.0).into(),
                });
            }
            if ssd
                && entry
                    .outline(p.rect, &self.runtime.config.theme)
                    .inner
                    .contains(point)
                && let Some(part) = titlebar_hit(p.rect, point, &self.runtime.config.theme.titlebar)
                && let Some(top) = entry.window.toplevel()
            {
                return Some(SurfaceHit {
                    surface: top.wl_surface().clone(),
                    origin: origin.to_f64(),
                    owner: HitOwner::Decoration(p.window, part),
                    scale: (1.0, 1.0).into(),
                });
            }
        }
        None
    }

    fn placement_clips(&self, placement: &crate::core::Placement) -> Vec<Rect> {
        placement_clips(
            &self.runtime.desktop,
            placement,
            self.outputs.iter().map(|region| (region.id, region.rect)),
        )
    }

    pub fn scene_elements(
        &self,
        renderer: &mut GlesRenderer,
        wallpapers: &mut WallpaperCache,
        titlebars: &mut TitlebarCache,
        rounded: Option<&RoundedShaders>,
        blur: bool,
    ) -> Result<Scene, GlesError> {
        let mut elements = Vec::new();
        let mut groups = Vec::new();
        titlebars.retain(|id| self.windows.get(&id).is_some_and(|entry| entry.uses_ssd()));
        let full_outputs = self.fullscreen_outputs();
        self.layer_elements(renderer, &[Layer::Overlay], &mut elements, &mut groups);
        self.layer_elements_filtered(
            renderer,
            &[Layer::Top],
            &mut elements,
            &mut groups,
            |output| !full_outputs.contains(&output),
        );

        for elevated in [true, false] {
            if !elevated {
                self.layer_elements_filtered(
                    renderer,
                    &[Layer::Top],
                    &mut elements,
                    &mut groups,
                    |output| full_outputs.contains(&output),
                );
            }
            for placement in self.placements.iter().rev() {
                if self.elevated_window(placement.window, &full_outputs) != elevated {
                    continue;
                }
                let Some(entry) = self.windows.get(&placement.window) else {
                    continue;
                };
                let theme = &self.runtime.config.theme;
                let rounded = rounded
                    .filter(|_| !entry.committed_fullscreen && theme.corner_radius.is_rounded());
                let outline = entry.outline(placement.rect, theme);
                let ssd = entry.uses_ssd();
                let content = content_rect(placement.rect, ssd, theme.titlebar.height);
                for clip in self.placement_clips(placement) {
                    let titlebar = if ssd {
                        let window = self
                            .runtime
                            .desktop
                            .window(placement.window)
                            .expect("placed window");
                        let title = if window.title.is_empty() {
                            &window.app_id
                        } else {
                            &window.title
                        };
                        titlebars.elements(
                            renderer,
                            placement.window,
                            title,
                            placement.rect,
                            placement.focused
                                && self.host_focused
                                && self.layer_keyboard_focus().is_none(),
                            window.maximized,
                            clip,
                            &theme.titlebar,
                            &self.runtime.titlebar_assets,
                            &window.app_id,
                        )?
                    } else {
                        Vec::new()
                    };
                    let Some(top) = entry.window.toplevel() else {
                        continue;
                    };
                    // Popups may extend beyond the parent, but not its workspace viewport.
                    for (popup, offset) in PopupManager::popups_for_surface(top.wl_surface()) {
                        let location: Point<i32, Physical> = (
                            content.x + offset.x - popup.geometry().loc.x,
                            content.y + offset.y - popup.geometry().loc.y,
                        )
                            .into();
                        let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                            render_elements_from_surface_tree(
                                renderer,
                                popup.wl_surface(),
                                location,
                                1.0,
                                1.0,
                                Kind::Unspecified,
                            );
                        let shape = client_tree_shape(&surfaces);
                        let start = elements.len();
                        elements.extend(
                            surfaces
                                .into_iter()
                                .filter_map(|e| {
                                    CropRenderElement::from_element(e, 1.0, physical(clip))
                                })
                                .map(SceneElement::Surface),
                        );
                        groups.push(SceneGroup {
                            elements: start..elements.len(),
                            clip: physical(clip),
                            mask: shape.map(BlurMask::ClientAlpha),
                        });
                    }
                    let offset = entry.window.geometry().loc;
                    let location: Point<i32, Physical> =
                        (content.x - offset.x, content.y - offset.y).into();
                    let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                        render_elements_from_surface_tree(
                            renderer,
                            top.wl_surface(),
                            location,
                            1.0,
                            1.0,
                            Kind::Unspecified,
                        );
                    let color = if placement.focused {
                        theme.active_border
                    } else {
                        theme.inactive_border
                    };
                    let start = elements.len();
                    if let Some(shaders) = rounded {
                        if let Some(window) = shaders.window(
                            renderer,
                            surfaces,
                            content,
                            titlebar,
                            outline,
                            color,
                            self.host_size.h,
                            physical(clip),
                        )? {
                            elements.push(SceneElement::RoundedSurface(window));
                        }
                        groups.push(SceneGroup {
                            elements: start..elements.len(),
                            clip: physical(clip),
                            mask: Some(BlurMask::Window(outline.outer)),
                        });
                        continue;
                    }
                    // A client can commit an old or oversized buffer after a new configure.
                    // Constrain its body without clipping the separately rendered popups.
                    elements.extend(
                        titlebar
                            .into_iter()
                            .filter_map(|bar| {
                                CropRenderElement::from_element(bar, 1.0, physical(clip))
                            })
                            .map(SceneElement::Titlebar),
                    );
                    let body_clip = if placement.tiled || ssd || entry.committed_fullscreen {
                        physical(clip).intersection(physical(content))
                    } else {
                        Some(physical(clip))
                    };
                    if let Some(body_clip) = body_clip {
                        elements.extend(
                            surfaces
                                .into_iter()
                                .filter_map(|e| CropRenderElement::from_element(e, 1.0, body_clip))
                                .map(SceneElement::Surface),
                        );
                    }
                    let b = if entry.committed_fullscreen {
                        0
                    } else {
                        theme.border_width
                    };
                    if b > 0 {
                        for rect in square_border_regions(outline, clip, blur || ssd) {
                            let buffer = SolidColorBuffer::new(rect.size, premultiply(color));
                            elements.push(SceneElement::Border(
                                SolidColorRenderElement::from_buffer(
                                    &buffer,
                                    (rect.loc.x, rect.loc.y),
                                    1.0,
                                    1.0,
                                    Kind::Unspecified,
                                ),
                            ));
                        }
                    }
                    groups.push(SceneGroup {
                        elements: start..elements.len(),
                        clip: physical(clip),
                        mask: Some(BlurMask::SquareWindow(outline.outer)),
                    });
                }
            }
        }
        self.layer_elements(
            renderer,
            &[Layer::Bottom, Layer::Background],
            &mut elements,
            &mut groups,
        );
        wallpapers.retain(&self.runtime.wallpapers);
        for region in &self.outputs {
            if let Some(element) = wallpapers.element(
                renderer,
                &self.runtime.wallpapers,
                &region.output.name(),
                region.rect,
            ) {
                let start = elements.len();
                elements.push(SceneElement::Wallpaper(element));
                groups.push(SceneGroup {
                    elements: start..elements.len(),
                    clip: physical(region.rect),
                    mask: None,
                });
            }
        }
        Ok(Scene { elements, groups })
    }

    fn layer_elements(
        &self,
        renderer: &mut GlesRenderer,
        kinds: &[Layer],
        elements: &mut Vec<SceneElement>,
        groups: &mut Vec<SceneGroup>,
    ) {
        self.layer_elements_filtered(renderer, kinds, elements, groups, |_| true);
    }

    fn layer_elements_filtered(
        &self,
        renderer: &mut GlesRenderer,
        kinds: &[Layer],
        elements: &mut Vec<SceneElement>,
        groups: &mut Vec<SceneGroup>,
        include: impl Fn(OutputId) -> bool,
    ) {
        for kind in kinds {
            for region in &self.outputs {
                if !include(region.id) {
                    continue;
                }
                let map = layer_map_for_output(&region.output);
                for layer in map.layers_on(*kind).rev() {
                    let Some(geometry) = map.layer_geometry(layer) else {
                        continue;
                    };
                    let location: Point<i32, Physical> = (
                        region.rect.x + geometry.loc.x,
                        region.rect.y + geometry.loc.y,
                    )
                        .into();
                    // Match Smithay's front-to-back order, retaining separate popup
                    // trees so each menu samples the scene including its parent.
                    let popups = PopupManager::popups_for_surface(layer.wl_surface());
                    for (surface, location) in popups
                        .map(|(popup, offset)| {
                            let offset = offset - popup.geometry().loc;
                            (
                                popup.wl_surface().clone(),
                                location + Point::from((offset.x, offset.y)),
                            )
                        })
                        .chain(std::iter::once((layer.wl_surface().clone(), location)))
                    {
                        let start = elements.len();
                        let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                            render_elements_from_surface_tree(
                                renderer,
                                &surface,
                                location,
                                1.0,
                                1.0,
                                Kind::Unspecified,
                            );
                        let shape = client_tree_shape(&surfaces);
                        elements.extend(
                            surfaces
                                .into_iter()
                                .filter_map(|e| {
                                    CropRenderElement::from_element(e, 1.0, physical(region.rect))
                                })
                                .map(SceneElement::Surface),
                        );
                        let root = surface == *layer.wl_surface();
                        let mask = shape.map(|rect| {
                            glass_layer_shape(rect, layer.namespace(), root)
                                .map_or(BlurMask::ClientAlpha(rect), BlurMask::GlassLayer)
                        });
                        groups.push(SceneGroup {
                            elements: start..elements.len(),
                            clip: physical(region.rect),
                            mask,
                        });
                    }
                }
            }
        }
    }

    /// Apply preview output membership after Space refresh; preview trees are not
    /// mapped into Space and must keep toolkits aware of their visible output.
    pub fn refresh_overview_outputs(&mut self) {
        let live = self.overview_live_windows();
        let output = self.runtime.overview.as_ref().and_then(|session| {
            self.outputs
                .iter()
                .find(|r| r.id == session.output)
                .map(|r| r.output.clone())
        });
        self.overview_outputs.retain(|id, (window, previous)| {
            if live.contains(id) && output.as_ref() == Some(previous) {
                return true;
            }
            if let Some(top) = window.toplevel() {
                output_update(previous, None, top.wl_surface());
                // Restore ordinary output ownership, including per-surface clips.
                window.refresh();
            }
            false
        });
        if let Some(output) = output {
            for id in live {
                let entry = &self.windows[&id];
                if let Some(top) = entry.window.toplevel() {
                    // Source geometry, never thumbnail geometry or popup trees.
                    output_update(&output, Some(entry.window.geometry()), top.wl_surface());
                    self.overview_outputs
                        .insert(id, (entry.window.clone(), output.clone()));
                }
            }
        }
    }

    pub fn frame_callbacks(&self) {
        let mut ordinary_live = BTreeSet::new();
        for placement in &self.placements {
            let Some(entry) = self.windows.get(&placement.window) else {
                continue;
            };
            if let Some(region) = self.outputs.iter().find(|o| {
                logical(o.rect).overlaps(logical(placement.rect))
                    && self
                        .placement_clips(placement)
                        .iter()
                        .any(|clip| logical(o.rect).overlaps(logical(*clip)))
            }) {
                ordinary_live.insert(placement.window);
                entry.window.send_frame(
                    &region.output,
                    self.frame_time,
                    Some(Duration::ZERO),
                    |_, _| Some(region.output.clone()),
                );
            }
        }
        if let Some(session) = &self.runtime.overview
            && let Some(region) = self.outputs.iter().find(|r| r.id == session.output)
        {
            for id in self.overview_live_windows().difference(&ordinary_live) {
                if let Some(top) = self.windows[id].window.toplevel() {
                    // Includes offscreen placements and visible desktop miniatures;
                    // None honors the throttle without pretending to be scanout.
                    send_frames_surface_tree(
                        top.wl_surface(),
                        &region.output,
                        self.frame_time,
                        Some(Duration::from_millis(33)),
                        |_, _| None,
                    );
                }
            }
        }
        for region in &self.outputs {
            for layer in layer_map_for_output(&region.output).layers() {
                layer.send_frame(
                    &region.output,
                    self.frame_time,
                    Some(Duration::ZERO),
                    |_, _| Some(region.output.clone()),
                );
            }
        }
    }
}

fn placement_clips(
    desktop: &Desktop,
    placement: &Placement,
    outputs: impl Iterator<Item = (OutputId, Rect)>,
) -> Vec<Rect> {
    let Some(window) = desktop.window(placement.window) else {
        return Vec::new();
    };
    let viewport = placement.clip.or_else(|| {
        placement
            .tiled
            .then(|| {
                window
                    .output
                    .and_then(|id| desktop.output(id))
                    .map(|output| output.area)
            })
            .flatten()
    });
    // Use the actual output union, not its bounding box: independent workspace
    // groups and gaps between monitors must stay invisible. Both hit-testing
    // and rendering consume these clips, including for separate popup trees.
    outputs
        .filter(|(id, _)| desktop.workspace_for_output(*id) == Some(window.workspace))
        .filter_map(|(_, rect)| {
            let rect = viewport.map_or(Some(logical(rect)), |clip| {
                logical(rect).intersection(logical(clip))
            })?;
            Some(Rect::new(rect.loc.x, rect.loc.y, rect.size.w, rect.size.h))
        })
        .collect()
}

// Preserve the legacy backing only for undecorated, unblurred clients. SSD bars
// can be transparent too: their backdrop must never be filled with border color.
fn square_border_regions(
    outline: WindowOutline,
    clip: Rect,
    ring_only: bool,
) -> Vec<Rectangle<i32, Logical>> {
    let Some(rect) = logical(outline.outer.rect).intersection(logical(clip)) else {
        return Vec::new();
    };
    if ring_only {
        Rectangle::subtract_rects_many_in_place(vec![rect], [logical(outline.inner.rect)])
    } else {
        vec![rect]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Command, WorkspaceId};

    #[test]
    fn square_blur_or_ssd_border_does_not_fill_translucent_content() {
        let mut theme = crate::decoration::Theme::default();
        theme.border_width = 2;
        let outline = WindowOutline::new(Rect::new(10, 10, 20, 20), &theme);
        let clip = Rect::new(0, 0, 100, 100);
        let ring = square_border_regions(outline, clip, true);
        assert!(
            ring.iter()
                .all(|rect| rect.intersection(logical(outline.inner.rect)).is_none())
        );
        assert_eq!(
            ring.iter()
                .map(|rect| rect.size.w * rect.size.h)
                .sum::<i32>(),
            24 * 24 - 20 * 20
        );
        assert_eq!(
            square_border_regions(outline, clip, false),
            vec![logical(outline.outer.rect)]
        );
    }

    #[test]
    fn glass_layer_namespace_supplies_only_fitted_optical_radii() {
        let rect = Rect::new(12, 10, 616, 44);
        let pill = glass_layer_shape(rect, "clear-glass-pill-panel", true).unwrap();
        assert_eq!(pill.rect, rect);
        assert_eq!(pill.radii, [22.0; 4]);
        let rounded = glass_layer_shape(rect, "clear-glass-rounded-panel", true).unwrap();
        assert_eq!(rounded.radii, [22.0; 4]);
        assert!(glass_layer_shape(rect, "clear-example-panel", true).is_none());
        assert!(glass_layer_shape(rect, "clear-glass-pill-panel", false).is_none());
    }

    fn fixture() -> (Desktop, Placement, [(OutputId, Rect); 2]) {
        let mut desktop = Desktop::new();
        let outputs = [
            (OutputId(1), Rect::new(0, 0, 400, 300)),
            (OutputId(2), Rect::new(450, 0, 400, 250)),
        ];
        for (id, rect) in outputs {
            desktop.add_output(id, format!("output-{}", id.0), rect);
        }
        desktop.command(Command::FocusOutput(OutputId(1)));
        desktop.add_window(WindowId(1), String::new(), String::new());
        let placement = Placement {
            window: WindowId(1),
            rect: Rect::new(-50, -20, 900, 350),
            clip: None,
            focused: true,
            tiled: false,
        };
        (desktop, placement, outputs)
    }

    #[test]
    fn floats_clip_to_visible_workspace_group_without_filling_monitor_gaps() {
        let (mut desktop, placement, outputs) = fixture();
        assert_eq!(
            placement_clips(&desktop, &placement, outputs.into_iter()),
            vec![outputs[0].1]
        );
        desktop.command(Command::StretchAll);
        let clips = placement_clips(&desktop, &placement, outputs.into_iter());
        assert_eq!(clips, outputs.map(|(_, rect)| rect));
        for point in [(-1.0, 10.0), (425.0, 10.0), (460.0, 275.0)] {
            assert!(!clips.iter().any(|clip| contains(*clip, point.into())));
        }
        assert!(
            clips
                .iter()
                .any(|clip| contains(*clip, (500.0, 100.0).into()))
        );
        desktop.command(Command::SwitchWorkspace(WorkspaceId(3)));
        assert!(placement_clips(&desktop, &placement, outputs.into_iter()).is_empty());
    }

    #[test]
    fn tile_and_scroll_clips_respect_reservations_but_float_clips_do_not() {
        let (mut desktop, mut placement, outputs) = fixture();
        let usable = Rect::new(0, 32, 400, 268);
        desktop.set_output_area(OutputId(1), usable);
        assert_eq!(
            placement_clips(&desktop, &placement, outputs.into_iter()),
            vec![outputs[0].1]
        );
        placement.tiled = true;
        assert_eq!(
            placement_clips(&desktop, &placement, outputs.into_iter()),
            vec![usable]
        );
        placement.clip = Some(Rect::new(8, 40, 384, 252));
        assert_eq!(
            placement_clips(&desktop, &placement, outputs.into_iter()),
            vec![placement.clip.unwrap()]
        );
        desktop.remove_window(placement.window);
        assert!(placement_clips(&desktop, &placement, outputs.into_iter()).is_empty());
    }
}

pub(super) fn premultiply(color: [f32; 4]) -> [f32; 4] {
    [
        color[0] * color[3],
        color[1] * color[3],
        color[2] * color[3],
        color[3],
    ]
}
