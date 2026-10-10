//! Bounded advisory minimize targets. All policy and geometry stay backend-independent.

use super::server::{ClientId, PeerCredentials};
use crate::core::{Desktop, OutputId, Rect, WindowId, WindowRole};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, time::Duration};

/// Maximum icon records in one panel's atomic replacement.
pub const MAX_PANEL_TARGETS: usize = 128;
/// Maximum icon records across all providers.
pub const MAX_TOTAL_TARGETS: usize = 512;
/// Maximum IPC connections holding panel registrations.
pub const MAX_PROVIDERS: usize = 8;
/// Monotonic lease renewed only by an accepted complete replacement.
pub const TARGET_LEASE: Duration = Duration::from_secs(5);
const MAX_EXTENT: i32 = 16_384;
const MAX_APP_BYTES: usize = 256;

/// Strict panel-local integer logical geometry; no output coordinates are accepted.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Exactly one selector, preventing an app hint from overriding an exact-window hint.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum AnimationTarget {
    Window { window: String, rect: TargetRect },
    App { app_id: String, rect: TargetRect },
}

/// Same-process mapped panel discovery; the opaque ID changes with mapping or geometry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PanelSnapshot {
    pub panel: String,
    pub output: String,
    pub namespace: String,
    pub width: i32,
    pub height: i32,
}

/// Adapter-validated live panel body and credentials. Not supplied by IPC clients.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelDescriptor {
    pub id: String,
    pub output: OutputId,
    pub namespace: String,
    /// Committed surface origin in global logical coordinates.
    pub origin: (i32, i32),
    /// Visible committed panel body, clipped to full output bounds.
    pub body: Rect,
    pub size: (i32, i32),
    pub peer: PeerCredentials,
}
impl PanelDescriptor {
    pub fn snapshot(&self) -> PanelSnapshot {
        PanelSnapshot {
            panel: self.id.clone(),
            output: self.output.0.to_string(),
            namespace: self.namespace.clone(),
            width: self.size.0,
            height: self.size.1,
        }
    }
}

#[derive(Clone, Debug)]
enum Selector {
    Window(WindowId),
    App(String),
}
#[derive(Clone, Debug)]
struct Target {
    selector: Selector,
    rect: Rect,
}
#[derive(Debug)]
struct Registration {
    owner: ClientId,
    panel: PanelDescriptor,
    expires: Duration,
    targets: Vec<Target>,
}

/// Connection-owned registrations; expiry and external lifecycle pruning are explicit.
#[derive(Debug, Default)]
pub struct TargetStore {
    records: Vec<Registration>,
}
impl TargetStore {
    /// Replace one live panel's records atomically after all ownership/bounds/ID checks.
    pub fn replace(
        &mut self,
        owner: ClientId,
        peer: PeerCredentials,
        panel: &PanelDescriptor,
        output: &str,
        targets: &[AnimationTarget],
        desktop: &Desktop,
        now: Duration,
    ) -> Result<(), String> {
        if peer != panel.peer {
            return Err("panel belongs to another Wayland process".into());
        }
        if output != panel.output.0.to_string() || desktop.output(panel.output).is_none() {
            return Err("panel output is stale".into());
        }
        if targets.len() > MAX_PANEL_TARGETS {
            return Err("panel target limit exceeded".into());
        }
        if self
            .records
            .iter()
            .any(|r| r.panel.id == panel.id && r.owner != owner && r.expires > now)
        {
            return Err("panel is registered by another IPC connection".into());
        }
        let previous = self.records.iter().find(|r| r.panel.id == panel.id);
        let total: usize = self
            .records
            .iter()
            .filter(|r| r.expires > now)
            .map(|r| r.targets.len())
            .sum();
        let removed = previous
            .filter(|r| r.expires > now)
            .map_or(0, |r| r.targets.len());
        if total - removed + targets.len() > MAX_TOTAL_TARGETS {
            return Err("global target limit exceeded".into());
        }
        let providers: BTreeSet<_> = self
            .records
            .iter()
            .filter(|r| r.expires > now)
            .map(|r| r.owner.0)
            .collect();
        if !targets.is_empty() && !providers.contains(&owner.0) && providers.len() == MAX_PROVIDERS
        {
            return Err("animation target provider limit exceeded".into());
        }
        let mut seen = BTreeSet::new();
        let mut validated = Vec::with_capacity(targets.len());
        for target in targets {
            let (key, selector, local) = match target {
                AnimationTarget::Window { window, rect } => {
                    let window = desktop
                        .windows()
                        .find(|w| {
                            w.id.0.to_string() == *window
                                && w.role == WindowRole::Normal
                                && w.output == Some(panel.output)
                        })
                        .ok_or("target is not a live normal window on this output")?;
                    (
                        format!("window:{}", window.id.0),
                        Selector::Window(window.id),
                        rect,
                    )
                }
                AnimationTarget::App { app_id, rect } => {
                    if app_id.is_empty()
                        || app_id.len() > MAX_APP_BYTES
                        || app_id.contains('\0')
                        || !desktop.windows().any(|w| {
                            w.role == WindowRole::Normal
                                && w.app_id == *app_id
                                && w.output == Some(panel.output)
                        })
                    {
                        return Err("app target has no live normal window on this output".into());
                    }
                    (format!("app:{app_id}"), Selector::App(app_id.clone()), rect)
                }
            };
            if !seen.insert(key) {
                return Err("duplicate target selector".into());
            }
            if local.x < 0
                || local.y < 0
                || local.width <= 0
                || local.height <= 0
                || local.width > MAX_EXTENT
                || local.height > MAX_EXTENT
                || i64::from(local.x) + i64::from(local.width) > i64::from(panel.size.0)
                || i64::from(local.y) + i64::from(local.height) > i64::from(panel.size.1)
            {
                return Err("target rectangle is outside the committed panel".into());
            }
            let x = panel
                .origin
                .0
                .checked_add(local.x)
                .ok_or("target coordinate overflow")?;
            let y = panel
                .origin
                .1
                .checked_add(local.y)
                .ok_or("target coordinate overflow")?;
            let rect = Rect::new(x, y, local.width, local.height);
            if rect.width != local.width
                || rect.height != local.height
                || panel.body.intersection(rect) != Some(rect)
            {
                return Err("target rectangle is outside the visible panel body".into());
            }
            validated.push(Target { selector, rect });
        }
        self.records
            .retain(|r| r.panel.id != panel.id && r.expires > now);
        if !validated.is_empty() {
            self.records.push(Registration {
                owner,
                panel: panel.clone(),
                expires: now.saturating_add(TARGET_LEASE),
                targets: validated,
            });
            self.records
                .sort_by_key(|record| record.panel.id.parse::<u64>().unwrap_or(u64::MAX));
        }
        Ok(())
    }

    /// Drop disconnected, expired, moved/resized, unmapped, or output-lost registrations.
    pub fn retain_live(
        &mut self,
        panels: &[PanelDescriptor],
        connected: impl Fn(ClientId) -> bool,
        desktop: &Desktop,
        now: Duration,
    ) {
        self.records.retain_mut(|record| {
            if record.expires <= now || !connected(record.owner) || !panels.contains(&record.panel)
            {
                return false;
            }
            record.targets.retain(|target| match &target.selector {
                Selector::Window(id) => desktop.window(*id).is_some_and(|w| {
                    w.role == WindowRole::Normal && w.output == Some(record.panel.output)
                }),
                Selector::App(app) => desktop.windows().any(|w| {
                    w.role == WindowRole::Normal
                        && w.app_id == *app
                        && w.output == Some(record.panel.output)
                }),
            });
            !record.targets.is_empty()
        });
    }

    /// Resolve exact-window before exact-app hints on the source home output.
    pub fn resolve(&self, id: WindowId, desktop: &Desktop, now: Duration) -> Option<Rect> {
        let window = desktop.window(id)?;
        if window.role != WindowRole::Normal {
            return None;
        }
        let output = window.output?;
        for exact in [true, false] {
            for record in self
                .records
                .iter()
                .filter(|r| r.expires > now && r.panel.output == output)
            {
                for target in &record.targets {
                    let matches = match &target.selector {
                        Selector::Window(target) => exact && *target == id,
                        Selector::App(app) => !exact && *app == window.app_id,
                    };
                    if matches {
                        return Some(target.rect);
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Command;

    fn setup() -> (Desktop, PanelDescriptor, PeerCredentials) {
        let mut desktop = Desktop::new();
        desktop.add_output(OutputId(1), "left".into(), Rect::new(-800, 0, 800, 600));
        desktop.add_window(WindowId(1), "one".into(), "editor".into());
        let peer = PeerCredentials { pid: 1, uid: 1000 };
        let panel = PanelDescriptor {
            id: "1".into(),
            output: OutputId(1),
            namespace: "panel".into(),
            origin: (-800, 0),
            body: Rect::new(-800, 0, 800, 32),
            size: (800, 32),
            peer,
        };
        (desktop, panel, peer)
    }
    fn app(x: i32) -> AnimationTarget {
        AnimationTarget::App {
            app_id: "editor".into(),
            rect: TargetRect {
                x,
                y: 6,
                width: 20,
                height: 20,
            },
        }
    }
    fn exact(x: i32) -> AnimationTarget {
        AnimationTarget::Window {
            window: "1".into(),
            rect: TargetRect {
                x,
                y: 6,
                width: 20,
                height: 20,
            },
        }
    }
    #[test]
    fn owned_local_rectangles_translate_and_exact_window_precedes_app() {
        let (d, panel, peer) = setup();
        let mut store = TargetStore::default();
        store
            .replace(
                ClientId(1),
                peer,
                &panel,
                "1",
                &[app(10), exact(50)],
                &d,
                Duration::ZERO,
            )
            .unwrap();
        assert_eq!(
            store.resolve(WindowId(1), &d, Duration::ZERO),
            Some(Rect::new(-750, 6, 20, 20))
        );
        assert_eq!(store.resolve(WindowId(1), &d, TARGET_LEASE), None);
        assert_eq!(d.focused_window(), Some(WindowId(1)));
    }
    #[test]
    fn invalid_batch_never_replaces_valid_targets_and_other_process_cannot_claim_panel() {
        let (d, panel, peer) = setup();
        let mut store = TargetStore::default();
        store
            .replace(
                ClientId(1),
                peer,
                &panel,
                "1",
                &[app(10)],
                &d,
                Duration::ZERO,
            )
            .unwrap();
        let before = store.resolve(WindowId(1), &d, Duration::ZERO);
        let wrong = PeerCredentials { pid: 2, ..peer };
        for (owner, credentials, output, targets) in [
            (ClientId(2), wrong, "1", vec![app(50)]),
            (ClientId(2), peer, "1", vec![app(50)]),
            (ClientId(1), peer, "2", vec![app(50)]),
            (ClientId(1), peer, "1", vec![app(50), exact(790)]),
            (ClientId(1), peer, "1", vec![app(50), app(60)]),
            (ClientId(1), peer, "1", vec![exact(-1)]),
            (ClientId(1), peer, "1", vec![exact(i32::MAX)]),
        ] {
            assert!(
                store
                    .replace(
                        owner,
                        credentials,
                        &panel,
                        output,
                        &targets,
                        &d,
                        Duration::ZERO
                    )
                    .is_err()
            );
            assert_eq!(store.resolve(WindowId(1), &d, Duration::ZERO), before);
        }
    }
    #[test]
    fn lifecycle_disconnect_geometry_generation_window_role_and_unmap_invalidate() {
        let (mut d, panel, peer) = setup();
        for cause in 0..6 {
            let mut store = TargetStore::default();
            store
                .replace(
                    ClientId(1),
                    peer,
                    &panel,
                    "1",
                    &[app(10)],
                    &d,
                    Duration::ZERO,
                )
                .unwrap();
            let mut live = vec![panel.clone()];
            match cause {
                0 => live.clear(),
                1 => live[0].id = "2".into(),
                2 => live[0].origin.0 += 1,
                4 => d.set_window_role(WindowId(1), WindowRole::Launcher),
                5 => d.remove_window(WindowId(1)),
                _ => {}
            }
            store.retain_live(&live, |_| cause != 3, &d, Duration::ZERO);
            assert_eq!(store.resolve(WindowId(1), &d, Duration::ZERO), None);
            d.remove_window(WindowId(1));
            d.add_window(WindowId(1), "one".into(), "editor".into());
        }
    }
    #[test]
    fn lease_renewal_and_empty_replace_release_registration() {
        let (d, panel, peer) = setup();
        let mut store = TargetStore::default();
        store
            .replace(
                ClientId(1),
                peer,
                &panel,
                "1",
                &[app(10)],
                &d,
                Duration::ZERO,
            )
            .unwrap();
        store
            .replace(
                ClientId(1),
                peer,
                &panel,
                "1",
                &[app(30)],
                &d,
                Duration::from_secs(4),
            )
            .unwrap();
        assert!(
            store
                .resolve(WindowId(1), &d, Duration::from_secs(8))
                .is_some()
        );
        assert!(
            store
                .resolve(WindowId(1), &d, Duration::from_secs(9))
                .is_none()
        );
        store
            .replace(
                ClientId(1),
                peer,
                &panel,
                "1",
                &[],
                &d,
                Duration::from_secs(4),
            )
            .unwrap();
        assert!(store.records.is_empty());
        store
            .replace(
                ClientId(2),
                peer,
                &panel,
                "1",
                &[app(10)],
                &d,
                Duration::from_secs(4),
            )
            .unwrap();
    }
    #[test]
    fn clipped_empty_foreign_and_launcher_targets_are_rejected() {
        let (mut d, mut panel, peer) = setup();
        let mut store = TargetStore::default();
        panel.body = Rect::new(-780, 0, 780, 32);
        assert!(
            store
                .replace(
                    ClientId(1),
                    peer,
                    &panel,
                    "1",
                    &[app(10)],
                    &d,
                    Duration::ZERO
                )
                .is_err()
        );
        let zero = AnimationTarget::App {
            app_id: "editor".into(),
            rect: TargetRect {
                x: 30,
                y: 0,
                width: 0,
                height: 20,
            },
        };
        assert!(
            store
                .replace(ClientId(1), peer, &panel, "1", &[zero], &d, Duration::ZERO)
                .is_err()
        );
        d.set_window_role(WindowId(1), WindowRole::Launcher);
        assert!(
            store
                .replace(
                    ClientId(1),
                    peer,
                    &panel,
                    "1",
                    &[app(30)],
                    &d,
                    Duration::ZERO
                )
                .is_err()
        );
        assert!(
            store
                .replace(
                    ClientId(1),
                    peer,
                    &panel,
                    "1",
                    &[exact(30)],
                    &d,
                    Duration::ZERO
                )
                .is_err()
        );
    }
    #[test]
    fn provider_global_and_per_panel_limits_are_enforced_atomically() {
        let (mut d, mut panel, peer) = setup();
        for id in 2..=129 {
            d.add_window(WindowId(id), "test".into(), format!("app-{id}"));
        }
        let targets: Vec<_> = (1..=129)
            .map(|id| AnimationTarget::Window {
                window: id.to_string(),
                rect: TargetRect {
                    x: 30,
                    y: 0,
                    width: 20,
                    height: 20,
                },
            })
            .collect();
        let mut store = TargetStore::default();
        assert!(
            store
                .replace(ClientId(1), peer, &panel, "1", &targets, &d, Duration::ZERO)
                .is_err()
        );
        for id in 1..=4 {
            panel.id = id.to_string();
            store
                .replace(
                    ClientId(id),
                    peer,
                    &panel,
                    "1",
                    &targets[..128],
                    &d,
                    Duration::ZERO,
                )
                .unwrap();
        }
        panel.id = "5".into();
        assert!(
            store
                .replace(
                    ClientId(5),
                    peer,
                    &panel,
                    "1",
                    &[app(30)],
                    &d,
                    Duration::ZERO
                )
                .is_err()
        );
        let mut store = TargetStore::default();
        for id in 1..=8 {
            panel.id = id.to_string();
            store
                .replace(
                    ClientId(id),
                    peer,
                    &panel,
                    "1",
                    &[app(30)],
                    &d,
                    Duration::ZERO,
                )
                .unwrap();
        }
        panel.id = "9".into();
        assert!(
            store
                .replace(
                    ClientId(9),
                    peer,
                    &panel,
                    "1",
                    &[app(30)],
                    &d,
                    Duration::ZERO
                )
                .is_err()
        );
        d.command(Command::SetMinimized(WindowId(1), true));
        assert!(store.resolve(WindowId(1), &d, Duration::ZERO).is_some());
    }
}
