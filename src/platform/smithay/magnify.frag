// First SVG displacement stage, materialized separately for correct interpolation.
uniform sampler2D magnifying_map;
uniform vec2 glass_texel;
uniform vec4 glass_bounds;
uniform float glass_zoom;
void main() {
    vec2 uv = (desktop_point() - outline.xy) / max(outline.zw, vec2(1.0));
    if (any(lessThan(uv, vec2(0.0))) || any(greaterThanEqual(uv, vec2(1.0)))) {
        gl_FragColor = texture2D(tex, v_coords);
        return;
    }
    vec2 map_value = texture2D(magnifying_map, uv).rg;
    vec2 source = v_coords + 24.0 * glass_zoom * (map_value - 0.5) * vec2(1.0, -1.0) * glass_texel;
    gl_FragColor = texture2D(tex, clamp(source, glass_bounds.xy, glass_bounds.zw));
}
