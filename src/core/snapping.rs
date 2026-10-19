//! Floating placement overrides and transactional pointer-move policy.

use super::*;

/// An edge or corner destination within one output's usable area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapTarget {
    Maximize,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl SnapTarget {
    /// Partition without gaps or overlap; reject destinations too small to split.
    pub fn rect(self, area: Rect) -> Option<Rect> {
        let area = area.normalized();
        if area.is_empty() {
            return None;
        }
        if self == Self::Maximize {
            return Some(area);
        }
        if area.width < 2 {
            return None;
        }
        let left = area.width / 2;
        let top = area.height / 2;
        let right = matches!(self, Self::Right | Self::TopRight | Self::BottomRight);
        let quarter = !matches!(self, Self::Left | Self::Right);
        if quarter && area.height < 2 {
            return None;
        }
        let bottom = matches!(self, Self::BottomLeft | Self::BottomRight);
        Some(Rect::new(
            area.x + if right { left } else { 0 },
            area.y + if quarter && bottom { top } else { 0 },
            if right { area.width - left } else { left },
            if !quarter {
                area.height
            } else if bottom {
                area.height - top
            } else {
                top
            },
        ))
    }

    /// Twelve-pixel edge strips and forgiving 64-pixel corner zones, output-local.
    pub fn at_pointer(bounds: Rect, x: f64, y: f64) -> Option<Self> {
        if !contains_pointer(bounds, x, y) {
            return None;
        }
        let corner_x = (f64::from(bounds.width) / 2.0).min(64.0);
        let corner_y = (f64::from(bounds.height) / 2.0).min(64.0);
        let near_left = x < f64::from(bounds.x) + corner_x;
        let near_right = x >= f64::from(bounds.right()) - corner_x;
        let near_top = y < f64::from(bounds.y) + corner_y;
        let near_bottom = y >= f64::from(bounds.bottom()) - corner_y;
        match (near_left, near_right, near_top, near_bottom) {
            (true, _, true, _) => return Some(Self::TopLeft),
            (_, true, true, _) => return Some(Self::TopRight),
            (true, _, _, true) => return Some(Self::BottomLeft),
            (_, true, _, true) => return Some(Self::BottomRight),
            _ => {}
        }
        let dx = (f64::from(bounds.width) / 2.0).min(12.0);
        let dy = (f64::from(bounds.height) / 2.0).min(12.0);
        let left = x < f64::from(bounds.x) + dx;
        let right = x >= f64::from(bounds.right()) - dx;
        let top = y < f64::from(bounds.y) + dy;
        let bottom = y >= f64::from(bounds.bottom()) - dy;
        match (left, right, top, bottom) {
            (_, _, true, _) => Some(Self::Maximize),
            (true, _, _, _) => Some(Self::Left),
            (_, true, _, _) => Some(Self::Right),
            _ => None,
        }
    }
}

fn contains_pointer(bounds: Rect, x: f64, y: f64) -> bool {
    x.is_finite()
        && y.is_finite()
        && !bounds.is_empty()
        && x >= f64::from(bounds.x)
        && y >= f64::from(bounds.y)
        && x < f64::from(bounds.right())
        && y < f64::from(bounds.bottom())
}

/// A noninteractive, output-local preview; it does not resize the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapPreview {
    pub output: OutputId,
    pub target: SnapTarget,
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MoveState {
    workspace: WorkspaceId,
    output: Option<OutputId>,
    floating: bool,
    maximized: bool,
    snapped: Option<SnapTarget>,
    rect: Rect,
}
impl MoveState {
    fn of(window: &Window) -> Self {
        Self {
            workspace: window.workspace,
            output: window.output,
            floating: window.floating,
            maximized: window.maximized,
            snapped: window.snapped,
            rect: window.floating_rect,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MoveEnvironment(Vec<(OutputId, Rect, u64)>);
impl MoveEnvironment {
    fn capture(desktop: &Desktop) -> Self {
        Self(
            desktop
                .outputs()
                .map(|o| (o.id, o.bounds, Self::revision(desktop, o.id)))
                .collect(),
        )
    }
    fn revision(desktop: &Desktop, output: OutputId) -> u64 {
        desktop
            .workspace_for_output(output)
            .and_then(|w| desktop.workspace(w))
            .and_then(|w| w.regions.get(&output))
            .map_or(0, |r| r.resize_revision)
    }
    fn matches(&self, desktop: &Desktop) -> bool {
        self.0.iter().copied().eq(desktop
            .outputs()
            .map(|o| (o.id, o.bounds, Self::revision(desktop, o.id))))
    }
}

/// Opaque move baseline. Only successful release applies its snap target.
#[derive(Debug, Clone)]
pub struct MoveSession {
    window: WindowId,
    original: MoveState,
    original_order: usize,
    expected: MoveState,
    environment: MoveEnvironment,
    origin: (f64, f64),
    rect: Rect,
    preview: Option<SnapPreview>,
    snapping_enabled: bool,
}

impl MoveSession {
    /// Gate preview and release snapping without losing the latest pointer target.
    /// Timing belongs to the caller; moves enable snapping by default.
    pub fn set_snapping_enabled(&mut self, enabled: bool) {
        self.snapping_enabled = enabled;
    }
}

impl Desktop {
    /// Set a normal window's placement override without revealing or focusing it.
    /// Top-edge maximize uses the ordinary maximize flag; other targets remain floats.
    pub fn snap_window(&mut self, id: WindowId, target: SnapTarget) -> bool {
        let Some(window) = self.window(id) else {
            return false;
        };
        if window.role != WindowRole::Normal
            || window.minimized
            || window.fullscreen
            || window
                .output
                .and_then(|id| self.output(id))
                .and_then(|o| target.rect(o.area))
                .is_none()
        {
            return false;
        }
        let window = self.windows.get_mut(&id).unwrap();
        window.floating = true;
        window.maximized = target == SnapTarget::Maximize;
        window.snapped = (target != SnapTarget::Maximize).then_some(target);
        self.settle(false);
        true
    }

    /// Begin moving an already visible frame, including script-provided placements.
    /// Restores snapped/maximized size under the pointer; fullscreen remains immovable.
    pub fn begin_move(
        &mut self,
        id: WindowId,
        frame: Rect,
        origin: (f64, f64),
    ) -> Option<MoveSession> {
        let window = self.window(id)?;
        let output = window.output?;
        if window.role != WindowRole::Normal
            || window.minimized
            || window.fullscreen
            || self.workspace_for_output(output) != Some(window.workspace)
            || frame.is_empty()
            || !origin.0.is_finite()
            || !origin.1.is_finite()
        {
            return None;
        }
        let original = MoveState::of(window);
        let original_order = self
            .workspace(window.workspace)?
            .windows
            .iter()
            .position(|w| *w == id)?;
        let mut rect = frame;
        if window.maximized || window.snapped.is_some() {
            rect = window.floating_rect;
            let anchor_x =
                ((origin.0 - f64::from(frame.x)) / f64::from(frame.width)).clamp(0.0, 1.0);
            let anchor_y =
                ((origin.1 - f64::from(frame.y)) / f64::from(frame.height)).clamp(0.0, 1.0);
            rect.x = (origin.0 - anchor_x * f64::from(rect.width)).round() as i32;
            rect.y = (origin.1 - anchor_y * f64::from(rect.height)).round() as i32;
            rect = rect.normalized();
        }
        self.command(Command::Focus(id));
        let window = self.windows.get_mut(&id).unwrap();
        window.floating = true;
        window.maximized = false;
        window.snapped = None;
        window.floating_rect = rect;
        self.settle(false);
        Some(MoveSession {
            window: id,
            original,
            original_order,
            expected: MoveState::of(self.window(id)?),
            environment: MoveEnvironment::capture(self),
            origin,
            rect,
            preview: None,
            snapping_enabled: true,
        })
    }

    /// Client commits/metadata do not invalidate a move; policy and topology changes do.
    pub fn move_valid(&self, session: &MoveSession) -> bool {
        self.focused_window() == Some(session.window)
            && session.environment.matches(self)
            && self.window(session.window).is_some_and(|w| {
                w.role == WindowRole::Normal
                    && !w.minimized
                    && !w.fullscreen
                    && MoveState::of(w) == session.expected
            })
    }

    /// Apply total pointer displacement immediately and update the provisional target.
    pub fn update_move(&mut self, session: &mut MoveSession, position: (f64, f64)) -> bool {
        if !position.0.is_finite() || !position.1.is_finite() || !self.move_valid(session) {
            return false;
        }
        let destination = self
            .outputs()
            .find(|o| contains_pointer(o.bounds, position.0, position.1))
            .map(|o| o.id);
        if let Some(output) = destination
            && session.expected.output != Some(output)
        {
            self.command(Command::MoveToOutput(output));
            session.environment = MoveEnvironment::capture(self);
        }
        let rect = Rect::new(
            session
                .rect
                .x
                .saturating_add((position.0 - session.origin.0).round() as i32),
            session
                .rect
                .y
                .saturating_add((position.1 - session.origin.1).round() as i32),
            session.rect.width,
            session.rect.height,
        );
        self.command(Command::SetFloatingRect(session.window, rect));
        session.expected = MoveState::of(self.window(session.window).unwrap());
        session.preview = destination.and_then(|id| self.output(id)).and_then(|o| {
            let target = SnapTarget::at_pointer(o.bounds, position.0, position.1)?;
            Some(SnapPreview {
                output: o.id,
                target,
                rect: target.rect(o.area)?,
            })
        });
        true
    }

    /// Return the valid pointer target even while preview/release snapping is gated.
    pub fn snap_candidate(&self, session: &MoveSession) -> Option<SnapPreview> {
        self.move_valid(session)
            .then_some(session.preview)
            .flatten()
    }

    /// Return the enabled valid target; previews have no client or policy side effects.
    pub fn snap_preview(&self, session: &MoveSession) -> Option<SnapPreview> {
        if session.snapping_enabled {
            self.snap_candidate(session)
        } else {
            None
        }
    }

    /// Finish a valid move. Snapping preserves its pre-drag free geometry for restore.
    pub fn finish_move(&mut self, session: MoveSession) -> bool {
        if !self.move_valid(&session) {
            return false;
        }
        if let Some(preview) = self.snap_preview(&session) {
            self.windows.get_mut(&session.window).unwrap().floating_rect = session.original.rect;
            return self.snap_window(session.window, preview.target);
        }
        true
    }

    /// Escape/focus loss restores the gesture baseline; stale sessions never undo newer policy.
    pub fn cancel_move(&mut self, session: MoveSession) -> bool {
        if !self.move_valid(&session) {
            return false;
        }
        let original = session.original;
        self.command(Command::MoveToOutput(original.output.unwrap()));
        let window = self.windows.get_mut(&session.window).unwrap();
        window.floating = original.floating;
        window.maximized = original.maximized;
        window.snapped = original.snapped;
        window.floating_rect = original.rect;
        let order = &mut self
            .workspaces
            .get_mut(&original.workspace)
            .unwrap()
            .windows;
        order.retain(|id| *id != session.window);
        order.insert(session.original_order.min(order.len()), session.window);
        self.settle(false);
        true
    }
}
