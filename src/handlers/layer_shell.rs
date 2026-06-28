use smithay::{
    desktop::{LayerSurface, WindowSurfaceType, layer_map_for_output},
    output::Output,
    reexports::wayland_server::protocol::{wl_output, wl_surface::WlSurface},
    wayland::{
        compositor::with_states,
        shell::wlr_layer::{
            Layer, LayerSurface as WlrLayerSurface, LayerSurfaceCachedState, LayerSurfaceData,
            WlrLayerShellHandler, WlrLayerShellState,
        },
    },
};

use crate::Clear;

impl WlrLayerShellHandler for Clear {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<wl_output::WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let output = wl_output
            .as_ref()
            .and_then(Output::from_resource)
            .unwrap_or_else(|| self.space.outputs().next().unwrap().clone());

        if namespace == self.config.apps.status_bar.namespace() {
            self.force_status_bar_layer(surface.wl_surface());
        }

        let layer_surface = LayerSurface::new(surface.clone(), namespace.clone());
        let mut map = layer_map_for_output(&output);
        map.map_layer(&layer_surface).unwrap();

        if namespace == self.config.apps.status_bar.namespace() {
            self.active_status_bar_surface = Some(surface);
        }
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        if let Some((mut map, layer)) = self.space.outputs().find_map(|output| {
            let map = layer_map_for_output(output);
            let layer = map
                .layers()
                .find(|layer| layer.layer_surface() == &surface)
                .cloned();
            layer.map(|layer| (map, layer))
        }) {
            map.unmap_layer(&layer);
        }

        if self
            .active_status_bar_surface
            .as_ref()
            .is_some_and(|active| active == &surface)
            || self.status_bar_restart_pending
        {
            self.active_status_bar_surface = None;
        }

        if self.status_bar_restart_pending {
            self.status_bar_restart_pending = false;
            self.spawn_status_bar();
        }
    }
}

impl Clear {
    /// Send the initial configure for layer surfaces once their first commit arrives.
    pub(crate) fn handle_layer_shell_commit(&mut self, surface: &WlSurface) {
        let Some(output) = self.space.outputs().find(|output| {
            let map = layer_map_for_output(output);
            map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .is_some()
        }) else {
            return;
        };

        if self
            .active_status_bar_surface
            .as_ref()
            .is_some_and(|active| active.wl_surface() == surface)
        {
            self.force_status_bar_layer(surface);
        }

        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<LayerSurfaceData>()
                .map(|data| data.lock().unwrap().initial_configure_sent)
                .unwrap_or(false)
        });

        let mut map = layer_map_for_output(output);
        map.arrange();

        if !initial_configure_sent {
            if let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) {
                layer.layer_surface().send_configure();
            }
        }
    }

    fn force_status_bar_layer(&self, surface: &WlSurface) {
        let layer = self.config.apps.status_bar.layer.into();
        with_states(surface, |states| {
            let mut cached_state = states.cached_state.get::<LayerSurfaceCachedState>();
            cached_state.pending().layer = layer;
            cached_state.current().layer = layer;
        });
    }
}
