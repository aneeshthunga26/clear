//! Decoration negotiation is separate from the mode of the committed client buffer.

use super::rounded::{RoundedShape, WindowOutline};
use super::{
    state::{Compositor, ManagedWindow},
    titlebar::content_rect,
};
use crate::{core::Rect, decoration::Theme};
use smithay::{
    reexports::{
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::{
            Mode, ZxdgToplevelDecorationV1,
        },
        wayland_protocols_misc::server_decoration::server::org_kde_kwin_server_decoration::{
            Mode as KdeMode, OrgKdeKwinServerDecoration,
        },
        wayland_server::{Resource, WEnum, protocol::wl_surface::WlSurface},
    },
    utils::{SERIAL_COUNTER, Serial},
    wayland::{
        compositor::with_states,
        shell::{
            kde::decoration::{KdeDecorationHandler, KdeDecorationState},
            xdg::{ToplevelSurface, XdgToplevelSurfaceData, decoration::XdgDecorationHandler},
        },
    },
};
use std::sync::Mutex;

#[derive(Clone, Copy)]
struct XdgDecoration {
    mode: Mode,
    // A replacement object must not inherit an old object's committed configure.
    since: Serial,
}

#[derive(Default)]
struct Decorations {
    xdg: Option<XdgDecoration>,
    // Destruction changes negotiation now, but the committed buffer only on root commit.
    destroyed_xdg_ssd: Option<bool>,
    kde: Option<(OrgKdeKwinServerDecoration, KdeMode)>,
    kde_committed_ssd: bool,
}

impl Decorations {
    fn destroy_xdg(&mut self, committed: Option<(Serial, Option<Mode>)>) {
        if let Some(xdg) = self.xdg.take() {
            // Repeated destroy/recreate requests without a commit still display the
            // original buffer, not the replacement object's uncommitted preference.
            self.destroyed_xdg_ssd
                .get_or_insert_with(|| committed_ssd(Some(xdg), committed, false));
        }
    }

    fn render_ssd(&self, committed: Option<(Serial, Option<Mode>)>, kde_ssd: bool) -> bool {
        self.destroyed_xdg_ssd
            .unwrap_or_else(|| committed_ssd(self.xdg, committed, kde_ssd))
    }

    fn commit(&mut self, has_buffer: bool) {
        self.destroyed_xdg_ssd = None;
        self.kde_committed_ssd = has_buffer && self.kde_pending_ssd();
    }

    fn kde_pending_ssd(&self) -> bool {
        self.kde
            .as_ref()
            .is_some_and(|(object, mode)| object.is_alive() && *mode == KdeMode::Server)
    }

    fn pending_ssd(&self) -> bool {
        self.xdg.map_or_else(
            || self.kde_pending_ssd(),
            |xdg| xdg.mode == Mode::ServerSide,
        )
    }
}

fn with_decorations<T>(surface: &WlSurface, f: impl FnOnce(&mut Decorations) -> T) -> T {
    with_states(surface, |states| {
        states
            .data_map
            .insert_if_missing_threadsafe(|| Mutex::new(Decorations::default()));
        f(&mut states
            .data_map
            .get::<Mutex<Decorations>>()
            .unwrap()
            .lock()
            .unwrap())
    })
}

fn committed_ssd(
    xdg: Option<XdgDecoration>,
    committed: Option<(Serial, Option<Mode>)>,
    kde_ssd: bool,
) -> bool {
    match xdg {
        Some(xdg) => committed
            .is_some_and(|(serial, mode)| serial > xdg.since && mode == Some(Mode::ServerSide)),
        None => kde_ssd,
    }
}

fn preferred_mode(mode: Option<Mode>) -> Mode {
    match mode {
        Some(Mode::ClientSide) => Mode::ClientSide,
        _ => Mode::ServerSide,
    }
}

impl ManagedWindow {
    /// Drawing/hit-test mode of the committed buffer, never just an ack or request.
    /// XDG takes precedence over KDE through the first root commit after destruction.
    /// Without either protocol, this is CSD.
    pub fn uses_ssd(&self) -> bool {
        if !self.mapped || self.committed_fullscreen {
            return false;
        }
        let Some(top) = self.window.toplevel() else {
            return false;
        };
        let committed = top.with_cached_state(|state| {
            state
                .last_acked
                .as_ref()
                .map(|configure| (configure.serial, configure.state.decoration_mode))
        });
        with_decorations(top.wl_surface(), |state| {
            let kde = state.kde_committed_ssd
                && state
                    .kde
                    .as_ref()
                    .is_some_and(|(object, _)| object.is_alive());
            state.render_ssd(committed, kde)
        })
    }

    /// Committed fullscreen content has no compositor titlebar, border or corner cut-outs.
    pub fn outline(&self, frame: Rect, theme: &Theme) -> WindowOutline {
        if self.committed_fullscreen {
            let shape = RoundedShape {
                rect: frame,
                radii: [0.0; 4],
            };
            WindowOutline {
                outer: shape,
                inner: shape,
            }
        } else {
            WindowOutline::new(frame, theme)
        }
    }

    /// Negotiated next mode for configure sizing, including before the first commit.
    /// Do not use this for rendering or pointer coordinates.
    pub fn pending_ssd(&self) -> bool {
        self.window
            .toplevel()
            .is_some_and(|top| with_decorations(top.wl_surface(), |state| state.pending_ssd()))
    }

    /// Retain the exact total frame whenever scene reconciliation configures it.
    /// Hidden-window mode requests reuse it without modifying saved core geometry.
    pub fn remember_frame(&self, frame: Rect) {
        if frame.width > 0 && frame.height > 0 {
            self.last_frame.set(Some(frame));
        }
    }

    /// Restore protocol mode after Smithay resets XDG pending state on unmap.
    pub fn prepare_decoration_configure(&self) {
        if let Some(top) = self.window.toplevel() {
            let mode = with_decorations(top.wl_surface(), |state| state.xdg.map(|xdg| xdg.mode));
            top.with_pending_state(|pending| pending.decoration_mode = mode);
        }
    }

    /// Commit legacy negotiation and XDG destruction; subsurface commits do not count.
    pub fn commit_decoration(&self, has_buffer: bool) {
        if let Some(top) = self.window.toplevel() {
            with_decorations(top.wl_surface(), |state| {
                state.commit(has_buffer);
                if !has_buffer && self.mapped {
                    if let Some(xdg) = &mut state.xdg {
                        xdg.since = SERIAL_COUNTER.next_serial();
                    }
                    self.last_frame.set(None);
                }
            });
        }
    }
}

impl Compositor {
    fn decoration_changed(&mut self, surface: &WlSurface, old_ssd: bool) {
        let Some(id) = self.window_id(surface) else {
            return;
        };
        let entry = &self.windows[&id];
        let Some(top) = entry.window.toplevel() else {
            return;
        };
        let launcher = self
            .runtime
            .config
            .shell
            .is_launcher(&super::protocols::metadata(surface).1);
        let frame = self
            .placements
            .iter()
            .find(|p| p.window == id)
            .map(|p| p.rect)
            .or(entry.last_frame.get())
            .or_else(|| {
                // Fallback for an initial configure with no core placement yet.
                top.with_pending_state(|pending| pending.size).map(|size| {
                    Rect::new(
                        0,
                        0,
                        size.w,
                        size.h.saturating_add(if old_ssd {
                            self.runtime.config.theme.titlebar.height
                        } else {
                            0
                        }),
                    )
                })
            });
        entry.prepare_decoration_configure();
        if !launcher && let Some(frame) = frame {
            entry.remember_frame(frame);
            let content = content_rect(
                frame,
                entry.pending_ssd()
                    && !self
                        .runtime
                        .desktop
                        .window(id)
                        .map_or(entry.initial_fullscreen, |w| w.fullscreen),
                self.runtime.config.theme.titlebar.height,
            );
            top.with_pending_state(|pending| {
                pending.size = Some((content.width, content.height).into())
            });
        }
        // The first bufferless commit owns the initial configure. In particular,
        // get_decoration + set_mode must not send a premature default-sized configure.
        if top.is_initial_configure_sent() {
            top.send_configure();
        }
        self.dirty = true;
    }

    /// Smithay's pinned XdgDecorationHandler has no object-destruction callback.
    /// The dispatch bridge calls this after Smithay has removed its private handle.
    pub fn xdg_decoration_destroyed(&mut self, decoration: &ZxdgToplevelDecorationV1) {
        let Some(top) = decoration.data::<ToplevelSurface>() else {
            return;
        };
        if !top.wl_surface().is_alive() || !top.xdg_toplevel().is_alive() {
            return;
        }
        let committed = top.with_cached_state(|state| {
            state
                .last_acked
                .as_ref()
                .map(|configure| (configure.serial, configure.state.decoration_mode))
        });
        let old_ssd = with_decorations(top.wl_surface(), |state| {
            let old = state.pending_ssd();
            state.destroy_xdg(committed);
            old
        });
        self.decoration_changed(top.wl_surface(), old_ssd);
    }
}

impl XdgDecorationHandler for Compositor {
    fn new_decoration(&mut self, top: ToplevelSurface) {
        let old_ssd = with_decorations(top.wl_surface(), |state| {
            let old = state.pending_ssd();
            state.xdg = Some(XdgDecoration {
                mode: preferred_mode(None),
                since: SERIAL_COUNTER.next_serial(),
            });
            old
        });
        // Smithay clears the object on destroy, but not this per-toplevel flag.
        with_states(top.wl_surface(), |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_decoration_configure_sent = false;
        });
        self.decoration_changed(top.wl_surface(), old_ssd);
    }

    fn request_mode(&mut self, top: ToplevelSurface, mode: Mode) {
        let old_ssd = with_decorations(top.wl_surface(), |state| {
            let old = state.pending_ssd();
            if let Some(xdg) = &mut state.xdg {
                xdg.mode = preferred_mode(Some(mode));
            }
            old
        });
        self.decoration_changed(top.wl_surface(), old_ssd);
    }

    fn unset_mode(&mut self, top: ToplevelSurface) {
        XdgDecorationHandler::request_mode(self, top, preferred_mode(None));
    }
}

impl KdeDecorationHandler for Compositor {
    fn kde_decoration_state(&self) -> &KdeDecorationState {
        &self.kde_decoration_state
    }

    fn new_decoration(&mut self, surface: &WlSurface, decoration: &OrgKdeKwinServerDecoration) {
        let old_ssd = with_decorations(surface, |state| {
            let old = state.pending_ssd();
            state.kde = Some((decoration.clone(), KdeMode::Server));
            state.kde_committed_ssd = false;
            old
        });
        decoration.mode(KdeMode::Server);
        self.decoration_changed(surface, old_ssd);
    }

    fn request_mode(
        &mut self,
        surface: &WlSurface,
        decoration: &OrgKdeKwinServerDecoration,
        mode: WEnum<KdeMode>,
    ) {
        let WEnum::Value(mode) = mode else { return };
        let (old_ssd, changed) = with_decorations(surface, |state| {
            let old = state.pending_ssd();
            let changed = state
                .kde
                .as_ref()
                .is_none_or(|(object, previous)| object != decoration || *previous != mode);
            state.kde = Some((decoration.clone(), mode));
            (old, changed)
        });
        // KDE clients can echo the accepted mode; do not generate configure loops.
        if changed {
            decoration.mode(mode);
            self.decoration_changed(surface, old_ssd);
        }
    }

    fn release(&mut self, decoration: &OrgKdeKwinServerDecoration, surface: &WlSurface) {
        let old_ssd = with_decorations(surface, |state| {
            let old = state.pending_ssd();
            if state
                .kde
                .as_ref()
                .is_some_and(|(object, _)| object == decoration)
            {
                state.kde = None;
                state.kde_committed_ssd = false;
            }
            old
        });
        self.decoration_changed(surface, old_ssd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_defaults_to_ssd_but_honors_explicit_csd() {
        assert_eq!(preferred_mode(None), Mode::ServerSide);
        assert_eq!(preferred_mode(Some(Mode::ServerSide)), Mode::ServerSide);
        assert_eq!(preferred_mode(Some(Mode::ClientSide)), Mode::ClientSide);
        assert!(!Decorations::default().pending_ssd());
    }

    #[test]
    fn committed_mode_not_preference_controls_drawing() {
        let requested_csd = Some(XdgDecoration {
            mode: Mode::ClientSide,
            since: 10.into(),
        });
        assert!(committed_ssd(
            requested_csd,
            Some((11.into(), Some(Mode::ServerSide))),
            false
        ));
        let requested_ssd = Some(XdgDecoration {
            mode: Mode::ServerSide,
            since: 10.into(),
        });
        assert!(!committed_ssd(
            requested_ssd,
            Some((11.into(), Some(Mode::ClientSide))),
            true
        ));
        assert!(!committed_ssd(requested_ssd, None, true));
    }

    #[test]
    fn fresh_and_remapped_objects_cannot_reuse_old_ssd() {
        let old_commit = Some((10.into(), Some(Mode::ServerSide)));
        assert!(!committed_ssd(None, old_commit, false));
        let replacement = Some(XdgDecoration {
            mode: Mode::ServerSide,
            since: 11.into(),
        });
        assert!(!committed_ssd(replacement, old_commit, false));
        assert!(!committed_ssd(replacement, None, false));
        assert!(committed_ssd(
            replacement,
            Some((12.into(), Some(Mode::ServerSide))),
            false
        ));
    }

    #[test]
    fn destroy_keeps_committed_ssd_until_root_commit_not_request_or_ack() {
        let committed = Some((11.into(), Some(Mode::ServerSide)));
        let mut state = Decorations {
            xdg: Some(XdgDecoration {
                // A CSD request/ACK must not replace the committed SSD decision.
                mode: Mode::ClientSide,
                since: 10.into(),
            }),
            ..Default::default()
        };
        state.destroy_xdg(committed);
        assert!(!state.pending_ssd());
        assert!(state.render_ssd(committed, false));

        // A root commit counts even if it reuses the existing buffer/configure.
        state.commit(true);
        assert!(!state.render_ssd(committed, false));
    }

    #[test]
    fn destroyed_xdg_retains_precedence_then_uses_current_kde_lifetime() {
        for mode in [Mode::ClientSide, Mode::ServerSide] {
            let committed = Some((11.into(), Some(mode)));
            let mut state = Decorations {
                xdg: Some(XdgDecoration {
                    mode: Mode::ServerSide,
                    since: 10.into(),
                }),
                ..Default::default()
            };
            state.destroy_xdg(committed);
            for kde_ssd in [false, true] {
                assert_eq!(
                    state.render_ssd(committed, kde_ssd),
                    mode == Mode::ServerSide
                );
            }
            state.commit(true);
            // The retained XDG decision must not pin a legacy object's lifetime.
            assert!(state.render_ssd(committed, true));
            assert!(!state.render_ssd(committed, false));
        }
    }

    #[test]
    fn replacement_keeps_old_pixels_only_until_root_commit() {
        let committed = Some((11.into(), Some(Mode::ServerSide)));
        let mut state = Decorations {
            xdg: Some(XdgDecoration {
                mode: Mode::ServerSide,
                since: 10.into(),
            }),
            ..Default::default()
        };
        state.destroy_xdg(committed);
        state.xdg = Some(XdgDecoration {
            mode: Mode::ClientSide,
            since: 20.into(),
        });
        assert!(!state.pending_ssd());
        assert!(state.render_ssd(committed, false));
        // Destroying an uncommitted replacement must not overwrite the retained mode.
        state.destroy_xdg(committed);
        assert!(state.render_ssd(committed, false));
        state.xdg = Some(XdgDecoration {
            mode: Mode::ServerSide,
            since: 30.into(),
        });
        state.commit(true);
        assert!(!state.render_ssd(committed, true));
        assert!(!state.render_ssd(Some((31.into(), Some(Mode::ClientSide))), true));
        assert!(state.render_ssd(Some((32.into(), Some(Mode::ServerSide))), false));
    }

    #[test]
    fn null_root_commit_clears_destroyed_mode_before_remap() {
        let committed = Some((11.into(), Some(Mode::ServerSide)));
        let mut state = Decorations {
            xdg: Some(XdgDecoration {
                mode: Mode::ServerSide,
                since: 10.into(),
            }),
            kde_committed_ssd: true,
            ..Default::default()
        };
        state.destroy_xdg(committed);
        state.commit(false);
        assert_eq!(state.destroyed_xdg_ssd, None);
        assert!(!state.kde_committed_ssd);
        state.xdg = Some(XdgDecoration {
            mode: Mode::ServerSide,
            since: 20.into(),
        });
        assert!(!state.render_ssd(None, false));
        state.commit(true);
        assert!(!state.render_ssd(committed, false));
        assert!(state.render_ssd(Some((21.into(), Some(Mode::ServerSide))), false));
    }

    #[test]
    fn xdg_takes_precedence_over_legacy() {
        assert!(committed_ssd(None, None, true));
        assert!(!committed_ssd(None, None, false));
        let xdg = Some(XdgDecoration {
            mode: Mode::ServerSide,
            since: 10.into(),
        });
        assert!(!committed_ssd(
            xdg,
            Some((11.into(), Some(Mode::ClientSide))),
            true
        ));
        assert!(committed_ssd(
            xdg,
            Some((11.into(), Some(Mode::ServerSide))),
            false
        ));
    }
}
