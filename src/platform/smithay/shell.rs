//! Optional shell transport integration; policy and wire types remain backend-neutral.

use super::state::Compositor;
use crate::shell::{self, RequestKind, server::Server};
use std::path::PathBuf;

impl Compositor {
    pub fn init_shell(&mut self, enabled: bool) {
        if !enabled {
            return;
        }
        let server = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| std::io::Error::other("XDG_RUNTIME_DIR is unset"))
            .and_then(|directory| Server::bind(&directory, &self.socket_name.to_string_lossy()));
        match server {
            Ok(server) => {
                eprintln!("clear: shell socket {}", server.path().display());
                self.prune_animation_connections(&server);
                self.shell_server = Some(server);
            }
            Err(error) => {
                // An optional shell must never prevent the window manager starting.
                eprintln!("clear: shell IPC unavailable: {error}; continuing without it");
            }
        }
    }

    pub fn dispatch_shell(&mut self) {
        self.refresh_animation_targets();
        let Some(mut server) = self.shell_server.take() else {
            return;
        };
        let requests = server.poll();
        self.prune_animation_connections(&server);
        for (client, bytes) in requests {
            let request = match shell::parse_request(&bytes) {
                Ok(request) => request,
                Err(error) => {
                    server.reply(client, shell::encode_error(None, &error));
                    continue;
                }
            };
            match &request.request {
                RequestKind::AnimationPanels => {
                    let Some(peer) = server.peer_credentials(client) else {
                        server.reply(
                            client,
                            shell::encode_error(Some(request.id), "peer credentials unavailable"),
                        );
                        continue;
                    };
                    let panels: Vec<_> = self
                        .animation_targets
                        .panels
                        .iter()
                        .filter(|panel| panel.descriptor.peer == peer)
                        .map(|panel| panel.descriptor.snapshot())
                        .collect();
                    server.reply(client, shell::encode_animation_panels(request.id, &panels));
                    continue;
                }
                RequestKind::SetAnimationTargets {
                    panel,
                    output,
                    targets,
                } => {
                    let result = server
                        .peer_credentials(client)
                        .ok_or_else(|| "peer credentials unavailable".to_owned())
                        .and_then(|peer| {
                            let panel = self
                                .animation_targets
                                .panels
                                .iter()
                                .find(|candidate| candidate.descriptor.id == *panel)
                                .ok_or_else(|| "unknown or stale panel mapping".to_owned())?;
                            self.animation_targets.store.replace(
                                client,
                                peer,
                                &panel.descriptor,
                                output,
                                targets,
                                &self.runtime.desktop,
                                self.start.elapsed(),
                            )
                        });
                    server.reply(
                        client,
                        match result {
                            Ok(()) => shell::encode_ok(request.id),
                            Err(error) => shell::encode_error(Some(request.id), &error),
                        },
                    );
                    continue;
                }
                _ => {}
            }
            if let Err(error) = shell::execute(&mut self.runtime, &request) {
                server.reply(client, shell::encode_error(Some(request.id), &error));
                continue;
            }
            self.service_overview();
            match request.request {
                RequestKind::Snapshot | RequestKind::Subscribe => {
                    if matches!(request.request, RequestKind::Subscribe) {
                        server.subscribe(client);
                    }
                    server.reply(
                        client,
                        shell::encode_snapshot(request.id, &shell::snapshot(&self.runtime)),
                    );
                }
                _ => {
                    // Reconcile before acknowledging so a subsequent snapshot
                    // includes settled geometry/reservations and policy focus.
                    self.dirty = true;
                    self.reconcile();
                    server.reply(client, shell::encode_ok(request.id));
                }
            }
        }
        if server.has_subscribers() && (self.shell_dirty || self.shell_snapshot.is_none()) {
            let snapshot = shell::snapshot(&self.runtime);
            if self.shell_snapshot.as_ref() != Some(&snapshot) {
                server.publish(shell::encode_update(&snapshot));
                self.shell_snapshot = Some(snapshot);
            }
            self.shell_dirty = false;
        } else if !server.has_subscribers() {
            self.shell_snapshot = None;
        }
        self.prune_animation_connections(&server);
        self.shell_server = Some(server);
    }
}

use crate::shell::server::PeerCredentials;
use crate::{
    core::{Rect, WindowId},
    shell::animation_targets::{PanelDescriptor, TargetStore},
};
use smithay::{
    desktop::layer_map_for_output,
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    wayland::shell::wlr_layer::{Anchor, ExclusiveZone, Layer},
};

const MAX_DISCOVERED_PANELS: usize = 32;

struct LivePanel {
    surface: WlSurface,
    descriptor: PanelDescriptor,
    priority: u8,
}

/// Adapter-only mapping identities plus bounded backend-independent icon registrations.
#[derive(Default)]
pub(super) struct AnimationTargets {
    panels: Vec<LivePanel>,
    store: TargetStore,
    next_id: u64,
}

impl Compositor {
    fn discover_animation_panels(&self) -> Vec<LivePanel> {
        let mut panels = Vec::new();
        for entry in &self.layers {
            if panels.len() == MAX_DISCOVERED_PANELS {
                break;
            }
            if !entry.mapped || !matches!(entry.surface.layer(), Layer::Top | Layer::Bottom) {
                continue;
            }
            let Some(output) = self
                .outputs
                .iter()
                .find(|output| output.output == entry.output)
            else {
                continue;
            };
            let configured = self
                .runtime
                .config
                .shell
                .panels
                .iter()
                .any(|rule| rule.namespace == entry.surface.namespace());
            let state = entry.surface.cached_state();
            let edge = state.anchor.contains(Anchor::TOP) != state.anchor.contains(Anchor::BOTTOM)
                || state.anchor.contains(Anchor::LEFT) != state.anchor.contains(Anchor::RIGHT);
            let reserved =
                matches!(state.exclusive_zone, ExclusiveZone::Exclusive(zone) if zone > 0);
            if !configured && !(edge && reserved) {
                continue;
            }
            let geometry = layer_map_for_output(&entry.output).layer_geometry(&entry.surface);
            let Some(geometry) = geometry else {
                continue;
            };
            let arranged = Rect::new(
                geometry.loc.x,
                geometry.loc.y,
                geometry.size.w,
                geometry.size.h,
            );
            if entry.surface.namespace().len() > 256 {
                continue;
            }
            let offset = entry.surface.geometry().loc;
            let Some((origin, body)) = panel_geometry(output.rect, arranged, (offset.x, offset.y))
            else {
                continue;
            };
            let surface = entry.surface.wl_surface();
            let Some(peer) = surface
                .client()
                .and_then(|client| client.get_credentials(&self.display_handle).ok())
                .and_then(|peer| {
                    Some(PeerCredentials {
                        pid: peer.pid.try_into().ok()?,
                        uid: peer.uid,
                    })
                })
            else {
                continue;
            };
            panels.push(LivePanel {
                surface: surface.clone(),
                priority: if reserved { 0 } else { 1 },
                descriptor: PanelDescriptor {
                    id: String::new(),
                    output: output.id,
                    namespace: entry.surface.namespace().to_owned(),
                    origin,
                    body,
                    size: (geometry.size.w, geometry.size.h),
                    peer,
                },
            });
        }
        panels
    }

    /// Refresh after layer arrangement; geometry/output changes rotate opaque identities.
    pub fn refresh_animation_targets(&mut self) {
        let mut current = self.discover_animation_panels();
        for panel in &mut current {
            let previous = self
                .animation_targets
                .panels
                .iter()
                .find(|old| old.surface == panel.surface);
            if let Some(previous) = previous {
                panel.descriptor.id = previous.descriptor.id.clone();
                if panel.descriptor == previous.descriptor {
                    continue;
                }
            }
            panel.descriptor.id.clear();
            let Some(id) = self.animation_targets.next_id.checked_add(1) else {
                continue;
            };
            self.animation_targets.next_id = id;
            panel.descriptor.id = id.to_string();
        }
        current.retain(|panel| !panel.descriptor.id.is_empty());
        self.animation_targets.panels = current;
        let descriptors: Vec<_> = self
            .animation_targets
            .panels
            .iter()
            .map(|panel| panel.descriptor.clone())
            .collect();
        self.animation_targets.store.retain_live(
            &descriptors,
            |client| {
                self.shell_server
                    .as_ref()
                    .is_none_or(|server| server.is_connected(client))
            },
            &self.runtime.desktop,
            self.start.elapsed(),
        );
    }

    /// Forget a mapping before unmap/destruction, even if it remaps in the same loop turn.
    pub fn invalidate_animation_panel(&mut self, surface: &WlSurface) {
        self.animation_targets
            .panels
            .retain(|panel| panel.surface != *surface);
        let descriptors: Vec<_> = self
            .animation_targets
            .panels
            .iter()
            .map(|panel| panel.descriptor.clone())
            .collect();
        self.animation_targets.store.retain_live(
            &descriptors,
            |_| true,
            &self.runtime.desktop,
            self.start.elapsed(),
        );
    }

    fn prune_animation_connections(&mut self, server: &Server) {
        let descriptors: Vec<_> = self
            .animation_targets
            .panels
            .iter()
            .map(|panel| panel.descriptor.clone())
            .collect();
        self.animation_targets.store.retain_live(
            &descriptors,
            |client| server.is_connected(client),
            &self.runtime.desktop,
            self.start.elapsed(),
        );
    }

    /// Resolve a live exact-window or matching-app target, in global logical coordinates.
    pub fn minimize_target(&self, window: WindowId) -> Option<Rect> {
        self.animation_targets
            .store
            .resolve(window, &self.runtime.desktop, self.start.elapsed())
    }

    /// Stable eligible panel center, else full-output bottom center; never usable-area guesses.
    pub fn minimize_fallback(&self, window: WindowId) -> Option<Rect> {
        let window = self.runtime.desktop.window(window)?;
        let output = self.runtime.desktop.output(window.output?)?;
        let panels = self
            .animation_targets
            .panels
            .iter()
            .filter(|panel| panel.descriptor.output == output.id)
            .map(|panel| {
                (
                    panel.priority,
                    panel.descriptor.id.parse::<u64>().unwrap_or(u64::MAX),
                    panel.descriptor.body,
                )
            });
        fallback_rect(output.bounds, panels)
    }
}

fn panel_geometry(
    output: Rect,
    arranged: Rect,
    body_local_origin: (i32, i32),
) -> Option<((i32, i32), Rect)> {
    // Discovery exposes dimensions with an implicit local (0, 0).
    // A shifted committed root view cannot satisfy that narrow
    // contract; omit it instead of accepting misleading icon geometry.
    if body_local_origin != (0, 0) || arranged.is_empty() {
        return None;
    }
    let origin = (
        output.x.checked_add(arranged.x)?,
        output.y.checked_add(arranged.y)?,
    );
    let rect = Rect::new(origin.0, origin.1, arranged.width, arranged.height);
    let body = rect.intersection(output)?;
    (body != output).then_some((origin, body))
}

fn fallback_rect(bounds: Rect, panels: impl Iterator<Item = (u8, u64, Rect)>) -> Option<Rect> {
    if bounds.is_empty() {
        return None;
    }
    let panel = panels.min_by_key(|(priority, id, _)| (*priority, *id));
    let target = if let Some((_, _, body)) = panel {
        body.centered(24, 24)
    } else {
        let width = bounds.width.min(24);
        let height = bounds.height.min(24);
        Rect::new(
            bounds.x + (bounds.width - width) / 2,
            bounds.bottom() - height,
            width,
            height,
        )
    };
    Some(target.clamped_to(bounds))
}

#[cfg(test)]
mod tests {
    use super::{fallback_rect, panel_geometry};
    use crate::core::Rect;

    #[test]
    fn panel_local_domain_rejects_shifted_bodies_and_whole_output_overlays() {
        let output = Rect::new(-640, 100, 640, 480);
        let panel = Rect::new(-10, 0, 650, 32);
        assert_eq!(
            panel_geometry(output, panel, (0, 0)),
            Some(((-650, 100), Rect::new(-640, 100, 640, 32)))
        );
        // Both positive and negative committed root-view offsets would
        // require an additional body-local origin in the discovery protocol.
        for origin in [(10, 0), (-10, 0), (0, 10), (0, -10)] {
            assert_eq!(panel_geometry(output, panel, origin), None);
        }
        assert_eq!(
            panel_geometry(output, Rect::new(0, 0, 640, 480), (0, 0)),
            None
        );
        assert_eq!(panel_geometry(output, Rect::default(), (0, 0)), None);
    }

    #[test]
    fn fallback_uses_full_nonzero_output_bottom_and_fits_tiny_outputs() {
        assert_eq!(
            fallback_rect(Rect::new(-640, 100, 640, 480), std::iter::empty()),
            Some(Rect::new(-332, 556, 24, 24))
        );
        assert_eq!(
            fallback_rect(Rect::new(400, -100, 10, 8), std::iter::empty()),
            Some(Rect::new(400, -100, 10, 8))
        );
        assert_eq!(fallback_rect(Rect::default(), std::iter::empty()), None);
    }

    #[test]
    fn fallback_prefers_reserved_panel_then_identity_and_fits_short_body() {
        let bounds = Rect::new(640, 0, 640, 480);
        let panels = [
            (1, 1, Rect::new(640, 440, 640, 40)),
            (0, 3, Rect::new(640, 0, 640, 32)),
            (0, 2, Rect::new(680, 0, 560, 10)),
        ];
        assert_eq!(
            fallback_rect(bounds, panels.into_iter()),
            Some(Rect::new(948, 0, 24, 10))
        );
    }
}
