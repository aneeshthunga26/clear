//! Rounded window bodies and border rings. Layer-shell surfaces and popups bypass this.

use crate::{core::Rect, decoration::Theme};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, Frame, Offscreen, Renderer,
            element::{
                Element, Id, Kind, RenderElement, surface::WaylandSurfaceRenderElement,
                texture::TextureRenderElement, utils::CropRenderElement,
            },
            gles::{
                GlesError, GlesRenderer, GlesTexProgram, GlesTexture, Uniform, UniformName,
                UniformType, element::TextureShaderElement,
            },
        },
    },
    utils::{Logical, Physical, Point, Rectangle, Transform},
};

/// An unclipped shape in desktop coordinates; clipping never creates new rounded corners.
#[derive(Debug, Clone, Copy)]
pub(super) struct RoundedShape {
    pub rect: Rect,
    pub radii: [f32; 4],
}

impl RoundedShape {
    fn new(rect: Rect, radii: [f32; 4]) -> Self {
        // Uniformly scale all radii when neighboring arcs would overlap (CSS rule).
        let mut factor = 1.0_f32;
        for (extent, sum) in [
            (rect.width, radii[0] + radii[1]),
            (rect.width, radii[3] + radii[2]),
            (rect.height, radii[0] + radii[3]),
            (rect.height, radii[1] + radii[2]),
        ] {
            if sum > 0.0 {
                factor = factor.min(extent.max(0) as f32 / sum);
            }
        }
        Self {
            rect,
            radii: radii.map(|radius| radius * factor),
        }
    }

    /// Same arc test as the shader, at its 50% coverage boundary.
    pub fn contains(self, point: Point<f64, Logical>) -> bool {
        let x = point.x - f64::from(self.rect.x);
        let y = point.y - f64::from(self.rect.y);
        let w = f64::from(self.rect.width);
        let h = f64::from(self.rect.height);
        if x < 0.0 || y < 0.0 || x >= w || y >= h {
            return false;
        }
        for (cx, cy, radius, in_corner) in [
            (
                0.0,
                0.0,
                self.radii[0],
                x < f64::from(self.radii[0]) && y < f64::from(self.radii[0]),
            ),
            (
                w,
                0.0,
                self.radii[1],
                x > w - f64::from(self.radii[1]) && y < f64::from(self.radii[1]),
            ),
            (
                w,
                h,
                self.radii[2],
                x > w - f64::from(self.radii[2]) && y > h - f64::from(self.radii[2]),
            ),
            (
                0.0,
                h,
                self.radii[3],
                x < f64::from(self.radii[3]) && y > h - f64::from(self.radii[3]),
            ),
        ] {
            if in_corner {
                let r = f64::from(radius);
                let dx = (x - cx).abs() - r;
                let dy = (y - cy).abs() - r;
                // Opposite corner regions can overlap even after adjacent radii fit.
                if dx * dx + dy * dy > r * r {
                    return false;
                }
            }
        }
        true
    }

    pub(super) fn uniforms(self, target_height: i32) -> Vec<Uniform<'static>> {
        vec![
            Uniform::new(
                "outline",
                [
                    self.rect.x as f32,
                    self.rect.y as f32,
                    self.rect.width as f32,
                    self.rect.height as f32,
                ],
            ),
            Uniform::new("radii", self.radii),
            Uniform::new("target_height", target_height as f32),
        ]
    }
}

/// Both masks derive from the same normalized outer radii, preserving border thickness.
#[derive(Debug, Clone, Copy)]
pub(super) struct WindowOutline {
    pub outer: RoundedShape,
    pub inner: RoundedShape,
}

impl WindowOutline {
    pub fn new(rect: Rect, theme: &Theme) -> Self {
        let b = theme.border_width;
        let outer = RoundedShape::new(
            Rect::new(
                rect.x.saturating_sub(b),
                rect.y.saturating_sub(b),
                rect.width.saturating_add(2 * b),
                rect.height.saturating_add(2 * b),
            ),
            theme.corner_radius.values(),
        );
        let inner = RoundedShape::new(rect, outer.radii.map(|r| (r - b as f32).max(0.0)));
        Self { outer, inner }
    }

    /// Map an already fitted desktop outline into a scaled destination.
    pub fn scaled(self, dest: Rect) -> Self {
        let source = self.outer.rect;
        let sx = dest.width as f32 / source.width.max(1) as f32;
        let sy = dest.height as f32 / source.height.max(1) as f32;
        let map = |shape: RoundedShape| RoundedShape {
            rect: Rect::new(
                dest.x + ((shape.rect.x - source.x) as f32 * sx).round() as i32,
                dest.y + ((shape.rect.y - source.y) as f32 * sy).round() as i32,
                (shape.rect.width as f32 * sx).round().max(1.0) as i32,
                (shape.rect.height as f32 * sy).round().max(1.0) as i32,
            ),
            radii: shape.radii.map(|r| r * sx.min(sy)),
        };
        Self {
            outer: map(self.outer),
            inner: map(self.inner),
        }
    }

    /// Normal offscreen targets store logical row zero at GL row zero. Reflect
    /// geometry for the desktop shader's opposite, top-left coordinate system.
    pub fn offscreen(self, height: i32) -> Self {
        let map = |shape: RoundedShape| RoundedShape {
            rect: Rect::new(
                shape.rect.x,
                height - shape.rect.bottom(),
                shape.rect.width,
                shape.rect.height,
            ),
            radii: [
                shape.radii[3],
                shape.radii[2],
                shape.radii[1],
                shape.radii[0],
            ],
        };
        Self {
            outer: map(self.outer),
            inner: map(self.inner),
        }
    }

    /// A separate preview selection ring follows the scaled source silhouette.
    pub fn selection(self, thickness: i32) -> Self {
        let outer = self.outer;
        let b = thickness.clamp(0, (outer.rect.width.min(outer.rect.height) - 1).max(0) / 2);
        let inner = RoundedShape::new(
            Rect::new(
                outer.rect.x + b,
                outer.rect.y + b,
                outer.rect.width - 2 * b,
                outer.rect.height - 2 * b,
            ),
            outer.radii.map(|r| (r - b as f32).max(0.0)),
        );
        Self { outer, inner }
    }
}

/// Compiled once per renderer, on first use; reload only changes uniforms.
pub(super) struct RoundedShaders {
    window: GlesTexProgram,
}

pub(super) type RoundedSurface = TextureShaderElement;

impl RoundedShaders {
    pub fn new(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        let window = renderer.compile_custom_texture_shader(
            format!("{TEXTURE_HEADER}\n{SHAPE}\n{TEXTURE_MAIN}"),
            &[
                UniformName::new("outline", UniformType::_4f),
                UniformName::new("radii", UniformType::_4f),
                UniformName::new("target_height", UniformType::_1f),
                UniformName::new("inner_outline", UniformType::_4f),
                UniformName::new("inner_radii", UniformType::_4f),
                UniformName::new("border_color", UniformType::_4f),
                UniformName::new("body_alpha", UniformType::_1f),
            ],
        )?;
        Ok(Self { window })
    }

    /// Composite the unmasked surface tree, then mask its body and add the border once.
    pub fn window(
        &self,
        renderer: &mut GlesRenderer,
        surfaces: Vec<WaylandSurfaceRenderElement<GlesRenderer>>,
        content: Rect,
        titlebar: Vec<TextureRenderElement<GlesTexture>>,
        outline: WindowOutline,
        color: [f32; 4],
        height: i32,
        clip: Rectangle<i32, Physical>,
    ) -> Result<Option<RoundedSurface>, GlesError> {
        let Some(area) = super::scene::logical(outline.outer.rect)
            .to_physical_precise_round(1.0)
            .intersection(clip)
        else {
            return Ok(None);
        };
        let body_clip = super::scene::logical(content)
            .to_physical_precise_round(1.0)
            .intersection(area);
        // Bound allocations to the visible output clip, not potentially huge saved geometry.
        let mut texture: GlesTexture =
            renderer.create_buffer(Fourcc::Abgr8888, (area.size.w, area.size.h).into())?;
        let sync =
            {
                let mut target = renderer.bind(&mut texture)?;
                // Normal stores logical row zero at texture v=0. The final window target
                // uses Flipped180; applying that here too would invert the sampled body.
                let mut frame = renderer.render(&mut target, area.size, Transform::Normal)?;
                frame.clear([0.0; 4].into(), &[Rectangle::from_size(area.size)])?;
                if let Some(body_clip) = body_clip {
                    // Trees are front-to-back. Blend them back-to-front without rounding
                    // so covered parents cannot leak into their children's antialiased edges.
                    for surface in surfaces.into_iter().rev().filter_map(|surface| {
                        CropRenderElement::from_element(surface, 1.0, body_clip)
                    }) {
                        let geometry = surface.geometry(1.0.into());
                        let destination = Rectangle::new(geometry.loc - area.loc, geometry.size);
                        surface.draw(
                            &mut frame,
                            surface.src(),
                            destination,
                            &[Rectangle::from_size(geometry.size)],
                            &[],
                            None,
                        )?;
                    }
                }
                for bar in titlebar
                    .into_iter()
                    .filter_map(|bar| CropRenderElement::from_element(bar, 1.0, area))
                {
                    let geometry = bar.geometry(1.0.into());
                    RenderElement::<GlesRenderer>::draw(
                        &bar,
                        &mut frame,
                        bar.src(),
                        Rectangle::new(geometry.loc - area.loc, geometry.size),
                        &[Rectangle::from_size(geometry.size)],
                        &[],
                        None,
                    )?;
                }
                frame.finish()?
            };
        // Use a renderer-side wait before sampling, without forcing a CPU readback.
        renderer.wait(&sync)?;
        let body = TextureRenderElement::from_static_texture(
            Id::new(),
            renderer.context_id(),
            area.loc.to_f64(),
            texture,
            1,
            Transform::Normal,
            None,
            None,
            None,
            None,
            Kind::Unspecified,
        );
        Ok(Some(self.mask_texture(body, outline, color, height)))
    }

    /// Apply the ordinary outline to an already composited bounded thumbnail.
    pub(super) fn mask_texture(
        &self,
        body: TextureRenderElement<GlesTexture>,
        outline: WindowOutline,
        color: [f32; 4],
        height: i32,
    ) -> RoundedSurface {
        self.texture_element(body, outline, color, height, 1.0)
    }

    /// Draw only an accent ring; do not remask the already rounded client image.
    pub(super) fn outline_texture(
        &self,
        body: TextureRenderElement<GlesTexture>,
        outline: WindowOutline,
        color: [f32; 4],
        height: i32,
    ) -> RoundedSurface {
        self.texture_element(body, outline, color, height, 0.0)
    }

    fn texture_element(
        &self,
        body: TextureRenderElement<GlesTexture>,
        outline: WindowOutline,
        color: [f32; 4],
        height: i32,
        body_alpha: f32,
    ) -> RoundedSurface {
        let mut uniforms = outline.outer.uniforms(height);
        let rect = outline.inner.rect;
        uniforms.extend([
            Uniform::new(
                "inner_outline",
                [
                    rect.x as f32,
                    rect.y as f32,
                    rect.width as f32,
                    rect.height as f32,
                ],
            ),
            Uniform::new("inner_radii", outline.inner.radii),
            Uniform::new("body_alpha", body_alpha),
            Uniform::new("border_color", super::scene::premultiply(color)),
        ]);
        // Fresh element IDs damage the entire composited clip, including uniform-only
        // changes on reload. No opaque region may include the transparent cut-outs.
        TextureShaderElement::new(body, self.window.clone(), uniforms)
    }
}

// gl_FragCoord keeps masks aligned across cropped subsurfaces, buffer transforms,
// and output-group clips. The nested backend renders at scale 1 to a single target,
// with bottom-up GL coordinates and a top-left logical desktop origin.
pub(super) const SHAPE: &str = r#"
uniform vec4 outline;
uniform vec4 radii;
uniform float target_height;
float shape_distance(vec2 point, vec4 box, vec4 r) {
    vec2 p = point - box.xy;
    vec2 s = box.zw;
    float d = min(min(p.x, s.x - p.x), min(p.y, s.y - p.y));
    if (p.x < r.x && p.y < r.x)
        d = min(d, r.x - length(p - vec2(r.x)));
    if (p.x > s.x - r.y && p.y < r.y)
        d = min(d, r.y - length(p - vec2(s.x - r.y, r.y)));
    if (p.x > s.x - r.z && p.y > s.y - r.z)
        d = min(d, r.z - length(p - vec2(s.x - r.z, s.y - r.z)));
    if (p.x < r.w && p.y > s.y - r.w)
        d = min(d, r.w - length(p - vec2(r.w, s.y - r.w)));
    return d;
}
float coverage(vec2 point, vec4 box, vec4 r) {
    return smoothstep(-0.5, 0.5, shape_distance(point, box, r));
}
vec2 desktop_point() { return vec2(gl_FragCoord.x, target_height - gl_FragCoord.y); }
"#;

pub(super) const TEXTURE_HEADER: &str = r#"#version 100
//_DEFINES_
#ifdef EXTERNAL
#extension GL_OES_EGL_image_external : require
#endif
precision highp float;
#ifdef EXTERNAL
uniform samplerExternalOES tex;
#else
uniform sampler2D tex;
#endif
uniform float alpha;
varying vec2 v_coords;
#ifdef DEBUG_FLAGS
uniform float tint;
#endif
"#;

const TEXTURE_MAIN: &str = r#"
uniform vec4 inner_outline;
uniform vec4 inner_radii;
uniform vec4 border_color;
uniform float body_alpha;
void main() {
    vec2 p = desktop_point();
    float inside = coverage(p, inner_outline, inner_radii);
    float ring = max(0.0, coverage(p, outline, radii) - inside);
    // Body and ring occupy disjoint coverage, not source-over layers. Add their
    // premultiplied contributions before blending over the rest of the scene.
    vec4 color = (texture2D(tex, v_coords) * inside * body_alpha + border_color * ring) * alpha;
#ifdef DEBUG_FLAGS
    if (tint == 1.0) color = vec4(0.0, 0.2, 0.0, 0.2) + color * 0.8;
#endif
    gl_FragColor = color;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asymmetric_arcs_and_square_corners_have_matching_hit_regions() {
        let shape = RoundedShape::new(Rect::new(100, 200, 200, 120), [40.0, 0.0, 20.0, 10.0]);
        assert!(!shape.contains((101.0, 201.0).into()));
        assert!(shape.contains((299.0, 201.0).into()));
        assert!(!shape.contains((299.0, 319.0).into()));
        assert!(!shape.contains((101.0, 319.0).into()));
        assert!(shape.contains((140.0, 200.0).into()));
        assert!(shape.contains((200.0, 250.0).into()));
        assert!(!shape.contains((300.0, 250.0).into()));
        assert!(!shape.contains((200.0, 320.0).into()));
    }

    #[test]
    fn opposing_corner_regions_require_both_arc_constraints() {
        let rect = Rect::new(10, 20, 100, 100);
        for radii in [[100.0, 0.0, 100.0, 0.0], [0.0, 100.0, 0.0, 100.0]] {
            let shape = RoundedShape::new(rect, radii);
            assert_eq!(shape.radii, radii);
            assert!(shape.contains((60.0, 70.0).into()));
            // Every interior pixel is in both opposite corner regions. The result
            // must be the intersection of the disks, not whichever is checked first.
            for y in 0..100 {
                for x in 0..100 {
                    let x = f64::from(x) + 0.5;
                    let y = f64::from(y) + 0.5;
                    let expected = if radii[0] > 0.0 {
                        (x - 100.0).hypot(y - 100.0) <= 100.0 && x.hypot(y) <= 100.0
                    } else {
                        x.hypot(y - 100.0) <= 100.0 && (x - 100.0).hypot(y) <= 100.0
                    };
                    assert_eq!(
                        shape.contains((x + 10.0, y + 20.0).into()),
                        expected,
                        "radii={radii:?}, point=({x}, {y})",
                    );
                }
            }
        }
    }

    #[test]
    fn oversized_opposing_radii_still_mask_both_corners_after_fitting() {
        let shape = RoundedShape::new(Rect::new(0, 0, 100, 100), [256.0, 0.0, 256.0, 0.0]);
        assert_eq!(shape.radii, [100.0, 0.0, 100.0, 0.0]);
        assert!(!shape.contains((0.5, 0.5).into()));
        assert!(!shape.contains((99.5, 99.5).into()));
        assert!(shape.contains((50.0, 50.0).into()));
    }

    #[test]
    fn large_radii_scale_proportionally_without_overlapping_or_moving_rect() {
        let rect = Rect::new(-100, -200, 100, 60);
        let shape = RoundedShape::new(rect, [80.0, 40.0, 20.0, 40.0]);
        assert_eq!(shape.rect, rect);
        assert_eq!(shape.radii, [40.0, 20.0, 10.0, 20.0]);
        assert!(!shape.contains((-99.0, -199.0).into()));
        assert!(shape.contains((-50.0, -170.0).into()));
    }

    #[test]
    fn scaled_outlines_preserve_source_arcs_insets_and_offscreen_corner_order() {
        let config = crate::config::Config::from_source(
            "[theme]\nborder_width=4\ncorner_radius=[28,0,14,20]",
        )
        .unwrap();
        let source = WindowOutline::new(Rect::new(0, 0, 392, 292), &config.theme);
        let preview = source.scaled(Rect::new(100, 200, 200, 150));
        assert_eq!(preview.outer.radii, [14.0, 0.0, 7.0, 10.0]);
        assert_eq!(preview.inner.rect, Rect::new(102, 202, 196, 146));
        assert_eq!(preview.inner.radii, [12.0, 0.0, 5.0, 8.0]);
        let mini = source.scaled(Rect::new(0, 0, 20, 15));
        assert_eq!(mini.outer.radii, [1.4, 0.0, 0.7, 1.0]);
        let offscreen = source.scaled(Rect::new(0, 0, 400, 300)).offscreen(300);
        assert_eq!(offscreen.outer.radii, [20.0, 14.0, 0.0, 28.0]);
        assert_eq!(offscreen.inner.radii, [16.0, 10.0, 0.0, 24.0]);
        assert_eq!(offscreen.inner.rect, Rect::new(4, 4, 392, 292));
        let selection = preview.selection(2);
        assert_eq!(selection.outer.radii, preview.outer.radii);
        assert_eq!(selection.inner.radii, [12.0, 0.0, 5.0, 8.0]);
        assert_eq!(selection.inner.rect, Rect::new(102, 202, 196, 146));
        let tiny = source.scaled(Rect::new(0, 0, 1, 1)).selection(2);
        assert_eq!(tiny.inner.rect, tiny.outer.rect);
    }

    #[test]
    fn inner_shape_subtracts_border_and_zero_border_keeps_same_shape() {
        let config =
            crate::config::Config::from_source("[theme]\nborder_width=4\ncorner_radius=[20, 8]")
                .unwrap();
        let rect = Rect::new(12, 30, 200, 100);
        let outline = WindowOutline::new(rect, &config.theme);
        assert_eq!(outline.outer.rect, Rect::new(8, 26, 208, 108));
        assert_eq!(outline.outer.radii, [20.0, 20.0, 8.0, 8.0]);
        assert_eq!(outline.inner.radii, [16.0, 16.0, 4.0, 4.0]);
        assert_eq!(outline.inner.rect, rect);
        let mut theme = config.theme;
        theme.border_width = 0;
        let outline = WindowOutline::new(rect, &theme);
        assert_eq!(outline.outer.rect, outline.inner.rect);
        assert_eq!(outline.outer.radii, outline.inner.radii);
    }
}
