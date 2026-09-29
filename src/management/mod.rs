//! Pure geometry policies. Layouts never mutate desktop state or invoke scripts.

use std::collections::BTreeMap;

use crate::core::{LayoutContext, LayoutSizing, LayoutWindow, Mode, Placement, Rect, WindowId};

/// Arranges one output region. Script modes use master-stack as a safe fallback.
pub fn arrange(mode: &Mode, ctx: &LayoutContext) -> Vec<Placement> {
    arrange_with_sizing(mode, ctx, None)
}

/// Arranges a region using optional desktop-owned proportions. No state is mutated.
pub fn arrange_with_sizing(
    mode: &Mode,
    ctx: &LayoutContext,
    sizing: Option<&LayoutSizing>,
) -> Vec<Placement> {
    let area = ctx.area.normalized();
    let inner = area.inset(ctx.gaps);
    let count = ctx.windows.len();
    let rectangles = match mode {
        Mode::Floating => ctx
            .windows
            .iter()
            .map(|window| window.floating_rect.normalized())
            .collect(),
        Mode::Columns => split_sized(
            inner,
            count,
            ctx.gaps,
            true,
            weights(&ctx.windows, sizing.map(|s| &s.widths)).as_deref(),
        ),
        Mode::Rows => split_sized(
            inner,
            count,
            ctx.gaps,
            false,
            weights(&ctx.windows, sizing.map(|s| &s.heights)).as_deref(),
        ),
        Mode::Grid => grid(inner, &ctx.windows, ctx.gaps, sizing),
        Mode::Spiral => spiral(inner, count, ctx.gaps),
        Mode::Monocle => vec![inner; count],
        Mode::Scrolling => {
            let (widths, gap, _) = scrolling_metrics(ctx, sizing);
            let offset = scrolling_offset_with_sizing(ctx, false, sizing) as i64;
            let mut cursor = i64::from(inner.x) - offset;
            widths
                .into_iter()
                .map(|width| {
                    let rect = Rect::new(clamp_i32(cursor), inner.y, width, inner.height);
                    cursor = cursor.saturating_add(i64::from(width) + gap);
                    rect
                })
                .collect()
        }
        Mode::MasterStack | Mode::Script(_) => {
            if count <= 1 {
                vec![inner; count]
            } else {
                let gap = ctx.gaps.max(0).min(inner.width / 3);
                let available = inner.width - gap;
                let master_width = sizing.and_then(|s| s.master).map_or_else(
                    || (i64::from(available) * 3 / 5) as i32,
                    |(master, stack)| weighted_extents(available, &[master, stack], 64)[0],
                );
                let master = Rect::new(inner.x, inner.y, master_width, inner.height);
                let stack = Rect::new(
                    master.right() + gap,
                    inner.y,
                    available - master_width,
                    inner.height,
                );
                let mut result = vec![master];
                result.extend(split_sized(
                    stack,
                    count - 1,
                    ctx.gaps,
                    false,
                    weights(&ctx.windows[1..], sizing.map(|s| &s.heights)).as_deref(),
                ));
                result
            }
        }
    };
    let mut result: Vec<_> = ctx
        .windows
        .iter()
        .zip(rectangles)
        .map(|(window, rect)| {
            placement(
                window,
                rect,
                matches!(mode, Mode::Scrolling).then_some(inner),
                !matches!(mode, Mode::Floating),
                ctx,
            )
        })
        .collect();
    if matches!(mode, Mode::Floating | Mode::Monocle) {
        result.sort_by_key(|placement| placement.focused);
    }
    result
}

fn placement(
    window: &LayoutWindow,
    rect: Rect,
    clip: Option<Rect>,
    tiled: bool,
    ctx: &LayoutContext,
) -> Placement {
    Placement {
        window: window.id,
        rect,
        clip,
        focused: ctx.focused == Some(window.id),
        tiled,
    }
}

fn split_sized(
    area: Rect,
    count: usize,
    gaps: i32,
    horizontal: bool,
    weights: Option<&[u64]>,
) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let length = i64::from(if horizontal { area.width } else { area.height });
    let count64 = index_i64(count);
    let gap = i64::from(gaps.max(0)).min(length / count64.saturating_mul(2).saturating_sub(1));
    let available = length - gap * (count64 - 1);
    let base = available / count64;
    let remainder = available % count64;
    let sized = weights.map(|weights| {
        weighted_extents(available as i32, weights, if horizontal { 64 } else { 48 })
    });
    let mut cursor = 0i64;
    (0..count)
        .map(|index| {
            let extent = sized.as_ref().map_or_else(
                || base + i64::from(index_i64(index) < remainder),
                |sizes| i64::from(sizes[index]),
            );
            let rect = if horizontal {
                Rect::new(
                    clamp_i32(i64::from(area.x) + cursor),
                    area.y,
                    extent as i32,
                    area.height,
                )
            } else {
                Rect::new(
                    area.x,
                    clamp_i32(i64::from(area.y) + cursor),
                    area.width,
                    extent as i32,
                )
            };
            cursor += extent + gap;
            rect
        })
        .collect()
}

pub(crate) fn grid_columns(count: usize) -> usize {
    let mut columns = 1usize;
    while columns < count.div_ceil(columns) {
        columns += 1;
    }
    columns
}

fn grid(
    area: Rect,
    windows: &[LayoutWindow],
    gaps: i32,
    sizing: Option<&LayoutSizing>,
) -> Vec<Rect> {
    if windows.is_empty() {
        return Vec::new();
    }
    let columns = grid_columns(windows.len());
    let anchors: Vec<_> = windows.iter().step_by(columns).copied().collect();
    let rows = split_sized(
        area,
        anchors.len(),
        gaps,
        false,
        weights(&anchors, sizing.map(|s| &s.heights)).as_deref(),
    );
    let mut result = Vec::with_capacity(windows.len());
    for (row, cells) in rows.into_iter().zip(windows.chunks(columns)) {
        result.extend(split_sized(
            row,
            cells.len(),
            gaps,
            true,
            weights(cells, sizing.map(|s| &s.widths)).as_deref(),
        ));
    }
    result
}

fn weights(windows: &[LayoutWindow], values: Option<&BTreeMap<WindowId, u64>>) -> Option<Vec<u64>> {
    let values = values?;
    let known: Vec<_> = windows
        .iter()
        .filter_map(|window| values.get(&window.id).copied())
        .collect();
    if known.is_empty() {
        return None;
    }
    let mean =
        (known.iter().map(|weight| u128::from(*weight)).sum::<u128>() / known.len() as u128) as u64;
    Some(
        windows
            .iter()
            .map(|window| values.get(&window.id).copied().unwrap_or(mean.max(1)))
            .collect(),
    )
}

// Integer water filling preserves exact pixel baselines, imposes feasible floors,
// and assigns every available pixel without overlapping even in zero-sized areas.
fn weighted_extents(available: i32, weights: &[u64], minimum: i32) -> Vec<i32> {
    if weights.is_empty() {
        return Vec::new();
    }
    let minimum = minimum.min(available / weights.len() as i32);
    let mut result = vec![0; weights.len()];
    let mut active: Vec<_> = (0..weights.len()).collect();
    let mut remaining = available as u128;
    loop {
        let total: u128 = active.iter().map(|index| u128::from(weights[*index])).sum();
        let effective = |index: usize| {
            if total == 0 {
                1
            } else {
                u128::from(weights[index])
            }
        };
        let total = total.max(active.len() as u128 * u128::from(total == 0));
        if active.is_empty() {
            break;
        }
        let small: Vec<_> = active
            .iter()
            .copied()
            .filter(|index| effective(*index) * remaining < minimum as u128 * total)
            .collect();
        if !small.is_empty() {
            for index in &small {
                result[*index] = minimum;
                remaining -= minimum as u128;
            }
            active.retain(|index| !small.contains(index));
            continue;
        }
        let mut remainders = Vec::with_capacity(active.len());
        let mut used = 0;
        for index in active {
            let product = effective(index) * remaining;
            let size = product / total;
            result[index] = size as i32;
            used += size;
            remainders.push((index, product % total));
        }
        remainders.sort_by_key(|(index, remainder)| (std::cmp::Reverse(*remainder), *index));
        for (index, _) in remainders.into_iter().take((remaining - used) as usize) {
            result[index] += 1;
        }
        break;
    }
    result
}

fn spiral(area: Rect, count: usize, gaps: i32) -> Vec<Rect> {
    let mut result = Vec::with_capacity(count);
    let mut remaining = area;
    for index in 0..count {
        if index + 1 == count {
            result.push(remaining);
            break;
        }
        let horizontal = index % 2 == 0;
        let length = if horizontal {
            remaining.width
        } else {
            remaining.height
        };
        let gap = gaps.max(0).min(length / 3);
        let available = length - gap;
        let taken = (i64::from(available) * 3 / 5) as i32;
        let rest = available - taken;
        let (tile, tail) = match index % 4 {
            0 => (
                Rect::new(remaining.x, remaining.y, taken, remaining.height),
                Rect::new(
                    remaining.x + taken + gap,
                    remaining.y,
                    rest,
                    remaining.height,
                ),
            ),
            1 => (
                Rect::new(remaining.x, remaining.y, remaining.width, taken),
                Rect::new(
                    remaining.x,
                    remaining.y + taken + gap,
                    remaining.width,
                    rest,
                ),
            ),
            2 => (
                Rect::new(
                    remaining.right() - taken,
                    remaining.y,
                    taken,
                    remaining.height,
                ),
                Rect::new(remaining.x, remaining.y, rest, remaining.height),
            ),
            _ => (
                Rect::new(
                    remaining.x,
                    remaining.bottom() - taken,
                    remaining.width,
                    taken,
                ),
                Rect::new(remaining.x, remaining.y, remaining.width, rest),
            ),
        };
        result.push(tile);
        remaining = tail;
    }
    result
}

fn scrolling_metrics(ctx: &LayoutContext, sizing: Option<&LayoutSizing>) -> (Vec<i32>, i64, i32) {
    let area = ctx.area.inset(ctx.gaps);
    let default = ((i64::from(area.width) * 2 / 3) as i32)
        .max(1)
        .min(area.width);
    let widths: Vec<_> = ctx
        .windows
        .iter()
        .map(|window| {
            sizing
                .and_then(|s| s.scrolling.get(&window.id))
                .map_or(default, |(width, viewport)| {
                    (i64::from(*width) * i64::from(area.width) / i64::from((*viewport).max(1)))
                        .clamp(i64::from(64.min(area.width)), i64::from(area.width))
                        as i32
                })
        })
        .collect();
    let gap = i64::from(ctx.gaps.max(0).min(area.width / 3));
    let content = widths
        .iter()
        .fold(0i64, |sum, width| sum.saturating_add(i64::from(*width)))
        .saturating_add(index_i64(widths.len().saturating_sub(1)).saturating_mul(gap));
    let limit = clamp_i32(content.saturating_sub(i64::from(area.width)).max(0));
    (widths, gap, limit)
}

pub(crate) fn scrolling_offset_with_sizing(
    ctx: &LayoutContext,
    reveal: bool,
    sizing: Option<&LayoutSizing>,
) -> i32 {
    let (widths, gap, limit) = scrolling_metrics(ctx, sizing);
    let mut offset = ctx.scroll_offset.clamp(0, limit);
    if reveal
        && let Some(index) = ctx
            .windows
            .iter()
            .position(|window| Some(window.id) == ctx.focused)
    {
        let left = widths[..index].iter().fold(0i64, |sum, width| {
            sum.saturating_add(i64::from(*width) + gap)
        });
        let right = left.saturating_add(i64::from(widths[index]));
        let viewport = i64::from(ctx.area.inset(ctx.gaps).width);
        if left < i64::from(offset) {
            offset = clamp_i32(left);
        } else if right > i64::from(offset).saturating_add(viewport) {
            offset = clamp_i32(right.saturating_sub(viewport));
        }
    }
    offset.clamp(0, limit)
}

fn index_i64(index: usize) -> i64 {
    i64::try_from(index).unwrap_or(i64::MAX)
}

fn clamp_i32(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}
