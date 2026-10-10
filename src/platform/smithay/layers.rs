//! Layer-shell lifecycle, reserved areas, and keyboard ownership.

use super::state::Compositor;
use crate::config::{PanelLayer, ShellConfig};
use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    desktop::{LayerSurface, layer_map_for_output},
    output::Output,
    reexports::wayland_server::protocol::{wl_output, wl_surface::WlSurface},
    wayland::{
        compositor::with_states,
        shell::wlr_layer::{
            KeyboardInteractivity, Layer, LayerSurface as ProtocolLayer, LayerSurfaceCachedState,
            LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState,
        },
    },
};

pub(super) struct LayerEntry {
    pub surface: LayerSurface,
    pub output: Output,
    pub mapped: bool,
    requested_layer: Layer,
}

fn policy_layer(config: &ShellConfig, namespace: &str, requested: Layer) -> Layer {
    config
        .panels
        .iter()
        .find(|rule| rule.namespace == namespace)
        .map_or(requested, |rule| match rule.layer {
            PanelLayer::Background => Layer::Background,
            PanelLayer::Bottom => Layer::Bottom,
            PanelLayer::Top => Layer::Top,
            PanelLayer::Overlay => Layer::Overlay,
        })
}

fn apply_policy(entry: &LayerEntry, config: &ShellConfig) {
    let layer = policy_layer(config, entry.surface.namespace(), entry.requested_layer);
    // Smithay's arrangement/rendering reads current state. Never rewrite pending
    // client state: set_layer is double-buffered and may precede a later commit.
    // Each commit restores the raw client state before we apply policy again.
    with_states(entry.surface.wl_surface(), |states| {
        let mut cached = states.cached_state.get::<LayerSurfaceCachedState>();
        cached.current().layer = layer;
    });
}

impl WlrLayerShellHandler for Compositor {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: ProtocolLayer,
        output: Option<wl_output::WlOutput>,
        layer: Layer,
        namespace: String,
    ) {
        let output = output.as_ref().and_then(Output::from_resource).or_else(|| {
            self.outputs
                .iter()
                .find(|o| Some(o.id) == self.runtime.desktop.focused_output())
                .map(|o| o.output.clone())
        });
        let Some(output) = output else {
            surface.send_close();
            return;
        };
        self.layers.push(LayerEntry {
            surface: LayerSurface::new(surface, namespace),
            output,
            mapped: false,
            requested_layer: layer,
        });
    }

    fn layer_destroyed(&mut self, surface: ProtocolLayer) {
        if let Some(index) = self
            .layers
            .iter()
            .position(|entry| entry.surface.layer_surface() == &surface)
        {
            self.invalidate_animation_panel(surface.wl_surface());
            let entry = self.layers.remove(index);
            layer_map_for_output(&entry.output).unmap_layer(&entry.surface);
        }
        self.dirty = true;
    }
}

impl Compositor {
    pub fn layer_commit(&mut self, surface: &WlSurface) {
        let has_buffer =
            with_renderer_surface_state(surface, |state| state.buffer().is_some()).unwrap_or(false);
        if !has_buffer {
            self.invalidate_animation_panel(surface);
        }
        let Some(entry) = self
            .layers
            .iter_mut()
            .find(|entry| entry.surface.wl_surface() == surface)
        else {
            return;
        };

        let configured = with_states(surface, |states| {
            states
                .data_map
                .get::<LayerSurfaceData>()
                .is_some_and(|data| data.lock().unwrap().initial_configure_sent)
        });
        let mut map = layer_map_for_output(&entry.output);
        if entry.mapped && !has_buffer {
            // Smithay resets the role before this callback. Do not consume the
            // *next* mapping's initial configure on this null-buffer commit.
            map.unmap_layer(&entry.surface);
            entry.mapped = false;
            self.dirty = true;
            return;
        }
        entry.requested_layer = entry.surface.cached_state().layer;
        apply_policy(entry, &self.runtime.config.shell);
        // Initial configuration needs arrangement before a buffer exists. This
        // temporary mapping must not reserve space or take keyboard focus.
        if has_buffer || !configured {
            if let Err(error) = map.map_layer(&entry.surface) {
                eprintln!("clear: layer mapping failed: {error}");
                return;
            }
            map.arrange();
            if !configured {
                entry.surface.layer_surface().send_configure();
            }
        }
        if !has_buffer {
            map.unmap_layer(&entry.surface);
        } else {
            map.arrange();
        }
        entry.mapped = has_buffer;
        self.dirty = true;
    }

    /// Re-apply namespace policy after config reload, without restarting clients.
    pub fn refresh_layers(&mut self) {
        for entry in &mut self.layers {
            apply_policy(entry, &self.runtime.config.shell);
        }
        if self.layer_focus.as_ref().is_some_and(|surface| {
            !self.layers.iter().any(|entry| {
                entry.mapped
                    && entry.surface.wl_surface() == surface
                    && entry.surface.can_receive_keyboard_focus()
            })
        }) {
            self.layer_focus = None;
        }
    }

    /// Exclusive top/overlay clients block overview entry; on-demand focus does not.
    pub fn has_exclusive_layer(&self) -> bool {
        self.outputs.iter().any(|region| {
            let map = layer_map_for_output(&region.output);
            [Layer::Overlay, Layer::Top].iter().any(|kind| {
                map.layers_on(*kind).any(|surface| {
                    surface.cached_state().keyboard_interactivity
                        == KeyboardInteractivity::Exclusive
                })
            })
        })
    }

    /// Top/overlay exclusive surfaces preempt application focus until unmapped.
    pub fn layer_keyboard_focus(&self) -> Option<WlSurface> {
        for kind in [Layer::Overlay, Layer::Top] {
            for region in &self.outputs {
                let map = layer_map_for_output(&region.output);
                // Mapping order, not object creation order, determines stacking
                // after a client unmaps and subsequently reuses its layer role.
                if let Some(surface) = map.layers_on(kind).rev().find(|surface| {
                    surface.cached_state().keyboard_interactivity
                        == KeyboardInteractivity::Exclusive
                }) {
                    return Some(surface.wl_surface().clone());
                }
            }
        }
        self.layer_focus.clone()
    }

    pub fn focus_layer(&mut self, surface: &WlSurface) {
        if self.layers.iter().any(|entry| {
            entry.mapped
                && entry.surface.wl_surface() == surface
                && entry.surface.can_receive_keyboard_focus()
        }) {
            self.layer_focus = Some(surface.clone());
        }
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn panel_layer_override_does_not_affect_unmatched_clients() {
        let config = ShellConfig::default();
        assert_eq!(policy_layer(&config, "waybar", Layer::Bottom), Layer::Top);
        assert_eq!(
            policy_layer(&config, "launcher", Layer::Overlay),
            Layer::Overlay
        );
        assert_eq!(
            policy_layer(&config, "wallpaper", Layer::Background),
            Layer::Background
        );
        let without_override = ShellConfig {
            panels: Vec::new(),
            ..config
        };
        assert_eq!(
            policy_layer(&without_override, "waybar", Layer::Bottom),
            Layer::Bottom
        );
    }
}
