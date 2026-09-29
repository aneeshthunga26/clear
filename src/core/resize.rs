use std::collections::BTreeMap;

use super::*;
use crate::management;

/// Edges selected by a resize gesture. Opposing edges on one axis cancel that axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResizeEdges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

/// Optional persistent proportions consumed by pure built-in layouts.
/// Values are owned and updated by the desktop, not by layout policies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LayoutSizing {
    pub(crate) widths: BTreeMap<WindowId, u64>,
    pub(crate) heights: BTreeMap<WindowId, u64>,
    pub(crate) master: Option<(u64, u64)>,
    pub(crate) scrolling: BTreeMap<WindowId, (u32, u32)>,
}

impl LayoutSizing {
    pub(crate) fn remove(&mut self, id: WindowId) {
        self.widths.remove(&id);
        self.heights.remove(&id);
        self.scrolling.remove(&id);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ResizeEnvironment {
    mode: Mode,
    area: Rect,
    gaps: i32,
    group: Vec<OutputId>,
    members: Vec<(WindowId, bool, WindowRole, bool, bool)>,
}

/// Opaque baseline for a resize gesture. Drop it to end or cancel the gesture.
#[derive(Clone, Debug)]
pub struct ResizeSession {
    window: WindowId,
    workspace: WorkspaceId,
    output: OutputId,
    revision: u64,
    mode: Mode,
    baseline: LayoutSizing,
    operation: ResizeOperation,
}

#[derive(Clone, Debug)]
enum ResizeOperation {
    Floating(Rect, ResizeEdges),
    Tiled(Vec<PairResize>),
    Scrolling {
        width: i32,
        viewport: i32,
        reverse: bool,
    },
}

#[derive(Clone, Copy, Debug)]
enum Axis {
    Width,
    Height,
    Master,
}

#[derive(Clone, Debug)]
struct PairResize {
    axis: Axis,
    ids: Vec<WindowId>,
    sizes: Vec<i32>,
    before: usize,
}

impl Desktop {
    /// Starts a supported resize on a visible normal window, without changing its state.
    /// Tiled outer edges are ignored; at least one supported axis must remain.
    /// Spiral, monocle, and script tiles refuse resizing; floating exceptions do not.
    /// Launchers always refuse resizing. Opposing edges cancel only their own axis.
    pub fn begin_resize(&self, id: WindowId, edges: ResizeEdges) -> Option<ResizeSession> {
        let window = self.window(id)?;
        let output = window.output?;
        let workspace = window.workspace;
        if window.role != WindowRole::Normal
            || window.maximized
            || window.minimized
            || self.workspace_for_output(output) != Some(workspace)
        {
            return None;
        }
        let horizontal = edges.left != edges.right;
        let vertical = edges.top != edges.bottom;
        if !horizontal && !vertical {
            return None;
        }
        let mode = self.effective_mode(workspace, output)?.clone();
        let region = self.workspaces.get(&workspace)?.regions.get(&output)?;
        let baseline = region.sizing.get(&mode).cloned().unwrap_or_default();
        let operation = if window.floating || mode == Mode::Floating {
            ResizeOperation::Floating(window.floating_rect, edges)
        } else {
            let context = self.layout_context(workspace, output);
            let placements = management::arrange_with_sizing(&mode, &context, Some(&baseline));
            let ids: Vec<_> = context.windows.iter().map(|window| window.id).collect();
            let index = ids.iter().position(|candidate| *candidate == id)?;
            let rects: Vec<_> = placements.iter().map(|placement| placement.rect).collect();
            let mut pairs = Vec::new();
            let mut pair =
                |axis: Axis, members: &[WindowId], sizes: Vec<i32>, index: usize, leading: bool| {
                    let before = if leading {
                        index.checked_sub(1)
                    } else {
                        Some(index)
                    };
                    if let Some(before) = before.filter(|before| before + 1 < members.len()) {
                        pairs.push(PairResize {
                            axis,
                            ids: members.to_vec(),
                            sizes,
                            before,
                        });
                    }
                };
            match mode {
                Mode::Columns if horizontal => pair(
                    Axis::Width,
                    &ids,
                    rects.iter().map(|r| r.width).collect(),
                    index,
                    edges.left,
                ),
                Mode::Rows if vertical => pair(
                    Axis::Height,
                    &ids,
                    rects.iter().map(|r| r.height).collect(),
                    index,
                    edges.top,
                ),
                Mode::MasterStack => {
                    if ids.len() > 1
                        && horizontal
                        && ((index == 0 && edges.right) || (index > 0 && edges.left))
                    {
                        pair(
                            Axis::Master,
                            &ids[..2],
                            vec![rects[0].width, rects[1].width],
                            0,
                            false,
                        );
                    }
                    if index > 0 && vertical {
                        pair(
                            Axis::Height,
                            &ids[1..],
                            rects[1..].iter().map(|r| r.height).collect(),
                            index - 1,
                            edges.top,
                        );
                    }
                }
                Mode::Grid => {
                    let columns = management::grid_columns(ids.len());
                    let row = index / columns;
                    let start = row * columns;
                    let end = (start + columns).min(ids.len());
                    if horizontal {
                        pair(
                            Axis::Width,
                            &ids[start..end],
                            rects[start..end].iter().map(|r| r.width).collect(),
                            index - start,
                            edges.left,
                        );
                    }
                    if vertical {
                        let rows: Vec<_> = ids.iter().step_by(columns).copied().collect();
                        pair(
                            Axis::Height,
                            &rows,
                            rects.iter().step_by(columns).map(|r| r.height).collect(),
                            row,
                            edges.top,
                        );
                    }
                }
                Mode::Scrolling if horizontal => {}
                _ => return None,
            }
            if mode == Mode::Scrolling {
                ResizeOperation::Scrolling {
                    width: rects[index].width,
                    viewport: context.area.inset(context.gaps).width,
                    reverse: edges.left,
                }
            } else if pairs.is_empty() {
                return None;
            } else {
                ResizeOperation::Tiled(pairs)
            }
        };
        Some(ResizeSession {
            window: id,
            workspace,
            output,
            revision: region.resize_revision,
            mode,
            baseline,
            operation,
        })
    }

    /// Applies TOTAL logical displacement since `begin_resize`, never an incremental delta.
    /// Returns false after region lifecycle changes; clamped supported updates return true.
    /// Tiled updates never modify floating flags or saved floating rectangles.
    /// Floating axes keep the opposite edge fixed, with 64x48 minima where representable.
    /// Scrolling changes only width (64 pixels through viewport width, space permitting),
    /// keeping the window's content-space left edge and recomputing subsequent prefixes.
    pub fn update_resize(&mut self, session: &ResizeSession, dx: i32, dy: i32) -> bool {
        let Some(region) = self
            .workspaces
            .get(&session.workspace)
            .and_then(|workspace| workspace.regions.get(&session.output))
        else {
            return false;
        };
        if region.resize_revision != session.revision
            || region.resize_environment.is_none()
            || !self.windows.contains_key(&session.window)
        {
            return false;
        }
        let mut sizing = session.baseline.clone();
        match &session.operation {
            ResizeOperation::Floating(rect, edges) => {
                let (x, width) =
                    resize_float_axis(rect.x, rect.width, dx, edges.left, edges.right, 64);
                let (y, height) =
                    resize_float_axis(rect.y, rect.height, dy, edges.top, edges.bottom, 48);
                self.windows.get_mut(&session.window).unwrap().floating_rect =
                    Rect::new(x, y, width, height);
            }
            ResizeOperation::Scrolling {
                width,
                viewport,
                reverse,
            } => {
                let delta = if *reverse {
                    -i64::from(dx)
                } else {
                    i64::from(dx)
                };
                let width = (i64::from(*width) + delta)
                    .clamp(i64::from(64.min(*viewport)), i64::from(*viewport));
                sizing
                    .scrolling
                    .insert(session.window, (width as u32, (*viewport).max(1) as u32));
            }
            ResizeOperation::Tiled(pairs) => {
                for pair in pairs {
                    let mut sizes = pair.sizes.clone();
                    let before = pair.before;
                    let total = i64::from(sizes[before]) + i64::from(sizes[before + 1]);
                    let horizontal = !matches!(pair.axis, Axis::Height);
                    let floor = if horizontal { 64 } else { 48 };
                    // Under pressure, use the region's feasible per-tile floor, not a
                    // fixed minimum that could invert the neighboring rectangle.
                    let available: i64 = sizes.iter().map(|size| i64::from(*size)).sum();
                    let minimum = floor.min(available / sizes.len() as i64).min(total / 2);
                    let delta = i64::from(if horizontal { dx } else { dy });
                    sizes[before] =
                        (i64::from(sizes[before]) + delta).clamp(minimum, total - minimum) as i32;
                    sizes[before + 1] = (total - i64::from(sizes[before])) as i32;
                    match pair.axis {
                        Axis::Master => sizing.master = Some((sizes[0] as u64, sizes[1] as u64)),
                        Axis::Width | Axis::Height => {
                            let weights = if horizontal {
                                &mut sizing.widths
                            } else {
                                &mut sizing.heights
                            };
                            weights.extend(
                                pair.ids
                                    .iter()
                                    .copied()
                                    .zip(sizes.into_iter().map(|size| size as u64)),
                            );
                        }
                    }
                }
            }
        }
        if !matches!(session.operation, ResizeOperation::Floating(..)) {
            self.workspaces
                .get_mut(&session.workspace)
                .unwrap()
                .regions
                .get_mut(&session.output)
                .unwrap()
                .sizing
                .insert(session.mode.clone(), sizing);
        }
        // Do not reveal focus during a drag: retain the viewport except for content clamps.
        self.settle(false);
        true
    }

    pub(super) fn layout_sizing(
        &self,
        workspace: WorkspaceId,
        output: OutputId,
        mode: &Mode,
    ) -> Option<&LayoutSizing> {
        self.workspaces
            .get(&workspace)?
            .regions
            .get(&output)?
            .sizing
            .get(mode)
    }

    pub(super) fn refresh_resize_environments(&mut self) {
        let environments: BTreeMap<_, _> = self
            .outputs()
            .map(|output| {
                let workspace = self.workspace_for_output(output.id).unwrap();
                let environment = ResizeEnvironment {
                    mode: self.effective_mode(workspace, output.id).unwrap().clone(),
                    area: output.area,
                    gaps: self.gaps,
                    group: self
                        .groups()
                        .iter()
                        .find(|group| group.workspace == workspace)
                        .unwrap()
                        .outputs
                        .clone(),
                    members: self
                        .workspace(workspace)
                        .unwrap()
                        .windows()
                        .iter()
                        .filter_map(|id| self.window(*id))
                        .filter(|window| window.output == Some(output.id))
                        .map(|window| {
                            (
                                window.id,
                                window.floating,
                                window.role,
                                window.maximized,
                                window.minimized,
                            )
                        })
                        .collect(),
                };
                ((workspace, output.id), environment)
            })
            .collect();
        for (workspace, output) in environments.keys() {
            self.workspaces
                .get_mut(workspace)
                .unwrap()
                .regions
                .entry(*output)
                .or_default();
        }
        for workspace in self.workspaces.values_mut() {
            for (output, region) in &mut workspace.regions {
                let environment = environments.get(&(workspace.id, *output)).cloned();
                if region.resize_environment != environment {
                    region.resize_revision = region.resize_revision.wrapping_add(1);
                    region.resize_environment = environment;
                }
            }
        }
    }
}

fn resize_float_axis(
    origin: i32,
    extent: i32,
    delta: i32,
    leading: bool,
    trailing: bool,
    minimum: i32,
) -> (i32, i32) {
    if leading == trailing {
        return (origin, extent);
    }
    let fixed = i64::from(origin) + i64::from(extent);
    if leading {
        let maximum = (fixed - i64::from(i32::MIN)).min(i64::from(i32::MAX));
        let size =
            (i64::from(extent) - i64::from(delta)).clamp(i64::from(minimum).min(maximum), maximum);
        ((fixed - size) as i32, size as i32)
    } else {
        let maximum = (i64::from(i32::MAX) - i64::from(origin)).min(i64::from(i32::MAX));
        let size =
            (i64::from(extent) + i64::from(delta)).clamp(i64::from(minimum).min(maximum), maximum);
        (origin, size as i32)
    }
}
