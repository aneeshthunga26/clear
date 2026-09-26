use super::state::Compositor;
use crate::core::{Desktop, OutputId, Placement, Rect, WindowId, WindowRole};
use smithay::{
    backend::renderer::{
        element::{
            AsRenderElements, Kind,
            solid::{SolidColorBuffer, SolidColorRenderElement},
            surface::{WaylandSurfaceRenderElement, render_elements_from_surface_tree},
            utils::CropRenderElement,
        },
        gles::GlesRenderer,
    },
    desktop::{PopupManager, WindowSurfaceType, layer_map_for_output},
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Physical, Point, Rectangle, SERIAL_COUNTER},
    wayland::shell::wlr_layer::Layer,
};
use std::time::Duration;

smithay::backend::renderer::element::render_elements! {
    pub(super) SceneElement<=GlesRenderer>;
    Surface=CropRenderElement<WaylandSurfaceRenderElement<GlesRenderer>>,
    Border=SolidColorRenderElement,
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
    Layer(WlSurface),
}

pub(super) struct SurfaceHit {
    pub surface: WlSurface,
    pub origin: Point<f64, Logical>,
    pub owner: HitOwner,
}

impl Compositor {
    pub fn reconcile(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.shell_dirty = true;
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
        let placements = self.runtime.placements();
        for (id, entry) in &self.windows {
            if !placements.iter().any(|p| p.window == *id) {
                self.space.unmap_elem(&entry.window);
                entry.window.set_activated(false);
                if let Some(top) = entry.window.toplevel() {
                    top.send_pending_configure();
                }
            }
        }
        for placement in &placements {
            let Some(entry) = self.windows.get(&placement.window) else {
                continue;
            };
            let window = &entry.window;
            if let Some(top) = window.toplevel() {
                top.with_pending_state(|pending| {
                    // Launcher geometry is client-owned; a size hint here creates
                    // a configure/commit feedback loop when the client resizes.
                    let launcher = self
                        .runtime
                        .desktop
                        .window(placement.window)
                        .is_some_and(|window| window.role == WindowRole::Launcher);
                    pending.size =
                        (!launcher).then_some((placement.rect.width, placement.rect.height).into());
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
                window
                    .set_activated(placement.focused && self.host_focused && layer_focus.is_none());
                top.send_pending_configure();
            }
            self.space
                .map_element(window.clone(), (placement.rect.x, placement.rect.y), false);
            self.space.raise_element(window, false);
        }
        self.placements = placements;
        let focus = layer_focus
            .or_else(|| {
                self.runtime
                    .desktop
                    .focused_window()
                    .and_then(|id| self.windows.get(&id))
                    .and_then(|w| w.window.toplevel())
                    .map(|t| t.wl_surface().clone())
            })
            .filter(|_| self.host_focused);
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
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.hit_test(point).map(|hit| (hit.surface, hit.origin))
    }

    pub fn hit_test(&self, point: Point<f64, Logical>) -> Option<SurfaceHit> {
        if let Some(layer) = self.layer_under(point, &[Layer::Overlay, Layer::Top]) {
            return Some(layer);
        }
        for p in self.placements.iter().rev() {
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
            let origin: Point<i32, Logical> = (p.rect.x, p.rect.y).into();
            let surface_origin = origin - entry.window.geometry().loc;
            if let Some((surface, offset)) = entry.window.surface_under(
                point - surface_origin.to_f64(),
                if p.tiled && !contains(p.rect, point) {
                    WindowSurfaceType::POPUP
                } else {
                    WindowSurfaceType::ALL
                },
            ) {
                return Some(SurfaceHit {
                    surface,
                    origin: (surface_origin + offset).to_f64(),
                    owner: HitOwner::Window(p.window),
                });
            }
        }
        self.layer_under(point, &[Layer::Bottom, Layer::Background])
    }

    fn placement_clips(&self, placement: &crate::core::Placement) -> Vec<Rect> {
        placement_clips(
            &self.runtime.desktop,
            placement,
            self.outputs.iter().map(|region| (region.id, region.rect)),
        )
    }

    pub fn scene_elements(&self, renderer: &mut GlesRenderer) -> Vec<SceneElement> {
        let mut elements = Vec::new();
        self.layer_elements(renderer, &[Layer::Overlay, Layer::Top], &mut elements);

        for placement in self.placements.iter().rev() {
            let Some(entry) = self.windows.get(&placement.window) else {
                continue;
            };
            for clip in self.placement_clips(placement) {
                let Some(top) = entry.window.toplevel() else {
                    continue;
                };
                // Popups may extend beyond the parent, but not its workspace viewport.
                for (popup, offset) in PopupManager::popups_for_surface(top.wl_surface()) {
                    let location: Point<i32, Physical> = (
                        placement.rect.x + offset.x - popup.geometry().loc.x,
                        placement.rect.y + offset.y - popup.geometry().loc.y,
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
                    elements.extend(
                        surfaces
                            .into_iter()
                            .filter_map(|e| CropRenderElement::from_element(e, 1.0, physical(clip)))
                            .map(SceneElement::Surface),
                    );
                }
                let offset = entry.window.geometry().loc;
                let location: Point<i32, Physical> =
                    (placement.rect.x - offset.x, placement.rect.y - offset.y).into();
                let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                    render_elements_from_surface_tree(
                        renderer,
                        top.wl_surface(),
                        location,
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    );
                // A client can commit an old or oversized buffer after a new configure.
                // Constrain its body without clipping the separately rendered popups.
                let body_clip = if placement.tiled {
                    physical(clip).intersection(physical(placement.rect))
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
                let theme = &self.runtime.config.theme;
                let b = theme.border_width;
                if b > 0 {
                    let rect = Rect::new(
                        placement.rect.x - b,
                        placement.rect.y - b,
                        placement.rect.width + 2 * b,
                        placement.rect.height + 2 * b,
                    );
                    if let Some(rect) = logical(rect).intersection(logical(clip)) {
                        let color = if placement.focused {
                            theme.active_border
                        } else {
                            theme.inactive_border
                        };
                        let buffer = SolidColorBuffer::new(rect.size, premultiply(color));
                        elements.push(SceneElement::Border(SolidColorRenderElement::from_buffer(
                            &buffer,
                            (rect.loc.x, rect.loc.y),
                            1.0,
                            1.0,
                            Kind::Unspecified,
                        )));
                    }
                }
            }
        }
        self.layer_elements(renderer, &[Layer::Bottom, Layer::Background], &mut elements);
        elements
    }

    fn layer_elements(
        &self,
        renderer: &mut GlesRenderer,
        kinds: &[Layer],
        elements: &mut Vec<SceneElement>,
    ) {
        for kind in kinds {
            for region in &self.outputs {
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
                    // Layer menus are separate XDG popup trees, not subsurfaces.
                    // Smithay includes them in the same order as surface_under.
                    let surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                        layer.render_elements(renderer, location, 1.0.into(), 1.0);
                    elements.extend(
                        surfaces
                            .into_iter()
                            .filter_map(|e| {
                                CropRenderElement::from_element(e, 1.0, physical(region.rect))
                            })
                            .map(SceneElement::Surface),
                    );
                }
            }
        }
    }

    pub fn frame_callbacks(&self) {
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
                entry.window.send_frame(
                    &region.output,
                    self.start.elapsed(),
                    Some(Duration::ZERO),
                    |_, _| Some(region.output.clone()),
                );
            }
        }
        for region in &self.outputs {
            for layer in layer_map_for_output(&region.output).layers() {
                layer.send_frame(
                    &region.output,
                    self.start.elapsed(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Command, WorkspaceId};

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
