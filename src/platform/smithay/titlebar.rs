//! Compact SSD rasterization at logical scale 1. The caller clips and rounds the
//! combined client/titlebar body; this module deliberately keeps square corners.

use crate::{
    core::{Rect, WindowId},
    decoration::{ControlsSide, TitlebarTheme},
    runtime::titlebar::{TitlebarAssets, TitlebarImage},
};
use cosmic_text::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap, fontdb,
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem,
            element::{
                Kind,
                texture::{TextureBuffer, TextureRenderElement},
            },
            gles::{GlesError, GlesRenderer, GlesTexture},
        },
    },
    utils::{Logical, Point, Transform},
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

const MAX_BAR_HEIGHT: i32 = 128;
const MAX_TEXTURE_WIDTH: i32 = 4096;
const MAX_TITLE_BYTES: usize = 1024;
const MAX_TEXT_WIDTH: i32 = 1024;
// Both limits apply, excluding outstanding render elements and driver overhead.
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;
const TEXT_INSET: i32 = 12;
const LINE_HEIGHT: i32 = 20;

/// A titlebar action; Maximize means restore when the window is maximized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TitlebarPart {
    Drag,
    Minimize,
    Maximize,
    Close,
}

fn bar_height(frame: Rect, height: i32) -> i32 {
    frame
        .height
        .saturating_sub(1)
        .clamp(0, height.clamp(0, MAX_BAR_HEIGHT))
}

/// Reserve SSD height without consuming the last client row. Empty frames stay
/// empty; undecorated frames are unchanged. This does not clamp saved geometry.
pub(super) fn content_rect(frame: Rect, ssd: bool, height: i32) -> Rect {
    let height = if ssd { bar_height(frame, height) } else { 0 };
    Rect {
        y: frame.y.saturating_add(height),
        height: frame.height - height,
        ..frame
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Style {
    height: i32,
    controls_left: bool,
    show_icon: bool,
    show_title: bool,
    icon_size: i32,
    // Preserve exact style identity without requiring Eq on schema floats.
    colors: [[u32; 4]; 4],
}

impl Style {
    fn new(theme: &TitlebarTheme) -> Self {
        Self {
            height: theme.height,
            controls_left: matches!(theme.controls_side, ControlsSide::Left),
            show_icon: theme.show_icon,
            show_title: theme.show_title,
            icon_size: theme.icon_size().min(MAX_BAR_HEIGHT as u32) as i32,
            colors: [
                theme.active_background,
                theme.inactive_background,
                theme.active_foreground,
                theme.inactive_foreground,
            ]
            .map(|color| color.map(f32::to_bits)),
        }
    }

    fn color(&self, index: usize) -> [u8; 4] {
        self.colors[index]
            .map(|channel| (f32::from_bits(channel).clamp(0.0, 1.0) * 255.0).round() as u8)
    }
}

struct Layout {
    width: i32,
    height: i32,
    // Close has priority on narrow frames, followed by maximize and minimize.
    // All rectangles are titlebar-local, including when only a strip is visible.
    controls: Vec<(TitlebarPart, Rect)>,
    text_left: i32,
    text_right: i32,
    icon: Option<Rect>,
    style: Style,
}

impl Layout {
    fn new(frame: Rect, theme: &TitlebarTheme) -> Option<Self> {
        let height = bar_height(frame, theme.height);
        if frame.width <= 0 || height == 0 {
            return None;
        }
        let style = Style::new(theme);
        let button_width = theme.height.clamp(16, MAX_BAR_HEIGHT);
        let mut remaining = frame.width;
        let mut controls = Vec::with_capacity(3);
        for part in [
            TitlebarPart::Close,
            TitlebarPart::Maximize,
            TitlebarPart::Minimize,
        ] {
            if remaining == 0 || (part != TitlebarPart::Close && remaining < button_width) {
                break;
            }
            let width = remaining.min(button_width);
            let x = if style.controls_left {
                frame.width - remaining
            } else {
                remaining - width
            };
            remaining -= width;
            controls.push((part, Rect::new(x, 0, width, height)));
        }
        let (left, text_right) = if style.controls_left {
            (frame.width - remaining, frame.width)
        } else {
            (0, remaining)
        };
        let mut text_left = left.saturating_add(TEXT_INSET).min(text_right);
        let icon_size = style.icon_size.min(height).min(text_right - text_left);
        let icon = (style.show_icon && icon_size > 0)
            .then(|| Rect::new(text_left, (height - icon_size) / 2, icon_size, icon_size));
        if let Some(icon) = icon {
            text_left = icon.right().saturating_add(TEXT_INSET).min(text_right);
        }
        Some(Self {
            width: frame.width,
            height,
            controls,
            text_left,
            text_right,
            icon,
            style,
        })
    }
}

// Frame-local viewports bound allocation without moving the original controls.
fn viewports(frame: Rect, clip: Rect, height: i32) -> impl Iterator<Item = Rect> {
    let bar = Rect::new(frame.x, frame.y, frame.width, bar_height(frame, height));
    bar.intersection(clip).into_iter().flat_map(move |visible| {
        (0..visible.width)
            .step_by(MAX_TEXTURE_WIDTH as usize)
            .map(move |offset| {
                Rect::new(
                    visible.x - frame.x + offset,
                    visible.y - frame.y,
                    (visible.width - offset).min(MAX_TEXTURE_WIDTH),
                    visible.height,
                )
            })
    })
}

/// Half-open hit boxes shared with rasterization, independent of texture limits.
/// The caller must also apply workspace/output clips and the combined rounded
/// mask. Non-finite points and empty bars return None.
pub(super) fn titlebar_hit(
    frame: Rect,
    point: Point<f64, Logical>,
    theme: &TitlebarTheme,
) -> Option<TitlebarPart> {
    let layout = Layout::new(frame, theme)?;
    let x = point.x - f64::from(frame.x);
    let y = point.y - f64::from(frame.y);
    if !x.is_finite()
        || !y.is_finite()
        || x < 0.0
        || y < 0.0
        || x >= f64::from(layout.width)
        || y >= f64::from(layout.height)
    {
        return None;
    }
    for (part, rect) in layout.controls {
        if x >= f64::from(rect.x) && x < f64::from(rect.right()) {
            return Some(part);
        }
    }
    Some(TitlebarPart::Drag)
}

// Weak identity keeps old allocations from reusing an address while cached,
// without retaining decoded image storage after a reload.
#[derive(Debug, Clone)]
struct ImageKey(Weak<TitlebarImage>);

impl PartialEq for ImageKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.ptr_eq(&other.0)
    }
}
impl Eq for ImageKey {}

fn image_key(image: Option<&Arc<TitlebarImage>>) -> Option<ImageKey> {
    image.map(|image| ImageKey(Arc::downgrade(image)))
}

#[derive(Debug, PartialEq, Eq)]
struct CacheKey {
    title: String,
    width: i32,
    height: i32,
    viewport: Rect,
    focused: bool,
    maximized: bool,
    style: Style,
    app_id: String,
    // Minimize, maximize/restore, close, application icon.
    images: [Option<ImageKey>; 4],
}

impl CacheKey {
    fn new(title: &str, layout: &Layout, viewport: Rect, focused: bool, maximized: bool) -> Self {
        Self {
            title: bounded_title(title),
            width: layout.width,
            height: layout.height,
            viewport,
            focused,
            maximized,
            style: layout.style.clone(),
            app_id: String::new(),
            images: std::array::from_fn(|_| None),
        }
    }

    fn with_assets(mut self, assets: &TitlebarAssets, app_id: &str) -> Self {
        self.app_id = app_id.to_owned();
        self.images = [
            image_key(assets.control("minimize")),
            image_key(if self.maximized {
                assets
                    .control("restore")
                    .or_else(|| assets.control("maximize"))
            } else {
                assets.control("maximize")
            }),
            image_key(assets.control("close")),
            image_key(if self.style.show_icon {
                assets.app_icon(app_id)
            } else {
                None
            }),
        ];
        self
    }

    fn bytes(&self) -> usize {
        self.viewport.width as usize * self.viewport.height as usize * 4
    }
}

fn cache_has_room(entries: usize, bytes: usize, incoming: usize) -> bool {
    entries < MAX_ENTRIES && incoming <= MAX_CACHE_BYTES.saturating_sub(bytes)
}

// Bound scanning as well as shaping and retained strings; never split UTF-8.
fn bounded_title(title: &str) -> String {
    let mut end = title.len().min(MAX_TITLE_BYTES);
    while !title.is_char_boundary(end) {
        end -= 1;
    }
    let truncated = end < title.len();
    if truncated {
        end = end.min(MAX_TITLE_BYTES - '…'.len_utf8());
        while !title.is_char_boundary(end) {
            end -= 1;
        }
    }
    let mut title: String = title[..end]
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                c
            }
        })
        .collect();
    if truncated {
        title.push('…');
    }
    title
}

struct CachedBar {
    key: CacheKey,
    buffer: TextureBuffer<GlesTexture>,
}

/// Renderer-local cache. Construct once per GLES renderer; do not share textures
/// between contexts. Font discovery and file reads happen only in `default`.
pub(super) struct TitlebarCache {
    fonts: FontSystem,
    entries: BTreeMap<(WindowId, (i32, i32, i32, i32)), CachedBar>,
}

impl Default for TitlebarCache {
    fn default() -> Self {
        let (locale, mut db) = FontSystem::new().into_locale_and_db();
        // Replace file sources with owned bytes, including every face in font
        // collections. Cosmic's otherwise-lazy mmap would do I/O during draw.
        let faces: Vec<_> = db.faces().cloned().collect();
        let mut files = BTreeMap::new();
        for mut face in faces {
            let data = match &face.source {
                fontdb::Source::Binary(data) => Some(data.clone()),
                fontdb::Source::SharedFile(_, data) => Some(data.clone()),
                fontdb::Source::File(path) => files
                    .entry(path.clone())
                    .or_insert_with(|| {
                        std::fs::read(path)
                            .ok()
                            .map(|bytes| Arc::new(bytes) as Arc<dyn AsRef<[u8]> + Send + Sync>)
                    })
                    .clone(),
            };
            db.remove_face(face.id);
            if let Some(data) = data {
                face.source = fontdb::Source::Binary(data);
                db.push_face_info(face);
            }
        }
        if db.faces().next().is_none() {
            eprintln!(
                "clear: no titlebar fonts available; rendering window controls without title text"
            );
        }
        Self {
            fonts: FontSystem::new_with_locale_and_db(locale, db),
            entries: BTreeMap::new(),
        }
    }
}

impl TitlebarCache {
    /// Rasterize a bounded overview label with the same prepared fonts and icons.
    pub(super) fn overview_label(
        &mut self,
        text: &str,
        width: i32,
        icon: Option<&TitlebarImage>,
        color: [u8; 4],
    ) -> Vec<u8> {
        let viewport = Rect::new(0, 0, width.clamp(1, 1024), 32);
        let mut pixels = vec![0; (viewport.width * viewport.height * 4) as usize];
        let inset = if let Some(icon) = icon {
            draw_image(&mut pixels, viewport, Rect::new(4, 6, 20, 20), icon);
            32
        } else {
            8
        };
        let text_width = (viewport.width - inset - 8).max(0);
        if text_width > 0 && self.fonts.db().faces().next().is_some() {
            let fonts = &mut self.fonts;
            let mut buffer = Buffer::new(fonts, Metrics::new(14.0, 20.0));
            buffer.set_wrap(fonts, Wrap::None);
            buffer.set_size(fonts, Some(text_width as f32), Some(20.0));
            buffer.set_text(
                fonts,
                &bounded_title(text),
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
                Some(cosmic_text::Align::Left),
            );
            buffer.draw(
                fonts,
                &mut SwashCache::new(),
                Color::rgba(color[0], color[1], color[2], color[3]),
                |x, y, _, _, color| {
                    if (0..text_width).contains(&x) {
                        let mut rgba = color.as_rgba();
                        rgba[3] = (u32::from(rgba[3]) * (text_width - x).min(12) as u32 / 12) as u8;
                        blend(&mut pixels, viewport, x + inset, y + 6, rgba);
                    }
                },
            );
        }
        pixels
    }

    /// Drop textures for windows no longer managed (or no longer using SSD).
    pub(super) fn retain(&mut self, mut keep: impl FnMut(WindowId) -> bool) {
        self.entries.retain(|(id, _), _| keep(*id));
    }

    /// Return square, premultiplied RGBA strips for the titlebar/clip intersection.
    /// `frame` and `clip` share global logical coordinates; textures are at scale 1
    /// and at most 4096 pixels wide. Text and controls retain full-frame positions.
    /// Cache identity uses frame-local viewports, not global placement. The caller
    /// must round the combined body using its original outline, not these strips.
    /// Empty intersections return no elements; upload failures are propagated.
    pub(super) fn elements(
        &mut self,
        renderer: &mut GlesRenderer,
        id: WindowId,
        title: &str,
        frame: Rect,
        focused: bool,
        maximized: bool,
        clip: Rect,
        theme: &TitlebarTheme,
        assets: &TitlebarAssets,
        app_id: &str,
    ) -> Result<Vec<TextureRenderElement<GlesTexture>>, GlesError> {
        let Some(layout) = Layout::new(frame, theme) else {
            self.entries.retain(|(window, _), _| *window != id);
            return Ok(Vec::new());
        };
        let mut elements = Vec::new();
        for viewport in viewports(frame, clip, theme.height) {
            let slot = (
                id,
                (viewport.x, viewport.y, viewport.width, viewport.height),
            );
            let key = CacheKey::new(title, &layout, viewport, focused, maximized)
                .with_assets(assets, app_id);
            if !self
                .entries
                .get(&slot)
                .is_some_and(|entry| entry.key == key)
            {
                self.entries.remove(&slot);
                let mut bytes: usize = self.entries.values().map(|entry| entry.key.bytes()).sum();
                while !cache_has_room(self.entries.len(), bytes, key.bytes()) {
                    let Some((_, old)) = self.entries.pop_first() else {
                        break;
                    };
                    bytes -= old.key.bytes();
                }
                let pixels = rasterize(&mut self.fonts, &layout, &key);
                // Rows run top-to-bottom, like logical coordinates. Abgr8888 means
                // RGBA bytes on this adapter's little-endian GLES path; no Y flip.
                let texture = renderer.import_memory(
                    &pixels,
                    Fourcc::Abgr8888,
                    (viewport.width, viewport.height).into(),
                    false,
                )?;
                let buffer =
                    TextureBuffer::from_texture(renderer, texture, 1, Transform::Normal, None);
                self.entries.insert(slot, CachedBar { key, buffer });
            }
            let cached = &self.entries[&slot];
            elements.push(TextureRenderElement::from_texture_buffer(
                (
                    f64::from(frame.x) + f64::from(viewport.x),
                    f64::from(frame.y) + f64::from(viewport.y),
                ),
                &cached.buffer,
                None,
                None,
                None,
                Kind::Unspecified,
            ));
        }
        Ok(elements)
    }
}

fn rasterize(fonts: &mut FontSystem, layout: &Layout, key: &CacheKey) -> Vec<u8> {
    let background = key.style.color(if key.focused { 0 } else { 1 });
    let foreground = key.style.color(if key.focused { 2 } else { 3 });
    let viewport = key.viewport;
    let mut pixels = premultiply(background).repeat((viewport.width * viewport.height) as usize);
    let text_width = (layout.text_right - layout.text_left - TEXT_INSET).min(MAX_TEXT_WIDTH);
    if key.style.show_title
        && text_width > 0
        && viewport.x < layout.text_left + text_width
        && viewport.right() > layout.text_left
        && !key.title.is_empty()
        && fonts.db().faces().next().is_some()
    {
        let mut buffer = Buffer::new(fonts, Metrics::new(13.0, LINE_HEIGHT as f32));
        buffer.set_wrap(fonts, Wrap::None);
        buffer.set_size(fonts, Some(text_width as f32), Some(LINE_HEIGHT as f32));
        buffer.set_text(
            fonts,
            &key.title,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            Some(cosmic_text::Align::Left),
        );
        // Per-raster storage prevents an endless stream of titles retaining every
        // glyph ever seen. The resulting GPU texture is the persistent cache.
        let mut glyphs = SwashCache::new();
        let top = (layout.height - LINE_HEIGHT) / 2;
        buffer.draw(
            fonts,
            &mut glyphs,
            Color::rgba(foreground[0], foreground[1], foreground[2], 255),
            |x, y, _, _, color| {
                if (0..text_width).contains(&x) {
                    let mut rgba = color.as_rgba();
                    // Fade the clipped edge without ever drawing over the controls.
                    // Apply theme alpha even to color-font glyphs (whose RGB is
                    // supplied by the font rather than the monochrome foreground).
                    rgba[3] = (u32::from(rgba[3])
                        * u32::from(foreground[3])
                        * (text_width - x).min(12) as u32
                        / (255 * 12)) as u8;
                    let y = top + y;
                    if y > 0 && y < layout.height - 1 {
                        blend(&mut pixels, viewport, layout.text_left + x, y, rgba);
                    }
                }
            },
        );
    }
    if let Some(rect) = layout.icon {
        if !draw_asset(&mut pixels, viewport, rect, key.images[3].as_ref()) {
            draw_fallback_icon(&mut pixels, viewport, rect, foreground);
        }
    }
    for (part, rect) in &layout.controls {
        let index = match part {
            TitlebarPart::Minimize => 0,
            TitlebarPart::Maximize => 1,
            TitlebarPart::Close => 2,
            TitlebarPart::Drag => continue,
        };
        let size = key.style.icon_size.min(rect.width).min(rect.height);
        let image_rect = Rect::new(
            rect.x + (rect.width - size) / 2,
            rect.y + (rect.height - size) / 2,
            size,
            size,
        );
        if draw_asset(
            &mut pixels,
            viewport,
            image_rect,
            key.images[index].as_ref(),
        ) {
            continue;
        }
        draw_control(
            &mut pixels,
            viewport,
            *rect,
            *part,
            key.maximized,
            foreground,
        );
    }
    pixels
}

fn premultiply(mut color: [u8; 4]) -> [u8; 4] {
    for channel in 0..3 {
        color[channel] = ((u32::from(color[channel]) * u32::from(color[3]) + 127) / 255) as u8;
    }
    color
}

// Straight glyph/theme input, premultiplied destination, including its alpha.
fn blend(pixels: &mut [u8], viewport: Rect, x: i32, y: i32, color: [u8; 4]) {
    blend_premultiplied(pixels, viewport, x, y, premultiply(color));
}

fn blend_premultiplied(pixels: &mut [u8], viewport: Rect, x: i32, y: i32, color: [u8; 4]) {
    if !viewport.contains(x, y) {
        return;
    }
    let offset = (((y - viewport.y) * viewport.width + (x - viewport.x)) * 4) as usize;
    for channel in 0..4 {
        pixels[offset + channel] = (u32::from(color[channel])
            + (u32::from(pixels[offset + channel]) * (255 - u32::from(color[3])) + 127) / 255)
            .min(255) as u8;
    }
}

// SVG/desktop icon decoding is entirely runtime-owned. Draw prepared premultiplied
// pixels directly: multiplying their alpha again would darken translucent edges.
fn draw_asset(pixels: &mut [u8], viewport: Rect, rect: Rect, key: Option<&ImageKey>) -> bool {
    let Some(image) = key.and_then(|key| key.0.upgrade()) else {
        return false;
    };
    draw_image(pixels, viewport, rect, &image)
}

fn draw_image(pixels: &mut [u8], viewport: Rect, rect: Rect, image: &TitlebarImage) -> bool {
    let expected = (image.width as usize)
        .checked_mul(image.height as usize)
        .and_then(|size| size.checked_mul(4));
    if image.width == 0 || image.height == 0 || expected != Some(image.pixels.len()) {
        return false;
    }
    if rect.width <= 0 || rect.height <= 0 {
        return true;
    }
    let (width, height) = if u64::from(image.width) * rect.height as u64
        > u64::from(image.height) * rect.width as u64
    {
        (
            rect.width,
            (u64::from(image.height) * rect.width as u64 / u64::from(image.width)).max(1) as i32,
        )
    } else {
        (
            (u64::from(image.width) * rect.height as u64 / u64::from(image.height)).max(1) as i32,
            rect.height,
        )
    };
    let dest = Rect::new(
        rect.x + (rect.width - width) / 2,
        rect.y + (rect.height - height) / 2,
        width,
        height,
    );
    let Some(visible) = dest.intersection(viewport) else {
        return true;
    };
    for y in visible.y..visible.bottom() {
        for x in visible.x..visible.right() {
            // Center-sampled scaling remains local and exact for extreme frame
            // origins. Normal-size prepared assets are copied 1:1 to stay sharp.
            let sx = ((2 * (x - dest.x) as u64 + 1) * u64::from(image.width) / (2 * width as u64))
                as usize;
            let sy = ((2 * (y - dest.y) as u64 + 1) * u64::from(image.height) / (2 * height as u64))
                as usize;
            let offset = (sy * image.width as usize + sx) * 4;
            let color = image.pixels[offset..offset + 4].try_into().unwrap();
            blend_premultiplied(pixels, viewport, x, y, color);
        }
    }
    true
}

fn draw_fallback_icon(pixels: &mut [u8], viewport: Rect, rect: Rect, color: [u8; 4]) {
    let Some(visible) = rect.intersection(viewport) else {
        return;
    };
    for y in visible.y..visible.bottom() {
        for x in visible.x..visible.right() {
            let (local_x, local_y) = (x - rect.x, y - rect.y);
            if local_x == 0
                || local_x == rect.width - 1
                || local_y == 0
                || local_y == rect.height - 1
                || local_y == (rect.height / 3).max(1)
            {
                blend(pixels, viewport, x, y, color);
            }
        }
    }
}

fn draw_control(
    pixels: &mut [u8],
    viewport: Rect,
    rect: Rect,
    part: TitlebarPart,
    maximized: bool,
    color: [u8; 4],
) {
    let size = (rect.width.min(rect.height) - 8).clamp(1, 10) as f32;
    let left = (rect.width as f32 - size) / 2.0;
    let top = (rect.height as f32 - size) / 2.0;
    // Normalized line segments, rasterized with coverage rather than font glyphs.
    let segments: &[(f32, f32, f32, f32)] = match part {
        TitlebarPart::Close => &[(0., 0., 1., 1.), (1., 0., 0., 1.)],
        TitlebarPart::Minimize => &[(0., 0.7, 1., 0.7)],
        TitlebarPart::Maximize if maximized => &[
            (0.25, 0., 1., 0.),
            (1., 0., 1., 0.75),
            (0.75, 0.75, 1., 0.75),
            (0., 0.25, 0.75, 0.25),
            (0.75, 0.25, 0.75, 1.),
            (0.75, 1., 0., 1.),
            (0., 1., 0., 0.25),
        ],
        TitlebarPart::Maximize => &[
            (0., 0., 1., 0.),
            (1., 0., 1., 1.),
            (1., 1., 0., 1.),
            (0., 1., 0., 0.),
        ],
        TitlebarPart::Drag => return,
    };
    let Some(visible) = rect.intersection(viewport) else {
        return;
    };
    for y in visible.y..visible.bottom() {
        for x in visible.x..visible.right() {
            let mut distance = f32::INFINITY;
            for &(ax, ay, bx, by) in segments {
                let a = (left + ax * size, top + ay * size);
                let b = (left + bx * size, top + by * size);
                // Keep coverage math button-local even for frames wider than f32's
                // exact integer range.
                let p = (
                    (x - rect.x) as f32 + 0.5 - a.0,
                    (y - rect.y) as f32 + 0.5 - a.1,
                );
                let v = (b.0 - a.0, b.1 - a.1);
                let t = ((p.0 * v.0 + p.1 * v.1) / (v.0 * v.0 + v.1 * v.1)).clamp(0., 1.);
                distance = distance.min((p.0 - t * v.0).hypot(p.1 - t * v.1));
            }
            let mut ink = color;
            ink[3] = ((1.25 - distance).clamp(0., 1.) * f32::from(color[3])).round() as u8;
            blend(pixels, viewport, x, y, ink);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLEBAR_HEIGHT: i32 = 32;
    const BUTTON_WIDTH: i32 = 32;

    fn default_layout(frame: Rect) -> Option<Layout> {
        Layout::new(frame, &TitlebarTheme::default())
    }

    fn default_hit(frame: Rect, point: Point<f64, Logical>) -> Option<TitlebarPart> {
        titlebar_hit(frame, point, &TitlebarTheme::default())
    }

    fn default_content_rect(frame: Rect, ssd: bool) -> Rect {
        content_rect(frame, ssd, TITLEBAR_HEIGHT)
    }

    fn default_viewports(frame: Rect, clip: Rect) -> impl Iterator<Item = Rect> {
        viewports(frame, clip, TITLEBAR_HEIGHT)
    }

    fn empty_fonts() -> FontSystem {
        FontSystem::new_with_locale_and_db("en-US".into(), fontdb::Database::new())
    }

    fn full_key(title: &str, layout: &Layout, focused: bool, maximized: bool) -> CacheKey {
        CacheKey::new(
            title,
            layout,
            Rect::new(0, 0, layout.width, layout.height),
            focused,
            maximized,
        )
    }

    #[test]
    fn insets_preserve_a_client_row_and_undecorated_frames() {
        for height in 0..70 {
            let frame = Rect::new(-200, 7, 240, height);
            assert_eq!(default_content_rect(frame, false), frame);
            let client = default_content_rect(frame, true);
            let bar = (height - 1).clamp(0, TITLEBAR_HEIGHT);
            assert_eq!(client, Rect::new(-200, 7 + bar, 240, height - bar));
            if height > 0 {
                assert!(client.height >= 1);
            }
        }
    }

    #[test]
    fn tiny_controls_fit_and_prioritize_close_then_maximize_then_minimize() {
        for width in 1..150 {
            for height in 2..35 {
                let frame = Rect::new(-100, 13, width, height);
                let layout = default_layout(frame).unwrap();
                assert_eq!(
                    layout.controls.len(),
                    (width / BUTTON_WIDTH).clamp(1, 3) as usize
                );
                let mut right = width;
                for (part, rect) in &layout.controls {
                    assert_eq!(rect.right(), right);
                    assert!(rect.x >= 0 && rect.height == layout.height);
                    right = rect.x;
                    for x in rect.x..rect.right() {
                        assert_eq!(
                            default_hit(frame, (f64::from(frame.x + x) + 0.5, 13.5).into()),
                            Some(*part)
                        );
                    }
                }
                for x in 0..right {
                    assert_eq!(
                        default_hit(frame, (f64::from(frame.x + x) + 0.5, 13.5).into()),
                        Some(TitlebarPart::Drag)
                    );
                }
            }
        }
    }

    #[test]
    fn hit_edges_are_half_open_and_reject_non_finite_points() {
        let frame = Rect::new(-20, 30, 200, 100);
        assert_eq!(
            default_hit(frame, (-20., 30.).into()),
            Some(TitlebarPart::Drag)
        );
        assert_eq!(
            default_hit(frame, (148., 30.).into()),
            Some(TitlebarPart::Close)
        );
        for point in [
            (-20.01, 30.),
            (180., 30.),
            (0., 62.),
            (0., 29.99),
            (f64::NAN, 30.),
            (0., f64::INFINITY),
        ] {
            assert_eq!(default_hit(frame, point.into()), None);
        }
    }

    #[test]
    fn empty_extents_have_no_controls() {
        for (width, height) in [(0, 100), (-1, 100), (100, 0), (100, 1)] {
            let frame = Rect {
                x: 0,
                y: 0,
                width,
                height,
            };
            assert!(default_layout(frame).is_none());
            assert_eq!(default_hit(frame, (0., 0.).into()), None);
        }
        let layout = default_layout(Rect::new(0, 0, MAX_TEXTURE_WIDTH, 100)).unwrap();
        assert_eq!(layout.width * layout.height * 4, 524_288);
    }

    #[test]
    fn wide_frames_keep_original_right_aligned_controls() {
        for width in [4097, 20_000, i32::MAX] {
            let frame = Rect::new(-100, 20, width, 100);
            let layout = default_layout(frame).unwrap();
            assert_eq!(layout.text_right, width - 3 * BUTTON_WIDTH);
            for (index, (part, rect)) in layout.controls.iter().enumerate() {
                assert_eq!(rect.x, width - (index as i32 + 1) * BUTTON_WIDTH);
                assert_eq!(
                    default_hit(
                        frame,
                        (f64::from(frame.x) + f64::from(rect.x) + 16., 36.).into()
                    ),
                    Some(*part),
                );
            }
            assert_eq!(
                default_hit(frame, (0., 36.).into()),
                Some(TitlebarPart::Drag)
            );
        }
    }

    #[test]
    fn visible_strips_cover_only_the_intersection_with_bounded_allocations() {
        let frame = Rect::new(-250, 70, 30_000, 200);
        for width in [1, 4096, 4097, 16_384, 20_003] {
            let clip = Rect::new(50, 75, width, 20);
            let strips: Vec<_> = default_viewports(frame, clip).collect();
            assert_eq!(strips.len(), ((width + 4095) / 4096) as usize);
            let mut right = 300;
            for strip in &strips {
                assert_eq!(strip.x, right);
                assert_eq!((strip.y, strip.height), (5, 20));
                assert!((1..=MAX_TEXTURE_WIDTH).contains(&strip.width));
                assert!(strip.width * strip.height * 4 <= 524_288);
                right = strip.right();
            }
            assert_eq!(right, 300 + width);
            let translated_frame = Rect::new(750, -30, frame.width, frame.height);
            let translated_clip = Rect::new(1050, -25, width, 20);
            assert_eq!(
                strips,
                default_viewports(translated_frame, translated_clip).collect::<Vec<_>>()
            );
        }
        for clip in [
            Rect::new(-300, 70, 50, 32),
            Rect::new(-250, 102, 100, 50),
            Rect::new(-250, 70, 0, 32),
            Rect::new(-250, 70, 100, 0),
        ] {
            assert_eq!(default_viewports(frame, clip).count(), 0);
        }
        assert_eq!(default_viewports(Rect::new(0, 0, 100, 1), frame).count(), 0);
    }

    #[test]
    fn cropped_strips_match_full_raster_without_new_edges_or_corners() {
        let frame = Rect::new(-120, -10, 8240, 100);
        let layout = default_layout(frame).unwrap();
        let mut fonts = empty_fonts();
        for maximized in [false, true] {
            let full = rasterize(&mut fonts, &layout, &full_key("", &layout, true, maximized));
            for clip in [frame, Rect::new(-115, -5, 8230, 20)] {
                for viewport in default_viewports(frame, clip) {
                    let key = CacheKey::new("", &layout, viewport, true, maximized);
                    let pixels = rasterize(&mut fonts, &layout, &key);
                    assert_eq!(
                        pixels.len(),
                        (viewport.width * viewport.height * 4) as usize
                    );
                    for y in 0..viewport.height {
                        let start = (((viewport.y + y) * layout.width + viewport.x) * 4) as usize;
                        let row = (y * viewport.width * 4) as usize;
                        let len = (viewport.width * 4) as usize;
                        assert_eq!(&pixels[row..row + len], &full[start..start + len]);
                    }
                    assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
                }
            }
        }
    }

    #[test]
    fn extreme_width_controls_match_small_frame_without_float_precision_loss() {
        let mut fonts = empty_fonts();
        let small = default_layout(Rect::new(0, 0, 96, 100)).unwrap();
        let wide = default_layout(Rect::new(0, 0, i32::MAX, 100)).unwrap();
        for maximized in [false, true] {
            let expected = rasterize(&mut fonts, &small, &full_key("", &small, true, maximized));
            let viewport = Rect::new(i32::MAX - 96, 0, 96, 32);
            let actual = rasterize(
                &mut fonts,
                &wide,
                &CacheKey::new("", &wide, viewport, true, maximized),
            );
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn viewport_cache_keys_track_local_crops_not_global_positions() {
        let frame = Rect::new(-500, 20, 10_000, 100);
        let clip = Rect::new(100, 25, 2000, 20);
        let layout = default_layout(frame).unwrap();
        let viewport = default_viewports(frame, clip).next().unwrap();
        let key = CacheKey::new("title", &layout, viewport, true, false);
        let moved = Rect::new(500, -80, 10_000, 500);
        let moved_clip = Rect::new(1100, -75, 2000, 20);
        assert_eq!(
            key,
            CacheKey::new(
                "title",
                &default_layout(moved).unwrap(),
                default_viewports(moved, moved_clip).next().unwrap(),
                true,
                false,
            )
        );
        for other in [
            Rect {
                x: viewport.x + 1,
                ..viewport
            },
            Rect {
                y: viewport.y + 1,
                ..viewport
            },
            Rect {
                width: viewport.width - 1,
                ..viewport
            },
            Rect {
                height: viewport.height - 1,
                ..viewport
            },
        ] {
            assert_ne!(key, CacheKey::new("title", &layout, other, true, false));
        }
    }

    #[test]
    fn title_normalization_is_bounded_unicode_and_single_line() {
        assert_eq!(
            bounded_title("Hello 世界 — مرحبا 👩‍💻"),
            "Hello 世界 — مرحبا 👩‍💻"
        );
        assert_eq!(bounded_title("a\n\r\t\0b\u{2028}c\u{2029}"), "a    b c ");
        for title in ["🦀".repeat(2000), "a".repeat(1025), "é".repeat(513)] {
            let title = bounded_title(&title);
            assert!(title.len() <= MAX_TITLE_BYTES);
            assert!(title.ends_with('…'));
        }
    }

    #[test]
    fn cache_key_tracks_visual_state_but_not_position_or_client_height() {
        let frame = Rect::new(0, 0, 300, 200);
        let layout = default_layout(frame).unwrap();
        let key = full_key("title", &layout, true, false);
        let moved = default_layout(Rect::new(-900, 70, 300, 500)).unwrap();
        assert_eq!(key, full_key("title", &moved, true, false));
        for other in [
            full_key("other", &layout, true, false),
            full_key("title", &layout, false, false),
            full_key("title", &layout, true, true),
            full_key(
                "title",
                &default_layout(Rect::new(0, 0, 301, 200)).unwrap(),
                true,
                false,
            ),
            full_key(
                "title",
                &default_layout(Rect::new(0, 0, 300, 10)).unwrap(),
                true,
                false,
            ),
        ] {
            assert_ne!(key, other);
        }
    }

    #[test]
    fn fontless_raster_has_controls_square_corners_and_top_first_rgba_rows() {
        let layout = default_layout(Rect::new(0, 0, 220, 200)).unwrap();
        let mut fonts = empty_fonts();
        let key = full_key("missing font", &layout, true, false);
        let pixels = rasterize(&mut fonts, &layout, &key);
        assert_eq!(pixels.len(), 220 * 32 * 4);
        assert_eq!(&pixels[..4], &[35, 40, 52, 255]);
        assert_eq!(&pixels[..4], &pixels[pixels.len() - 4..]);
        assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
        for (_, rect) in &layout.controls {
            assert!((2..layout.height - 2).any(|y| {
                (rect.x..rect.right()).any(|x| pixels[((y * layout.width + x) * 4) as usize] > 150)
            }));
        }
        let restored = rasterize(&mut fonts, &layout, &full_key("", &layout, true, true));
        assert_ne!(pixels, restored);
    }

    #[test]
    fn system_fonts_are_memory_backed_and_unicode_text_stays_out_of_controls() {
        let mut cache = TitlebarCache::default();
        if cache.fonts.db().faces().next().is_none() {
            eprintln!("skipping system-font raster assertions: no optional fonts installed");
            return;
        }
        assert!(
            cache
                .fonts
                .db()
                .faces()
                .all(|face| matches!(face.source, fontdb::Source::Binary(_)))
        );
        let layout = default_layout(Rect::new(0, 0, 280, 200)).unwrap();
        let blank = rasterize(
            &mut cache.fonts,
            &layout,
            &full_key("", &layout, true, false),
        );
        let key = full_key(
            &"Title — العربية 世界 👩‍💻 e\u{301} ".repeat(100),
            &layout,
            true,
            false,
        );
        let pixels = rasterize(&mut cache.fonts, &layout, &key);
        assert_ne!(pixels, blank);
        assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
        for y in 0..layout.height {
            let start = ((y * layout.width + layout.text_right) * 4) as usize;
            let end = ((y + 1) * layout.width * 4) as usize;
            assert_eq!(&pixels[start..end], &blank[start..end]);
        }
        assert_eq!(pixels, rasterize(&mut cache.fonts, &layout, &key));

        let wide = default_layout(Rect::new(-500, 70, 20_000, 200)).unwrap();
        let left = Rect::new(0, 0, 1200, TITLEBAR_HEIGHT);
        let wide_key = CacheKey::new(&key.title, &wide, left, true, false);
        let wide_pixels = rasterize(&mut cache.fonts, &wide, &wide_key);
        let narrow = default_layout(Rect::new(0, 0, 2000, 200)).unwrap();
        assert_eq!(
            wide_pixels,
            rasterize(
                &mut cache.fonts,
                &narrow,
                &CacheKey::new(&key.title, &narrow, left, true, false),
            )
        );
        let cropped = Rect::new(180, 5, 700, 20);
        let cropped_pixels = rasterize(
            &mut cache.fonts,
            &wide,
            &CacheKey::new(&key.title, &wide, cropped, true, false),
        );
        for y in 0..cropped.height {
            let start = (((cropped.y + y) * left.width + cropped.x) * 4) as usize;
            let row = (y * cropped.width * 4) as usize;
            let len = (cropped.width * 4) as usize;
            assert_eq!(
                &cropped_pixels[row..row + len],
                &wide_pixels[start..start + len]
            );
        }
        let beyond_text = Rect::new(TEXT_INSET + MAX_TEXT_WIDTH, 0, 1000, TITLEBAR_HEIGHT);
        assert_eq!(
            rasterize(
                &mut cache.fonts,
                &wide,
                &CacheKey::new(&key.title, &wide, beyond_text, true, false)
            ),
            rasterize(
                &mut cache.fonts,
                &wide,
                &CacheKey::new("", &wide, beyond_text, true, false)
            ),
        );
        cache.retain(|_| false);
        assert!(cache.entries.is_empty());
    }

    #[test]
    fn antialiased_ink_blends_to_opaque_premultiplied_pixels() {
        let viewport = Rect::new(0, 0, 1, 1);
        let mut pixels = vec![20, 40, 60, 255];
        blend(&mut pixels, viewport, 0, 0, [220, 140, 60, 128]);
        assert_eq!(pixels, [120, 90, 60, 255]);
        blend(&mut pixels, viewport, 0, 0, [0, 0, 0, 0]);
        assert_eq!(pixels, [120, 90, 60, 255]);
    }

    fn transparent_theme() -> TitlebarTheme {
        TitlebarTheme {
            active_background: [1.0, 0.5, 0.25, 0.0],
            inactive_background: [0.5, 1.0, 0.25, 0.0],
            ..TitlebarTheme::default()
        }
    }

    fn prepared_image(width: u32, height: u32, color: [u8; 4]) -> Arc<TitlebarImage> {
        Arc::new(TitlebarImage {
            width,
            height,
            pixels: color.repeat(width as usize * height as usize),
        })
    }

    fn assert_crop_matches(full: &[u8], width: i32, crop: Rect, actual: &[u8]) {
        assert_eq!(actual.len(), (crop.width * crop.height * 4) as usize);
        for y in 0..crop.height {
            let start = (((crop.y + y) * width + crop.x) * 4) as usize;
            let row = (y * crop.width * 4) as usize;
            let len = (crop.width * 4) as usize;
            assert_eq!(&actual[row..row + len], &full[start..start + len]);
        }
    }

    #[test]
    fn configurable_heights_and_mirrored_controls_share_hit_geometry() {
        for height in [16, 32, 64, 128] {
            for client_height in [1, 2, 8, 40, 129, 300] {
                for width in [1, 15, 31, 32, 63, 64, 95, 96, 200, 600] {
                    let frame = Rect::new(-80, 17, width, client_height);
                    let inset = (client_height - 1).min(height);
                    assert_eq!(
                        content_rect(frame, true, height),
                        Rect::new(-80, 17 + inset, width, client_height - inset)
                    );
                    assert_eq!(content_rect(frame, false, height), frame);
                    let right = TitlebarTheme {
                        height,
                        ..TitlebarTheme::default()
                    };
                    let left = TitlebarTheme {
                        controls_side: ControlsSide::Left,
                        ..right.clone()
                    };
                    let Some(r) = Layout::new(frame, &right) else {
                        assert_eq!(inset, 0);
                        assert_eq!(titlebar_hit(frame, (-80., 17.).into(), &left), None);
                        continue;
                    };
                    let l = Layout::new(frame, &left).unwrap();
                    assert_eq!(r.height, inset);
                    assert_eq!(r.controls.len(), (width / height).clamp(1, 3) as usize);
                    for ((rp, rr), (lp, lr)) in r.controls.iter().zip(&l.controls) {
                        assert_eq!(rp, lp);
                        assert_eq!(lr.x, width - rr.right());
                        assert_eq!(lr.width, rr.width);
                    }
                    for x in 0..width {
                        let rp = titlebar_hit(
                            frame,
                            (f64::from(frame.x + x) + 0.5, 17.5).into(),
                            &right,
                        );
                        let lp = titlebar_hit(
                            frame,
                            (f64::from(frame.x + width - 1 - x) + 0.5, 17.5).into(),
                            &left,
                        );
                        assert_eq!(rp, lp);
                    }
                    assert_eq!(
                        titlebar_hit(frame, (-80., f64::from(17 + inset)).into(), &left),
                        None
                    );
                }
            }
        }
    }

    #[test]
    fn transparent_backgrounds_have_no_accent_separator_or_opaque_coverage() {
        let mut fonts = empty_fonts();
        let theme = transparent_theme();
        let layout = Layout::new(Rect::new(0, 0, 300, 200), &theme).unwrap();
        for focused in [false, true] {
            let pixels = rasterize(&mut fonts, &layout, &full_key("", &layout, focused, false));
            for y in 0..layout.height {
                for x in 0..layout.text_right {
                    let offset = ((y * layout.width + x) * 4) as usize;
                    assert_eq!(&pixels[offset..offset + 4], &[0, 0, 0, 0]);
                }
            }
            assert!(pixels.chunks_exact(4).any(|p| p[3] > 0));
            assert!(
                pixels
                    .chunks_exact(4)
                    .all(|p| p[..3].iter().all(|c| *c <= p[3]))
            );
        }
        let invisible = TitlebarTheme {
            active_foreground: [1.0, 0.5, 0.25, 0.0],
            show_icon: true,
            ..theme
        };
        let layout = Layout::new(Rect::new(0, 0, 300, 200), &invisible).unwrap();
        assert!(
            rasterize(&mut fonts, &layout, &full_key("", &layout, true, false))
                .iter()
                .all(|byte| *byte == 0)
        );
    }

    #[test]
    fn source_over_preserves_translucent_alpha_and_does_not_double_premultiply_assets() {
        let viewport = Rect::new(7, 5, 1, 1);
        let mut pixels = vec![0; 4];
        blend(&mut pixels, viewport, 7, 5, [200, 100, 50, 128]);
        assert_eq!(pixels, [100, 50, 25, 128]);
        blend_premultiplied(&mut pixels, viewport, 7, 5, [20, 40, 60, 128]);
        assert_eq!(pixels, [70, 65, 72, 192]);
        blend(&mut pixels, viewport, 6, 5, [255; 4]);
        assert_eq!(pixels, [70, 65, 72, 192]);
        let theme = TitlebarTheme {
            active_background: [200.0 / 255.0, 100.0 / 255.0, 50.0 / 255.0, 128.0 / 255.0],
            active_foreground: [0.0; 4],
            ..TitlebarTheme::default()
        };
        let layout = Layout::new(Rect::new(0, 0, 300, 200), &theme).unwrap();
        assert!(
            rasterize(
                &mut empty_fonts(),
                &layout,
                &full_key("", &layout, true, false)
            )
            .chunks_exact(4)
            .all(|p| p == [100, 50, 25, 128])
        );
    }

    #[test]
    fn hidden_titles_and_icons_and_unknown_app_fallback() {
        let mut cache = TitlebarCache::default();
        let frame = Rect::new(0, 0, 300, 200);
        for side in [ControlsSide::Left, ControlsSide::Right] {
            let theme = TitlebarTheme {
                show_title: false,
                controls_side: side,
                ..transparent_theme()
            };
            let hidden = Layout::new(frame, &theme).unwrap();
            let blank = rasterize(
                &mut cache.fonts,
                &hidden,
                &full_key("", &hidden, true, false),
            );
            assert_eq!(
                blank,
                rasterize(
                    &mut cache.fonts,
                    &hidden,
                    &full_key("must not show 世界", &hidden, true, false)
                )
            );
            assert!(hidden.icon.is_none());
            let with_icon = Layout::new(
                frame,
                &TitlebarTheme {
                    show_icon: true,
                    ..theme
                },
            )
            .unwrap();
            let fallback = rasterize(
                &mut cache.fonts,
                &with_icon,
                &full_key("hidden", &with_icon, true, false),
            );
            assert_ne!(blank, fallback);
            let rect = with_icon.icon.unwrap();
            for y in 0..hidden.height {
                for x in 0..hidden.width {
                    if !rect.contains(x, y) {
                        let offset = ((y * hidden.width + x) * 4) as usize;
                        assert_eq!(&blank[offset..offset + 4], &fallback[offset..offset + 4]);
                    }
                }
            }
        }
    }

    #[test]
    fn prepared_svg_and_app_rasters_are_applied_and_cropped_in_original_boxes() {
        let mut fonts = empty_fonts();
        // The runtime's SVG decoder supplies this same premultiplied RGBA contract.
        let image = prepared_image(20, 20, [80, 40, 20, 128]);
        for side in [ControlsSide::Left, ControlsSide::Right] {
            for height in [16, 32, 128] {
                for (width, frame_height) in [(1, 2), (17, 8), (65, 33), (97, 200), (600, 200)] {
                    let theme = TitlebarTheme {
                        height,
                        controls_side: side,
                        show_icon: true,
                        show_title: false,
                        ..transparent_theme()
                    };
                    let layout = Layout::new(Rect::new(0, 0, width, frame_height), &theme).unwrap();
                    let mut key = full_key("", &layout, true, false);
                    key.images = std::array::from_fn(|_| image_key(Some(&image)));
                    let full = rasterize(&mut fonts, &layout, &key);
                    assert!(full.chunks_exact(4).any(|p| p == [80, 40, 20, 128]));
                    assert!(
                        full.chunks_exact(4)
                            .all(|p| p == [0; 4] || p == [80, 40, 20, 128])
                    );
                    let crop = Rect::new(
                        width / 3,
                        layout.height / 3,
                        width - width / 3,
                        layout.height - layout.height / 3,
                    );
                    key.viewport = crop;
                    assert_crop_matches(&full, width, crop, &rasterize(&mut fonts, &layout, &key));
                }
            }
        }
    }

    #[test]
    fn prepared_image_scaling_preserves_aspect_and_source_rows_at_extreme_origins() {
        let image = TitlebarImage {
            width: 2,
            height: 2,
            pixels: vec![
                128, 0, 0, 128, 0, 128, 0, 128, 0, 0, 128, 128, 128, 128, 0, 128,
            ],
        };
        let viewport = Rect::new(i32::MAX - 4, 1, 4, 4);
        let mut pixels = vec![0; 64];
        assert!(draw_image(&mut pixels, viewport, viewport, &image));
        assert_eq!(&pixels[..8], &[128, 0, 0, 128, 128, 0, 0, 128]);
        assert_eq!(&pixels[8..16], &[0, 128, 0, 128, 0, 128, 0, 128]);
        assert_eq!(&pixels[32..40], &[0, 0, 128, 128, 0, 0, 128, 128]);
        let crop = Rect::new(i32::MAX - 3, 2, 2, 2);
        let mut cropped = vec![0; 16];
        assert!(draw_image(&mut cropped, crop, viewport, &image));
        assert_eq!(cropped, image.pixels);
        let wide = prepared_image(4, 2, [50, 20, 10, 100]);
        pixels.fill(0);
        assert!(draw_image(&mut pixels, viewport, viewport, &wide));
        assert!(pixels[..16].iter().all(|b| *b == 0));
        assert!(pixels[48..].iter().all(|b| *b == 0));
        assert!(
            pixels[16..48]
                .chunks_exact(4)
                .all(|p| p == [50, 20, 10, 100])
        );
        let malformed = TitlebarImage {
            width: 2,
            height: 2,
            pixels: vec![0; 3],
        };
        assert!(!draw_image(&mut pixels, viewport, viewport, &malformed));
    }

    #[test]
    fn cache_identity_tracks_styles_app_id_and_same_path_asset_replacements() {
        let theme = TitlebarTheme::default();
        let frame = Rect::new(0, 0, 600, 200);
        let layout = Layout::new(frame, &theme).unwrap();
        let key = full_key("title", &layout, true, false);
        for changed in [
            TitlebarTheme {
                height: 128,
                ..theme.clone()
            },
            TitlebarTheme {
                controls_side: ControlsSide::Left,
                ..theme.clone()
            },
            TitlebarTheme {
                show_title: false,
                ..theme.clone()
            },
            TitlebarTheme {
                show_icon: true,
                ..theme.clone()
            },
            TitlebarTheme {
                active_background: [0.0; 4],
                ..theme.clone()
            },
            TitlebarTheme {
                inactive_background: [0.0; 4],
                ..theme.clone()
            },
            TitlebarTheme {
                active_foreground: [0.0; 4],
                ..theme.clone()
            },
            TitlebarTheme {
                inactive_foreground: [0.0; 4],
                ..theme.clone()
            },
        ] {
            let other = Layout::new(frame, &changed).unwrap();
            assert_ne!(key, full_key("title", &other, true, false));
        }
        let mut other = full_key("title", &layout, true, false);
        other.app_id = "new.app".into();
        assert_ne!(key, other);
        let original = prepared_image(1, 1, [80, 40, 20, 128]);
        let replacement = prepared_image(1, 1, [20, 40, 80, 128]);
        for index in 0..4 {
            let mut before = full_key("title", &layout, true, false);
            before.images[index] = image_key(Some(&original));
            let mut after = full_key("title", &layout, true, false);
            after.images[index] = image_key(Some(&original.clone()));
            assert_eq!(before, after);
            after.images[index] = image_key(Some(&replacement));
            assert_ne!(before, after);
            assert_ne!(key, before);
        }
        let weak = image_key(Some(&original)).unwrap();
        assert_eq!(Arc::strong_count(&original), 1);
        drop(original);
        assert!(weak.0.upgrade().is_none());
        assert_ne!(Some(weak), image_key(Some(&replacement)));
    }

    #[test]
    fn texture_budget_is_byte_bounded_at_max_height_as_well_as_entry_bounded() {
        let theme = TitlebarTheme {
            height: 128,
            ..TitlebarTheme::default()
        };
        let frame = Rect::new(-100, 20, 30_000, 300);
        for viewport in viewports(frame, frame, theme.height) {
            assert!(viewport.width <= MAX_TEXTURE_WIDTH);
            assert_eq!(viewport.height, 128);
            let layout = Layout::new(frame, &theme).unwrap();
            assert!(CacheKey::new("", &layout, viewport, true, false).bytes() <= 2 * 1024 * 1024);
        }
        let strip_bytes = MAX_TEXTURE_WIDTH as usize * MAX_BAR_HEIGHT as usize * 4;
        assert_eq!(MAX_CACHE_BYTES / strip_bytes, 32);
        assert!(cache_has_room(31, 31 * strip_bytes, strip_bytes));
        assert!(!cache_has_room(32, 32 * strip_bytes, strip_bytes));
        assert!(!cache_has_room(MAX_ENTRIES, 4 * MAX_ENTRIES, 4));
        assert!(cache_has_room(MAX_ENTRIES - 1, 4 * (MAX_ENTRIES - 1), 4));
        assert!(!cache_has_room(0, 0, MAX_CACHE_BYTES + 1));
    }

    #[test]
    fn tiny_rasters_remain_bounded_and_premultiplied() {
        let mut fonts = empty_fonts();
        for width in 1..100 {
            for height in 2..35 {
                let layout = default_layout(Rect::new(0, 0, width, height)).unwrap();
                let pixels = rasterize(&mut fonts, &layout, &full_key("", &layout, false, false));
                assert_eq!(pixels.len(), (layout.width * layout.height * 4) as usize);
                assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
                assert!(pixels.chunks_exact(4).any(|p| p[0] > 100));
            }
        }
    }
}
