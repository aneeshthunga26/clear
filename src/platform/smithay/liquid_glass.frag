// Snell refraction and meniscus lighting for Clear's premultiplied per-tree
// backdrop and fitted outlines.
uniform sampler2D backdrop;
uniform float glass_enabled;
uniform vec2 glass_texel;
uniform vec4 glass_bounds;
// displacement, edge width, dome curvature, dispersion
uniform vec4 glass_optics;
uniform float glass_highlight;

vec4 glass_sample(vec2 uv) {
    return texture2D(backdrop, clamp(uv, glass_bounds.xy, glass_bounds.zw));
}
vec2 glass_uv(vec3 normal, float ior) {
    vec3 ray = refract(vec3(0.0, 0.0, -1.0), normal, 1.0 / ior);
    // The unit ray bounds each displacement axis by refraction_strength.
    // Desktop +Y points down; offscreen texture storage +Y points up.
    return v_coords + ray.xy * vec2(1.0, -1.0) * glass_texel * glass_optics.x;
}
vec3 straight_rgb(vec4 color) {
    return color.a > 0.00001 ? color.rgb / color.a : vec3(0.0);
}
vec4 glass_backdrop(vec2 point) {
    if (glass_enabled < 0.5) return texture2D(backdrop, v_coords);
    float d = shape_distance(point, outline, radii);
    // Square client overflow keeps ordinary blur outside its actual frame.
    if (d < 0.0) return glass_sample(v_coords);
    vec2 half_size = max(outline.zw * 0.5, vec2(0.5));
    float width = min(glass_optics.y, min(half_size.x, half_size.y));
    float edge = 1.0 - smoothstep(0.0, width, d);
    vec2 gradient = vec2(
        shape_distance(point - vec2(0.5, 0.0), outline, radii)
            - shape_distance(point + vec2(0.5, 0.0), outline, radii),
        shape_distance(point - vec2(0.0, 0.5), outline, radii)
            - shape_distance(point + vec2(0.0, 0.5), outline, radii));
    vec2 outward = gradient / max(length(gradient), 0.0001);
    vec2 body = (point - outline.xy - half_size) / half_size;
    vec3 normal = normalize(vec3(outward * edge * 2.0
        + body * glass_optics.z * 0.35, 1.0));
    vec4 green = glass_sample(glass_uv(normal, 1.5));
    vec3 rgb = straight_rgb(green);
    if (glass_optics.w > 0.0 && glass_optics.x > 0.0) {
        rgb.r = straight_rgb(glass_sample(glass_uv(normal, 1.5 - glass_optics.w * 0.15))).r;
        rgb.b = straight_rgb(glass_sample(glass_uv(normal, 1.5 + glass_optics.w * 0.15))).b;
    }
    // A top-left light catches the curved meniscus without painting the client.
    float fresnel = pow(1.0 - normal.z, 3.0);
    float light = max(dot(outward, normalize(vec2(-1.0, -1.0))), 0.0);
    float reflection = clamp(glass_highlight * (edge * edge * light * 0.6 + fresnel), 0.0, 1.0);
    rgb = mix(rgb, vec3(1.0), reflection);
    // Dispersion uses straight channels then the green sample's alpha, so RGB<=A
    // even when different rays cross translucent lower-scene boundaries.
    return vec4(clamp(rgb, 0.0, 1.0) * green.a, green.a);
}
