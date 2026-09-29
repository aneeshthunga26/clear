//! Scale-one nested backdrop composition. Foreground trees are never filtered.

use super::{
    rounded::{RoundedShape, SHAPE, TEXTURE_HEADER},
    scene::{Scene, SceneElement},
};
use crate::decoration::BlurMethod;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Frame, Offscreen, Renderer,
            element::{Element, Id, Kind, RenderElement, texture::TextureRenderElement},
            gles::{
                GlesError, GlesFrame, GlesRenderer, GlesTexProgram, GlesTexture, Uniform,
                UniformName, UniformType, ffi,
            },
        },
    },
    utils::{Buffer, Physical, Rectangle, Size, Transform},
};

#[derive(Clone, Copy)]
pub(super) enum BlurMask {
    /// Independent geometric coverage, not the already-masked client alpha.
    Window(RoundedShape),
    /// Square floating trees may extend beyond the requested frame while resizing.
    SquareWindow(RoundedShape),
    /// No protocol blur region: weight blur by tree alpha, preserving transparent holes.
    ClientAlpha,
}

/// Four framebuffer-sized textures, reused across trees and frames; resize replaces them.
struct Scratch {
    size: Size<i32, Physical>,
    scene: GlesTexture,
    horizontal: GlesTexture,
    vertical: GlesTexture,
    foreground: GlesTexture,
}

pub(super) struct BackdropBlur {
    gaussian: Option<GlesTexProgram>,
    kawase: Option<Kawase>,
    composite: GlesTexProgram,
    scratch: Option<Scratch>,
}

impl BackdropBlur {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let composite = renderer.compile_custom_texture_shader(
            format!("{TEXTURE_HEADER}\n{SHAPE}\n{COMPOSITE}"),
            &[
                UniformName::new("backdrop", UniformType::_1i),
                UniformName::new("original", UniformType::_1i),
                UniformName::new("client_shape", UniformType::_1f),
                UniformName::new("outline", UniformType::_4f),
                UniformName::new("radii", UniformType::_4f),
                UniformName::new("target_height", UniformType::_1f),
            ],
        )?;
        Ok(Self {
            gaussian: None,
            kawase: None,
            composite,
            scratch: None,
        })
    }

    /// Consume lower groups before each tree. The backend must bind/render the window
    /// target afterwards; no offscreen pass may be followed directly by EGL submission.
    pub fn render(
        &mut self,
        renderer: &mut GlesRenderer,
        scene: &Scene,
        size: Size<i32, Physical>,
        radius: f32,
        method: BlurMethod,
        passes: u8,
        background: [f32; 4],
    ) -> Result<TextureRenderElement<GlesTexture>, GlesError> {
        // Config validation owns finite 0..=32; the backend owns the exact zero path.
        match method {
            BlurMethod::Gaussian if self.gaussian.is_none() => {
                self.gaussian = Some(renderer.compile_custom_texture_shader(
                    format!("{TEXTURE_HEADER}\n{GAUSSIAN}"),
                    &[
                        UniformName::new("direction", UniformType::_2f),
                        UniformName::new("sample_bounds", UniformType::_4f),
                        UniformName::new("radius", UniformType::_1f),
                        UniformName::new("coefficient", UniformType::_1f),
                        UniformName::new("coefficient_step", UniformType::_1f),
                    ],
                )?);
            }
            BlurMethod::Kawase if self.kawase.is_none() => {
                self.kawase = Some(Kawase::new(renderer)?);
            }
            _ => {}
        }
        if self
            .scratch
            .as_ref()
            .is_none_or(|scratch| scratch.size != size)
        {
            if let Some(kawase) = &mut self.kawase {
                kawase.pyramids.clear();
            }
            let mut texture = || scratch_texture(renderer, size);
            self.scratch = Some(Scratch {
                size,
                scene: texture()?,
                horizontal: texture()?,
                vertical: texture()?,
                foreground: texture()?,
            });
        }
        let scratch = self.scratch.as_mut().expect("scratch allocated");
        let full = Rectangle::from_size(size);
        offscreen(renderer, &mut scratch.scene, size, |frame| {
            frame.clear(background.into(), &[full])
        })?;
        for group in scene.groups.iter().rev() {
            let elements = &scene.elements[group.elements.clone()];
            let Some(clip) = group.clip.intersection(full) else {
                continue;
            };
            let Some(area) = tree_bounds(elements).and_then(|bounds| bounds.intersection(clip))
            else {
                continue;
            };
            if group.mask.is_none() || tree_is_opaque(elements, area) {
                offscreen(renderer, &mut scratch.scene, size, |frame| {
                    draw_tree(frame, elements)
                })?;
                continue;
            }
            let mask = group.mask.expect("blurred tree has a mask");
            offscreen(renderer, &mut scratch.foreground, size, |frame| {
                frame.clear([0.0; 4].into(), &[area])?;
                draw_tree(frame, elements)
            })?;

            if method == BlurMethod::Kawase {
                self.kawase
                    .as_mut()
                    .expect("Kawase initialized")
                    .filter(renderer, scratch, clip, area, radius, passes)?;
            } else {
                let kernel = Gaussian::new(radius);
                let bounds = sample_bounds(clip, size);
                // The vertical pass needs horizontally filtered rows beyond the tree.
                // Both passes clamp to this viewport, never the neighboring virtual output.
                let horizontal_area = vertical_support(area, clip, kernel.support());
                let pass = |renderer: &mut GlesRenderer,
                            source: &GlesTexture,
                            target: &mut GlesTexture,
                            area,
                            direction: [f32; 2]| {
                    let uniforms = [
                        Uniform::new("direction", direction),
                        Uniform::new("sample_bounds", bounds),
                        Uniform::new("radius", radius),
                        Uniform::new("coefficient", kernel.coefficient),
                        Uniform::new("coefficient_step", kernel.coefficient_step),
                    ];
                    offscreen(renderer, target, size, |frame| {
                        draw_texture(frame, source, size, area, self.gaussian.as_ref(), &uniforms)
                    })
                };
                pass(
                    renderer,
                    &scratch.scene,
                    &mut scratch.horizontal,
                    horizontal_area,
                    [1.0 / size.w as f32, 0.0],
                )?;
                pass(
                    renderer,
                    &scratch.horizontal,
                    &mut scratch.vertical,
                    area,
                    [0.0, 1.0 / size.h as f32],
                )?;
            }

            let shape = match mask {
                BlurMask::Window(shape) | BlurMask::SquareWindow(shape) => shape,
                BlurMask::ClientAlpha => RoundedShape {
                    rect: crate::core::Rect::new(0, 0, size.w, size.h),
                    radii: [0.0; 4],
                },
            };
            let mut uniforms = shape.uniforms(size.h);
            uniforms.extend([
                Uniform::new("backdrop", 1_i32),
                Uniform::new("original", 2_i32),
                Uniform::new(
                    "client_shape",
                    match mask {
                        BlurMask::Window(_) => 0.0_f32,
                        BlurMask::ClientAlpha => 1.0,
                        BlurMask::SquareWindow(_) => 2.0,
                    },
                ),
            ]);
            // Horizontal is no longer needed. Reuse it for the resolved group, reading
            // original/blurred/foreground from three *different* texture attachments.
            offscreen(renderer, &mut scratch.horizontal, size, |frame| {
                with_backdrops(frame, &scratch.vertical, &scratch.scene, |frame| {
                    draw_texture(
                        frame,
                        &scratch.foreground,
                        size,
                        area,
                        Some(&self.composite),
                        &uniforms,
                    )
                })
            })?;
            // Replace rather than source-over: the shader already included the original
            // backdrop, including its alpha. This also supports non-opaque theme colors.
            offscreen(renderer, &mut scratch.scene, size, |frame| {
                draw_texture(frame, &scratch.horizontal, size, area, None, &[])
            })?;
        }
        Ok(TextureRenderElement::from_static_texture(
            Id::new(),
            renderer.context_id(),
            (0.0, 0.0),
            scratch.scene.clone(),
            1,
            Transform::Flipped180,
            None,
            None,
            None,
            None,
            Kind::Unspecified,
        ))
    }
}

// These taps also generate the GLSL, so CPU kernel tests exercise the actual weights.
// Offsets are multiples of radius in SOURCE texels, without an implicit half-texel.
const KAWASE_DOWN: &[[f32; 3]] = &[
    [0.0, 0.0, 4.0],
    [-1.0, -1.0, 1.0],
    [1.0, -1.0, 1.0],
    [-1.0, 1.0, 1.0],
    [1.0, 1.0, 1.0],
];
const KAWASE_UP: &[[f32; 3]] = &[
    [-2.0, 0.0, 1.0],
    [2.0, 0.0, 1.0],
    [0.0, -2.0, 1.0],
    [0.0, 2.0, 1.0],
    [-1.0, -1.0, 2.0],
    [1.0, -1.0, 2.0],
    [-1.0, 1.0, 2.0],
    [1.0, 1.0, 2.0],
];

struct Pyramid {
    sizes: Vec<Size<i32, Physical>>,
    levels: Vec<GlesTexture>,
}

struct Kawase {
    down: GlesTexProgram,
    up: GlesTexProgram,
    // LRU, keyed by viewport size/effective depth, never by origin or radius.
    // At most four entries and three framebuffer areas of ABGR8888 storage.
    pyramids: Vec<Pyramid>,
}

impl Kawase {
    fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let mut compile = |taps| {
            renderer.compile_custom_texture_shader(
                kawase_shader(taps),
                &[
                    UniformName::new("offset", UniformType::_2f),
                    UniformName::new("sample_bounds", UniformType::_4f),
                ],
            )
        };
        Ok(Self {
            down: compile(KAWASE_DOWN)?,
            up: compile(KAWASE_UP)?,
            pyramids: Vec::new(),
        })
    }

    fn filter(
        &mut self,
        renderer: &mut GlesRenderer,
        scratch: &mut Scratch,
        clip: Rectangle<i32, Physical>,
        area: Rectangle<i32, Physical>,
        radius: f32,
        passes: u8,
    ) -> Result<(), GlesError> {
        let sizes = pyramid_sizes(clip.size, passes);
        if let Some(index) = self.pyramids.iter().position(|p| p.sizes == sizes) {
            let pyramid = self.pyramids.remove(index);
            self.pyramids.push(pyramid);
        } else {
            let needed = pyramid_pixels(&sizes);
            let budget = 3 * pixel_count(scratch.size);
            while !self.pyramids.is_empty()
                && (self.pyramids.len() >= 4
                    || self
                        .pyramids
                        .iter()
                        .map(|p| pyramid_pixels(&p.sizes))
                        .sum::<u64>()
                        + needed
                        > budget)
            {
                self.pyramids.remove(0);
            }
            let levels = sizes
                .iter()
                .map(|&size| scratch_texture(renderer, size))
                .collect::<Result<Vec<_>, _>>()?;
            self.pyramids.push(Pyramid { sizes, levels });
        }
        let pyramid = self.pyramids.last_mut().expect("pyramid allocated");
        // Isolate the *whole* viewport before filtering, not merely the tree bounds.
        // The scene is bottom-up in texture storage; source crops must be too.
        let source = storage_rect(clip, scratch.size);
        kawase_pass(
            renderer,
            &scratch.scene,
            &mut pyramid.levels[0],
            source,
            clip.size,
            sample_bounds(clip, scratch.size),
            [0.0; 2],
            &self.down,
        )?;
        let depth = pyramid.levels.len() - 1;
        for level in 1..=depth {
            let (sources, targets) = pyramid.levels.split_at_mut(level);
            kawase_level(
                renderer,
                &sources[level - 1],
                pyramid.sizes[level - 1],
                &mut targets[0],
                pyramid.sizes[level],
                radius,
                &self.down,
            )?;
        }
        // Overwrite the old downsample levels on ascent: never sample the writable
        // attachment, and never blend with an earlier tree/frame's cached pixels.
        for level in (1..=depth).rev() {
            let (targets, sources) = pyramid.levels.split_at_mut(level);
            kawase_level(
                renderer,
                &sources[0],
                pyramid.sizes[level],
                &mut targets[level - 1],
                pyramid.sizes[level - 1],
                radius,
                &self.up,
            )?;
        }
        // Return to the existing full-frame composite without changing its masks.
        // Damage/opaque regions are relative to the destination, not the framebuffer.
        offscreen(renderer, &mut scratch.vertical, scratch.size, |frame| {
            frame.render_texture_from_to(
                &pyramid.levels[0],
                storage_rect(Rectangle::from_size(clip.size), clip.size),
                clip,
                &[Rectangle::new(area.loc - clip.loc, area.size)],
                &[Rectangle::from_size(clip.size)],
                Transform::Flipped180,
                1.0,
                None,
                &[],
            )
        })
    }
}

/// Ceil-halving keeps odd edge texels; stop only when both dimensions reach one.
fn pyramid_sizes(size: Size<i32, Physical>, passes: u8) -> Vec<Size<i32, Physical>> {
    assert!(size.w > 0 && size.h > 0 && (1..=6).contains(&passes));
    let mut sizes = vec![size];
    let mut size = size;
    for _ in 0..passes {
        if size.w == 1 && size.h == 1 {
            break;
        }
        size = (size.w / 2 + size.w % 2, size.h / 2 + size.h % 2).into();
        sizes.push(size);
    }
    sizes
}

fn pixel_count(size: Size<i32, Physical>) -> u64 {
    size.w as u64 * size.h as u64
}

fn pyramid_pixels(sizes: &[Size<i32, Physical>]) -> u64 {
    sizes.iter().map(|&size| pixel_count(size)).sum()
}

fn storage_rect(
    clip: Rectangle<i32, Physical>,
    size: Size<i32, Physical>,
) -> Rectangle<f64, Buffer> {
    Rectangle::new(
        (
            clip.loc.x as f64,
            (size.h - clip.loc.y - clip.size.h) as f64,
        )
            .into(),
        (clip.size.w as f64, clip.size.h as f64).into(),
    )
}

fn kawase_level(
    renderer: &mut GlesRenderer,
    source: &GlesTexture,
    source_size: Size<i32, Physical>,
    target: &mut GlesTexture,
    target_size: Size<i32, Physical>,
    radius: f32,
    program: &GlesTexProgram,
) -> Result<(), GlesError> {
    let full = Rectangle::from_size(source_size);
    kawase_pass(
        renderer,
        source,
        target,
        storage_rect(full, source_size),
        target_size,
        sample_bounds(full, source_size),
        [radius / source_size.w as f32, radius / source_size.h as f32],
        program,
    )
}

fn kawase_pass(
    renderer: &mut GlesRenderer,
    source: &GlesTexture,
    target: &mut GlesTexture,
    source_rect: Rectangle<f64, Buffer>,
    target_size: Size<i32, Physical>,
    bounds: [f32; 4],
    offset: [f32; 2],
    program: &GlesTexProgram,
) -> Result<(), GlesError> {
    let full = Rectangle::from_size(target_size);
    offscreen(renderer, target, target_size, |frame| {
        // Smithay's linear min/mag defaults are retained. At destination pixel i,
        // v_coords maps to source edge coordinate (i + .5) * source/target.
        frame.render_texture_from_to(
            source,
            source_rect,
            full,
            &[full],
            &[full],
            Transform::Flipped180,
            1.0,
            Some(program),
            &[
                Uniform::new("offset", offset),
                Uniform::new("sample_bounds", bounds),
            ],
        )
    })
}

fn kawase_shader(taps: &[[f32; 3]]) -> String {
    use std::fmt::Write;
    let mut shader = format!(
        "{TEXTURE_HEADER}\n\
        uniform vec2 offset;\n\
        uniform vec4 sample_bounds;\n\
        vec4 sample_scene(vec2 p) {{\n\
            return texture2D(tex, clamp(p, sample_bounds.xy, sample_bounds.zw));\n\
        }}\n\
        void main() {{\n\
            vec4 color = vec4(0.0);\n"
    );
    for &[x, y, weight] in taps {
        writeln!(
            shader,
            "color += {weight:.1} * sample_scene(v_coords + offset * vec2({x:.1}, {y:.1}));"
        )
        .expect("write shader string");
    }
    writeln!(
        shader,
        "gl_FragColor = color / {:.1};\n}}",
        taps.iter().map(|tap| tap[2]).sum::<f32>()
    )
    .expect("write shader string");
    shader
}

fn scratch_texture(
    renderer: &mut GlesRenderer,
    size: Size<i32, Physical>,
) -> Result<GlesTexture, GlesError> {
    let texture: GlesTexture = renderer.create_buffer(Fourcc::Abgr8888, (size.w, size.h).into())?;
    // Auxiliary samplers bypass Smithay's texture draw setup. These are owned
    // textures; establish complete non-mipmapped sampling without leaking bindings.
    renderer.with_context(|gl| unsafe {
        let mut previous = 0;
        gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut previous);
        gl.BindTexture(ffi::TEXTURE_2D, texture.tex_id());
        for parameter in [ffi::TEXTURE_MIN_FILTER, ffi::TEXTURE_MAG_FILTER] {
            gl.TexParameteri(ffi::TEXTURE_2D, parameter, ffi::LINEAR as i32);
        }
        for parameter in [ffi::TEXTURE_WRAP_S, ffi::TEXTURE_WRAP_T] {
            gl.TexParameteri(ffi::TEXTURE_2D, parameter, ffi::CLAMP_TO_EDGE as i32);
        }
        gl.BindTexture(ffi::TEXTURE_2D, previous as u32);
    })?;
    Ok(texture)
}

fn offscreen(
    renderer: &mut GlesRenderer,
    texture: &mut GlesTexture,
    size: Size<i32, Physical>,
    draw: impl FnOnce(&mut GlesFrame<'_, '_>) -> Result<(), GlesError>,
) -> Result<(), GlesError> {
    let sync = {
        let mut target = renderer.bind(texture)?;
        // Match the window target so existing gl_FragCoord rounded masks stay aligned.
        let mut frame = renderer.render(&mut target, size, Transform::Flipped180)?;
        draw(&mut frame)?;
        frame.finish()?
    };
    renderer.wait(&sync)
}

fn draw_tree(frame: &mut GlesFrame<'_, '_>, elements: &[SceneElement]) -> Result<(), GlesError> {
    for element in elements.iter().rev() {
        let geometry = element.geometry(1.0.into());
        element.draw(
            frame,
            element.src(),
            geometry,
            &[Rectangle::from_size(geometry.size)],
            &[],
            None,
        )?;
    }
    Ok(())
}

fn tree_bounds(elements: &[SceneElement]) -> Option<Rectangle<i32, Physical>> {
    elements
        .iter()
        .map(|element| element.geometry(1.0.into()))
        .reduce(|a, b| a.merge(b))
}

/// Only trust declared opaque regions; never infer opacity from rectangular bounds.
fn tree_is_opaque(elements: &[SceneElement], area: Rectangle<i32, Physical>) -> bool {
    let regions = elements.iter().flat_map(|element| {
        let location = element.geometry(1.0.into()).loc;
        element
            .opaque_regions(1.0.into())
            .into_iter()
            .map(move |region| Rectangle::new(region.loc + location, region.size))
    });
    Rectangle::subtract_rects_many_in_place(vec![area], regions).is_empty()
}

fn draw_texture(
    frame: &mut GlesFrame<'_, '_>,
    texture: &GlesTexture,
    size: Size<i32, Physical>,
    area: Rectangle<i32, Physical>,
    program: Option<&GlesTexProgram>,
    uniforms: &[Uniform<'_>],
) -> Result<(), GlesError> {
    let full = Rectangle::from_size(size);
    // Here opaque_regions means disable blending, not that the stored alpha is one.
    // Every pass writes a complete result for its damage region.
    frame.render_texture_from_to(
        texture,
        Rectangle::from_size((size.w as f64, size.h as f64).into()),
        full,
        &[area],
        &[full],
        Transform::Flipped180,
        1.0,
        program,
        uniforms,
    )
}

/// Smithay owns unit zero. Preserve both auxiliary units and the active selector,
/// including when drawing fails, before any subsequent Smithay operation.
fn with_backdrops(
    frame: &mut GlesFrame<'_, '_>,
    blurred: &GlesTexture,
    original: &GlesTexture,
    draw: impl FnOnce(&mut GlesFrame<'_, '_>) -> Result<(), GlesError>,
) -> Result<(), GlesError> {
    let previous = frame.with_context(|gl| unsafe {
        let mut active = 0;
        let mut bindings = [0; 2];
        gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut active);
        for (i, texture) in [blurred, original].into_iter().enumerate() {
            gl.ActiveTexture(ffi::TEXTURE1 + i as u32);
            gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut bindings[i]);
            gl.BindTexture(ffi::TEXTURE_2D, texture.tex_id());
        }
        gl.ActiveTexture(active as u32);
        (active, bindings)
    })?;
    let result = draw(frame);
    frame.with_context(|gl| unsafe {
        for (i, binding) in previous.1.into_iter().enumerate() {
            gl.ActiveTexture(ffi::TEXTURE1 + i as u32);
            gl.BindTexture(ffi::TEXTURE_2D, binding as u32);
        }
        gl.ActiveTexture(previous.0 as u32);
    })?;
    result
}

struct Gaussian {
    radius: f32,
    coefficient: f32,
    coefficient_step: f32,
}

impl Gaussian {
    fn new(radius: f32) -> Self {
        assert!(
            radius.is_finite() && radius > 0.0 && radius <= 32.0,
            "validated nonzero blur radius"
        );
        let sigma = (radius / 3.0).max(0.5);
        let falloff = 0.5 / (sigma * sigma);
        Self {
            radius,
            coefficient: (-falloff).exp(),
            coefficient_step: (-2.0 * falloff).exp(),
        }
    }

    fn support(&self) -> i32 {
        self.radius.ceil() as i32
    }
}

fn vertical_support(
    area: Rectangle<i32, Physical>,
    clip: Rectangle<i32, Physical>,
    radius: i32,
) -> Rectangle<i32, Physical> {
    Rectangle::new(
        (area.loc.x, area.loc.y - radius).into(),
        (area.size.w, area.size.h + 2 * radius).into(),
    )
    .intersection(clip)
    .expect("area intersects its clip")
}

/// Normalized texel-center bounds in the flipped offscreen storage coordinates.
fn sample_bounds(clip: Rectangle<i32, Physical>, size: Size<i32, Physical>) -> [f32; 4] {
    [
        (clip.loc.x as f32 + 0.5) / size.w as f32,
        ((size.h - clip.loc.y - clip.size.h) as f32 + 0.5) / size.h as f32,
        ((clip.loc.x + clip.size.w) as f32 - 0.5) / size.w as f32,
        ((size.h - clip.loc.y) as f32 - 0.5) / size.h as f32,
    ]
}

const GAUSSIAN: &str = r#"
uniform vec2 direction;
uniform vec4 sample_bounds;
uniform float radius;
uniform float coefficient;
uniform float coefficient_step;
vec4 sample_scene(vec2 p) {
    return texture2D(tex, clamp(p, sample_bounds.xy, sample_bounds.zw));
}
void main() {
    vec4 color = sample_scene(v_coords);
    float total = 1.0;
    float weight = 1.0;
    float step_weight = coefficient;
    // A compile-time loop bound works on GLES2; at most 65 samples per pass.
    for (int i = 1; i <= 32; ++i) {
        if (float(i) < radius + 1.0) {
            weight *= step_weight;
            step_weight *= coefficient_step;
            float w = weight * min(1.0, radius + 1.0 - float(i));
            vec2 offset = direction * float(i);
            color += w * (sample_scene(v_coords - offset) + sample_scene(v_coords + offset));
            total += 2.0 * w;
        }
    }
    gl_FragColor = color / total;
}
"#;

const COMPOSITE: &str = r#"
uniform sampler2D backdrop;
uniform sampler2D original;
uniform float client_shape;
void main() {
    vec4 foreground = texture2D(tex, v_coords);
    float a = foreground.a;
    // Without an explicit blur region, alpha is the only available shape signal.
    // Continuous A*(1-A) blur weight avoids frosting holes or hard shadow edges.
    float tree_coverage = a + a * (1.0 - a);
    float c = coverage(desktop_point(), outline, radii);
    // Square clients retain their existing overflow clipping; outside the frame,
    // use their alpha shape rather than inventing a larger frosted rectangle.
    if (client_shape > 1.5) c = max(c, tree_coverage);
    else if (client_shape > 0.5) c = tree_coverage;
    c = max(c, a); // UNORM rounding can put stored alpha just above analytic coverage.
    gl_FragColor = foreground + (c - a) * texture2D(backdrop, v_coords)
        + (1.0 - c) * texture2D(original, v_coords);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kawase_pyramid_ceil_halves_and_stops_at_one() {
        let sizes = |w, h, passes| {
            pyramid_sizes((w, h).into(), passes)
                .iter()
                .map(|s| (s.w, s.h))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            sizes(17, 9, 6),
            [(17, 9), (9, 5), (5, 3), (3, 2), (2, 1), (1, 1)]
        );
        assert_eq!(sizes(17, 9, 1), [(17, 9), (9, 5)]);
        assert_eq!(sizes(1, 7, 6), [(1, 7), (1, 4), (1, 2), (1, 1)]);
        assert_eq!(sizes(7, 1, 6), [(7, 1), (4, 1), (2, 1), (1, 1)]);
        assert_eq!(sizes(1, 1, 6), [(1, 1)]);
        assert_eq!(sizes(2, 2, 1), sizes(2, 2, 6));
        assert_eq!(sizes(1920, 1080, 6).len(), 7);
    }

    #[test]
    fn kawase_pyramid_fits_cache_pixel_budget_even_for_skinny_odd_clips() {
        for w in 1..=129 {
            for h in 1..=129 {
                let size = (w, h).into();
                let sizes = pyramid_sizes(size, 6);
                assert!(pyramid_pixels(&sizes) <= 3 * pixel_count(size));
                assert!(sizes.iter().all(|s| s.w >= 1 && s.h >= 1));
                assert!(sizes.windows(2).all(|s| s[0] != s[1]));
            }
        }
    }

    #[test]
    fn kawase_crop_uses_storage_origin_and_each_level_has_local_center_bounds() {
        let frame = (97, 71).into();
        let clip = Rectangle::new((23, 11).into(), (17, 9).into());
        let stored = storage_rect(clip, frame);
        assert_eq!(
            stored,
            Rectangle::new((23.0, 51.0).into(), (17.0, 9.0).into())
        );
        let bounds = sample_bounds(clip, frame);
        assert_eq!(bounds, [23.5 / 97.0, 51.5 / 71.0, 39.5 / 97.0, 59.5 / 71.0]);
        for size in pyramid_sizes(clip.size, 6) {
            let bounds = sample_bounds(Rectangle::from_size(size), size);
            assert_eq!(
                bounds,
                [
                    0.5 / size.w as f32,
                    0.5 / size.h as f32,
                    1.0 - 0.5 / size.w as f32,
                    1.0 - 0.5 / size.h as f32
                ]
            );
            assert_eq!(
                storage_rect(Rectangle::from_size(size), size).loc,
                (0.0, 0.0).into()
            );
        }
    }

    #[test]
    fn kawase_kernels_are_normalized_symmetric_and_use_source_texel_offsets() {
        for (taps, total, moment) in [(KAWASE_DOWN, 8.0, 0.5), (KAWASE_UP, 12.0, 4.0 / 3.0)] {
            assert_eq!(taps.iter().map(|t| t[2]).sum::<f32>(), total);
            assert!(taps.iter().all(|&[x, y, w]| taps.contains(&[-x, -y, w])));
            for axis in 0..2 {
                assert_eq!(taps.iter().map(|t| t[axis] * t[2]).sum::<f32>(), 0.0);
                let variance = taps.iter().map(|t| t[axis].powi(2) * t[2]).sum::<f32>() / total;
                assert!((variance - moment).abs() < 1e-6);
            }
            let shader = kawase_shader(taps);
            assert_eq!(shader.matches("color +=").count(), taps.len());
            assert!(shader.contains(&format!("color / {total:.1}")));
            assert!(shader.contains("clamp(p, sample_bounds.xy, sample_bounds.zw)"));
        }
    }

    // Scalar CPU reference, top-left row order. GPU storage is bottom-up, but both
    // kernels are y-symmetric. Bilinear sampling clamps to centers BEFORE lookup.
    fn cpu_kawase(
        pixels: &[f64],
        source: (usize, usize),
        target: (usize, usize),
        radius: f64,
        taps: &[[f32; 3]],
    ) -> Vec<f64> {
        let mut result = Vec::new();
        let total = taps.iter().map(|t| f64::from(t[2])).sum::<f64>();
        for y in 0..target.1 {
            for x in 0..target.0 {
                let mut value = 0.0;
                for &[dx, dy, weight] in taps {
                    let sx = ((x as f64 + 0.5) * source.0 as f64 / target.0 as f64 - 0.5
                        + f64::from(dx) * radius)
                        .clamp(0.0, (source.0 - 1) as f64);
                    let sy = ((y as f64 + 0.5) * source.1 as f64 / target.1 as f64 - 0.5
                        + f64::from(dy) * radius)
                        .clamp(0.0, (source.1 - 1) as f64);
                    let (x0, y0) = (sx.floor() as usize, sy.floor() as usize);
                    let (x1, y1) = ((x0 + 1).min(source.0 - 1), (y0 + 1).min(source.1 - 1));
                    let (fx, fy) = (sx.fract(), sy.fract());
                    let a =
                        pixels[y0 * source.0 + x0] * (1.0 - fx) + pixels[y0 * source.0 + x1] * fx;
                    let b =
                        pixels[y1 * source.0 + x0] * (1.0 - fx) + pixels[y1 * source.0 + x1] * fx;
                    value += ((1.0 - fy) * a + fy * b) * f64::from(weight) / total;
                }
                result.push(value);
            }
        }
        result
    }

    #[test]
    fn kawase_pixel_centers_use_actual_odd_size_ratios_not_fixed_two() {
        let ramp = (0..9)
            .flat_map(|y| (0..17).map(move |x| f64::from(x + 2 * y)))
            .collect::<Vec<_>>();
        let down = cpu_kawase(&ramp, (17, 9), (9, 5), 0.25, KAWASE_DOWN);
        assert!((down[2 * 9 + 4] - 16.0).abs() < 1e-9);
        let expected = (3.5 * 17.0 / 9.0 - 0.5) + 2.0 * (1.5 * 9.0 / 5.0 - 0.5);
        assert!((down[9 + 3] - expected).abs() < 1e-9);
        let up = cpu_kawase(&down, (9, 5), (17, 9), 0.25, KAWASE_UP);
        assert!((up[4 * 17 + 8] - 16.0).abs() < 1e-9);
        let pair = cpu_kawase(&[0.0, 1.0], (2, 1), (1, 1), 0.0, KAWASE_DOWN);
        assert_eq!(pair, [0.5]);
        // All taps collapse onto the sole source texel, even at the maximum radius.
        assert!(
            cpu_kawase(&[0.375], (1, 1), (3, 2), 32.0, KAWASE_UP)
                .iter()
                .all(|value| (value - 0.375).abs() < 1e-9)
        );
    }

    #[test]
    fn kawase_every_level_preserves_constants_and_clamps_large_offsets() {
        for size in [(1, 1), (1, 7), (17, 9), (9, 1)] {
            let sizes = pyramid_sizes(size.into(), 6)
                .iter()
                .map(|s| (s.w as usize, s.h as usize))
                .collect::<Vec<_>>();
            for radius in [0.01, 0.5, 12.0, 32.0] {
                let mut pixels = vec![0.375; sizes[0].0 * sizes[0].1];
                for levels in sizes.windows(2) {
                    pixels = cpu_kawase(&pixels, levels[0], levels[1], radius, KAWASE_DOWN);
                }
                for levels in sizes.windows(2).rev() {
                    pixels = cpu_kawase(&pixels, levels[1], levels[0], radius, KAWASE_UP);
                }
                assert!(pixels.iter().all(|v| (v - 0.375).abs() < 1e-9));
            }
        }
        assert_eq!(
            cpu_kawase(&[0.0, 0.0, 1.0], (3, 1), (2, 1), 32.0, KAWASE_DOWN),
            [0.25, 0.625]
        );
    }

    #[test]
    fn opaque_skip_uses_regions_not_tree_bounds() {
        use smithay::backend::renderer::element::solid::{
            SolidColorBuffer, SolidColorRenderElement,
        };
        let element = |x, width, alpha| {
            SceneElement::Border(SolidColorRenderElement::from_buffer(
                &SolidColorBuffer::new((width, 20), [0.0, 0.0, 0.0, alpha]),
                (x, 10),
                1.0,
                1.0,
                Kind::Unspecified,
            ))
        };
        let area = Rectangle::new((10, 10).into(), (20, 20).into());
        assert!(tree_is_opaque(
            &[element(10, 10, 1.0), element(20, 10, 1.0)],
            area
        ));
        assert!(!tree_is_opaque(
            &[element(10, 9, 1.0), element(20, 10, 1.0)],
            area
        ));
        let translucent = [element(10, 20, 0.5)];
        assert_eq!(tree_bounds(&translucent), Some(area));
        assert!(!tree_is_opaque(&translucent, area));
        assert!(!tree_is_opaque(&[], area));
    }

    #[test]
    fn gaussian_is_bounded_symmetric_and_normalized() {
        for radius in [0.01, 0.5, 1.0, 12.0, 31.5, 32.0] {
            let kernel = Gaussian::new(radius);
            let mut weights = vec![1.0];
            let mut weight = 1.0;
            let mut step = kernel.coefficient;
            for i in 1..=kernel.support() {
                weight *= step;
                step *= kernel.coefficient_step;
                let w = weight * (radius + 1.0 - i as f32).min(1.0);
                let sigma = (radius / 3.0).max(0.5);
                let expected =
                    (-0.5 * (i as f32 / sigma).powi(2)).exp() * (radius + 1.0 - i as f32).min(1.0);
                assert!((w - expected).abs() < 0.00001);
                weights.push(w);
            }
            assert!(weights.len() <= 33);
            let sum = weights[0] + 2.0 * weights[1..].iter().sum::<f32>();
            assert!(
                (weights[0] / sum + 2.0 * weights[1..].iter().map(|w| w / sum).sum::<f32>() - 1.0)
                    .abs()
                    < 0.00001
            );
        }
    }

    #[test]
    fn adjacent_outputs_clamp_to_their_own_texel_centers() {
        let size = (200, 100).into();
        assert_eq!(
            sample_bounds(Rectangle::new((0, 0).into(), (100, 100).into()), size),
            [0.0025, 0.005, 0.4975, 0.995]
        );
        assert_eq!(
            sample_bounds(Rectangle::new((100, 20).into(), (100, 60).into()), size),
            [0.5025, 0.205, 0.9975, 0.795]
        );
        let one = sample_bounds(Rectangle::new((99, 50).into(), (1, 1).into()), size);
        assert_eq!(one[0], one[2]);
        assert_eq!(one[1], one[3]);
    }

    #[test]
    fn vertical_pass_only_reads_initialized_horizontal_rows() {
        let clip = Rectangle::new((100, 20).into(), (100, 60).into());
        let area = Rectangle::new((120, 22).into(), (10, 10).into());
        assert_eq!(
            vertical_support(area, clip, 12),
            Rectangle::new((120, 20).into(), (10, 24).into())
        );
    }

    #[test]
    fn coverage_is_not_masked_alpha_and_client_holes_remain_holes() {
        let compose = |f: f32, a: f32, c: f32, blurred: f32, original: f32| {
            f + (c - a) * blurred + (1.0 - c) * original
        };
        assert_eq!(compose(0.0, 0.0, 0.0, 0.8, 0.2), 0.2);
        assert_eq!(compose(0.4, 1.0, 1.0, 0.8, 0.2), 0.4);
        // Quarter-covered, half-transparent window: A=C*clientA, not C itself.
        assert!((compose(0.05, 0.125, 0.25, 0.8, 0.2) - 0.3).abs() < 0.00001);
        for a in [0.0_f32, 0.001, 0.25, 0.5, 1.0] {
            let c = a + a * (1.0 - a);
            assert!((a..=1.0).contains(&c));
            assert!(((c - a) - a * (1.0 - a)).abs() < 0.00001);
            assert!((compose(a, a, c, 1.0, 1.0) - 1.0).abs() < 0.00001);
        }
    }
}
