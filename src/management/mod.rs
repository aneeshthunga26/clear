//! Pure geometry policies. Layouts never mutate desktop state or invoke scripts.

use crate::core::{LayoutContext, LayoutWindow, Mode, Placement, Rect};

/// Arranges one output region. Script modes use master-stack as a safe fallback.
pub fn arrange(mode: &Mode, ctx: &LayoutContext) -> Vec<Placement> {
    let area = ctx.area.normalized();
    let inner = area.inset(ctx.gaps);
    let count = ctx.windows.len();
    let rectangles = match mode {
        Mode::Floating => ctx
            .windows
            .iter()
            .map(|window| window.floating_rect.normalized())
            .collect(),
        Mode::Columns => split(inner, count, ctx.gaps, true),
        Mode::Rows => split(inner, count, ctx.gaps, false),
        Mode::Grid => grid(inner, count, ctx.gaps),
        Mode::Spiral => spiral(inner, count, ctx.gaps),
        Mode::Monocle => vec![inner; count],
        Mode::Scrolling => {
            let (width, stride, _) = scrolling_metrics(ctx);
            let offset = scrolling_offset(ctx, false) as i64;
            (0..count)
                .map(|index| {
                    let x = i64::from(inner.x)
                        .saturating_add(index_i64(index).saturating_mul(stride))
                        .saturating_sub(offset);
                    Rect::new(clamp_i32(x), inner.y, width, inner.height)
                })
                .collect()
        }
        Mode::MasterStack | Mode::Script(_) => {
            if count <= 1 {
                vec![inner; count]
            } else {
                let gap = ctx.gaps.max(0).min(inner.width / 3);
                let available = inner.width - gap;
                let master_width = (i64::from(available) * 3 / 5) as i32;
                let master = Rect::new(inner.x, inner.y, master_width, inner.height);
                let stack = Rect::new(
                    master.right() + gap,
                    inner.y,
                    available - master_width,
                    inner.height,
                );
                let mut result = vec![master];
                result.extend(split(stack, count - 1, ctx.gaps, false));
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

fn split(area: Rect, count: usize, gaps: i32, horizontal: bool) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let length = i64::from(if horizontal { area.width } else { area.height });
    let count64 = index_i64(count);
    let gap = i64::from(gaps.max(0)).min(length / count64.saturating_mul(2).saturating_sub(1));
    let available = length - gap * (count64 - 1);
    let base = available / count64;
    let remainder = available % count64;
    let mut cursor = 0i64;
    (0..count)
        .map(|index| {
            let extent = base + i64::from(index_i64(index) < remainder);
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

fn grid(area: Rect, count: usize, gaps: i32) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let mut columns = 1usize;
    while columns < count.div_ceil(columns) {
        columns += 1;
    }
    let rows = count.div_ceil(columns);
    let mut result = Vec::with_capacity(count);
    for row in split(area, rows, gaps, false) {
        let cells = columns.min(count - result.len());
        result.extend(split(row, cells, gaps, true));
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

fn scrolling_metrics(ctx: &LayoutContext) -> (i32, i64, i32) {
    let area = ctx.area.inset(ctx.gaps);
    let width = ((i64::from(area.width) * 2 / 3) as i32)
        .max(1)
        .min(area.width);
    let gap = ctx.gaps.max(0).min(area.width / 3);
    let stride = i64::from(width) + i64::from(gap);
    let content = if ctx.windows.is_empty() {
        0
    } else {
        index_i64(ctx.windows.len() - 1)
            .saturating_mul(stride)
            .saturating_add(i64::from(width))
    };
    let limit = clamp_i32(content.saturating_sub(i64::from(area.width)).max(0));
    (width, stride, limit)
}

pub(crate) fn scrolling_offset(ctx: &LayoutContext, reveal: bool) -> i32 {
    let (width, stride, limit) = scrolling_metrics(ctx);
    let mut offset = ctx.scroll_offset.clamp(0, limit);
    if reveal
        && let Some(index) = ctx
            .windows
            .iter()
            .position(|window| Some(window.id) == ctx.focused)
    {
        let left = index_i64(index).saturating_mul(stride);
        let right = left.saturating_add(i64::from(width));
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
