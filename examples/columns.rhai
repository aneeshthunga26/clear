// Functions only: top-level statements are never executed. No imports or IO.
// ctx = #{area: #{x, y, width, height}, windows: [#{id, rect}],
//         focused: integer_or_unit, gaps: integer, scroll_offset: integer}
// Every supplied window must occur exactly once, with integer geometry inside area.
fn columns(ctx) {
    let count = ctx.windows.len();
    if count == 0 { return []; }
    let area = ctx.area;
    // Reduce gaps before sacrificing positive dimensions on small outputs.
    let gap = min(ctx.gaps, min(max(0, (area.width - count) / (count + 1)),
                                max(0, (area.height - 1) / 2)));
    let usable = area.width - gap * (count + 1);
    if usable < count {
        throw "output too narrow for positive-width columns";
    }
    let result = [];
    for index in 0..count {
        let left = usable * index / count;
        let right = usable * (index + 1) / count;
        result.push(#{
            window: ctx.windows[index].id,
            x: area.x + gap * (index + 1) + left,
            y: area.y + gap,
            width: right - left,
            height: area.height - gap * 2
        });
    }
    result
}

// Returns requests only; the platform decides whether and how to dispatch them.
fn focus_and_stretch() {
    [#{action: "focus_next"}, #{action: "stretch_all"}]
}
