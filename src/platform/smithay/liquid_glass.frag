// SVG filter translation: kube.io/blog/liquid-glass-css-svg/#magnifying-glass.
// Original map bytes and provenance are in glass-maps/.
uniform sampler2D backdrop;
// Strong displacement magnifies half-precision sampler rounding into visible offsets.
uniform highp sampler2D glass_refraction_map;
uniform sampler2D glass_specular_map;
uniform float glass_enabled;
uniform vec2 glass_texel;
uniform vec4 glass_bounds;
// Specular opacity, saturation, and refraction level (the reference's sliders).
uniform vec3 glass_controls;
uniform float glass_refraction_width;

float glass_strip(float p, float size, float first, float last, float middle) {
    if (p < first) return 75.0 * p / first;
    if (p > size - last) return 75.0 + middle + 75.0 * (p - size + last) / last;
    return 75.0 + middle * (p - first) / max(size - first - last, 0.0001);
}
vec2 glass_edge_uv(vec2 point) {
    vec2 p = point - outline.xy;
    vec2 size = max(outline.zw, vec2(1.0));
    vec4 r = max(radii, vec4(0.5));
    float left = max(r.x, r.w), right = max(r.y, r.z);
    float top = max(r.x, r.y), bottom = max(r.w, r.z);
    vec2 blend = clamp((p - vec2(left, top)) /
        max(size - vec2(left + right, top + bottom), vec2(0.0001)), 0.0, 1.0);
    blend = blend * blend * (3.0 - 2.0 * blend);
    float x = glass_strip(p.x, size.x, mix(r.x, r.w, blend.y), mix(r.y, r.z, blend.y), 60.0);
    float y = glass_strip(p.y, size.y, mix(r.x, r.y, blend.x), mix(r.w, r.z, blend.x), 0.0);
    return vec2(x / 210.0, y / 150.0);
}

vec2 glass_refraction_uv(vec2 uv) {
    if (glass_refraction_width == 1.0) return uv;
    vec2 p = uv * vec2(210.0, 150.0);
    vec2 center = vec2(clamp(p.x, 75.0, 135.0), 75.0);
    vec2 radial = p - center;
    float t = clamp(length(radial) / 75.0, 0.0, 1.0);
    // Fix the rim and center while spreading the map's edge profile inward.
    float scale = glass_refraction_width / ((1.0 - t) + glass_refraction_width * t);
    return (center + radial * scale) / vec2(210.0, 150.0);
}

vec2 glass_refraction_sample(vec2 uv) {
    if (glass_refraction_width == 1.0) return texture2D(glass_refraction_map, uv).rg;
    // Hardware filtering can quantize interpolation weights to 8 bits. At high
    // strength, interpolate map values explicitly before using them as offsets.
    vec2 map_size = vec2(420.0, 300.0);
    vec2 p = clamp(uv * map_size - 0.5, vec2(0.0), map_size - 1.0);
    vec2 base = floor(p), weight = fract(p);
    vec2 lo = (base + 0.5) / map_size;
    vec2 hi = (min(base + 1.0, map_size - 1.0) + 0.5) / map_size;
    vec2 top = mix(texture2D(glass_refraction_map, lo).rg,
                   texture2D(glass_refraction_map, vec2(hi.x, lo.y)).rg, weight.x);
    vec2 bottom = mix(texture2D(glass_refraction_map, vec2(lo.x, hi.y)).rg,
                      texture2D(glass_refraction_map, hi).rg, weight.x);
    return mix(top, bottom, weight.y);
}

vec4 glass_backdrop(vec2 point) {
    if (glass_enabled < 0.5) return texture2D(backdrop, v_coords);
    if (shape_distance(point, outline, radii) < 0.0) return texture2D(backdrop, v_coords);
    // The optical map belongs to the original frame, never its output crop.
    vec2 uv = glass_edge_uv(point);
    vec2 displacement = vec2(0.0);
    if (glass_refraction_width > 0.0) {
        displacement = glass_refraction_sample(glass_refraction_uv(uv)) - 0.5;
    }
    vec2 refracted_uv = v_coords + displacement * (122.80891678834695 * 0.8)
        * glass_controls.z * vec2(1.0, -1.0) * glass_texel;
    vec4 displaced = texture2D(backdrop, clamp(refracted_uv, glass_bounds.xy, glass_bounds.zw));
    vec4 specular = texture2D(glass_specular_map, uv);
    vec3 straight = displaced.a > 0.00001 ? displaced.rgb / displaced.a : vec3(0.0);
    float luminance = dot(straight, vec3(0.213, 0.715, 0.072));
    vec3 saturated = clamp(vec3(luminance) + glass_controls.y * (straight - luminance), 0.0, 1.0);
    // feColorMatrix, feComposite(in), then the two normal feBlend operations.
    vec4 masked = vec4(saturated * displaced.a, displaced.a) * specular.a;
    vec4 with_saturation = masked + displaced * (1.0 - masked.a);
    vec4 faded = specular * glass_controls.x;
    return faded + with_saturation * (1.0 - faded.a);
}
