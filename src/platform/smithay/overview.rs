//! Compositor-owned card layout, hits, and bounded renderer-local resources.

use super::{
    blur::BlurMask,
    rounded::RoundedShaders,
    scene::{Scene, SceneElement, SceneGroup, logical, premultiply},
    state::Compositor,
    titlebar::TitlebarCache,
    wallpaper::{WallpaperCache, WallpaperDestination},
    window_image::{PreparedWindowImage, WindowImage, WindowImageComposer, WindowImageIdentity},
};
use crate::{
    core::{Rect, WindowId, WorkspaceId},
    decoration::Theme,
    runtime::{OverviewSession, OverviewTarget},
};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem, Renderer,
            element::{Id, Kind, texture::TextureRenderElement},
            gles::{GlesError, GlesRenderer, GlesTexture},
        },
    },
    utils::{Physical, Rectangle, Transform},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 128;
const MAX_TEXTURE_EDGE: i32 = 2048;
const MAX_LABEL_BYTES: usize = 4 * 1024 * 1024;
const MAX_LABELS: usize = 128;

pub(super) use super::overview_drag::OverviewDrag;

#[derive(Debug, Clone)]
pub(super) struct OverviewItem {
    pub target: OverviewTarget,
    pub rect: Rect,
    pub preview: Option<Rect>,
}

/// One immutable snapshot supplies both rendering and pointer hit boxes.
#[derive(Debug, Clone)]
pub(super) struct OverviewLayout {
    pub output: Rect,
    pub canvas: Rect,
    pub columns: usize,
    pub items: Vec<OverviewItem>,
    pub page: usize,
    pub pages: usize,
}

impl OverviewLayout {
    pub fn new(session: &OverviewSession, output: Rect) -> Self {
        let margin = (output.width * 7 / 100).min(96).max(0);
        let gap = 12.min(output.width / 20).min(output.height / 20).max(0);
        let strip_height = (output.height / 14).clamp(1, 64);
        let strip_top = (output.height / 80).max(0);
        let tile_width = (strip_height * output.width / output.height.max(1)).max(1);
        let width = (output.width - 2 * margin).max(1);
        let slots = (width / (tile_width + gap).max(1)).clamp(1, 12) as usize;
        let desktops = session.desktop_targets();
        let workspace_index = desktops
            .iter()
            .position(|t| *t == session.desktop_target())
            .unwrap_or(0);
        let start = workspace_index
            .saturating_sub(slots / 2)
            .min(desktops.len().saturating_sub(slots));
        let visible = desktops.iter().skip(start).take(slots).count().max(1);
        let mut items = Vec::new();
        let strip_width = visible as i32 * (tile_width + gap) - gap;
        let strip_left = output.x + (output.width - strip_width) / 2;
        for (i, target) in desktops.iter().skip(start).take(slots).enumerate() {
            let preview = Rect::new(
                strip_left + i as i32 * (tile_width + gap),
                output.y + strip_top,
                tile_width,
                strip_height,
            );
            items.push(OverviewItem {
                target: *target,
                rect: Rect::new(preview.x, preview.y, preview.width, preview.height + 32),
                preview: preview.intersection(output),
            });
        }
        let top = output.y + strip_top + strip_height + 32 + gap;
        let height = (output.bottom() - (output.height / 40).max(1) - top).max(1);
        let canvas = Rect::new(output.x + margin, top, width, height)
            .intersection(output)
            .unwrap_or(Rect::new(output.x, output.y, 0, 0));
        let padding = 20.min(width / 12).min(height / 12).max(0);
        let width = (width - 2 * padding).max(1);
        let height = (height - 2 * padding).max(1);
        let columns = (width / 240).clamp(1, 4) as usize;
        let rows = (height / 180).clamp(1, 3) as usize;
        let capacity = columns * rows;
        let selected = match session.selected {
            OverviewTarget::Window(id) => {
                session.windows.iter().position(|w| *w == id).unwrap_or(0)
            }
            _ => 0,
        };
        let page = selected / capacity;
        let pages = session.windows.len().div_ceil(capacity).max(1);
        let count = session
            .windows
            .iter()
            .skip(page * capacity)
            .take(capacity)
            .count();
        let used_columns = count.min(columns).max(1);
        let used_rows = count.div_ceil(columns).max(1);
        let cell_width = width / used_columns as i32;
        let cell_height = height / used_rows as i32;
        for (i, id) in session
            .windows
            .iter()
            .skip(page * capacity)
            .take(capacity)
            .enumerate()
        {
            let row_count = (count - (i / columns) * columns).min(columns);
            let row_left = canvas.x + padding + (width - row_count as i32 * cell_width) / 2;
            let rect = Rect::new(
                row_left + (i % columns) as i32 * cell_width + gap / 2,
                top + padding + (i / columns) as i32 * cell_height + gap / 2,
                (cell_width - gap).max(1),
                (cell_height - gap).max(1),
            );
            let preview = Rect::new(rect.x, rect.y, rect.width, (rect.height - 40).max(1));
            items.push(OverviewItem {
                target: OverviewTarget::Window(*id),
                rect,
                preview: preview
                    .intersection(rect)
                    .and_then(|r| r.intersection(output)),
            });
        }
        // Clip tiny outputs too; no hit region may extend into another output.
        for item in &mut items {
            item.rect = item
                .rect
                .intersection(output)
                .unwrap_or(Rect::new(output.x, output.y, 0, 0));
        }
        Self {
            output,
            canvas,
            columns,
            items,
            page,
            pages,
        }
    }

    // A common scale preserves the relative committed sizes of windows, while
    // centering each frame in a nonoverlapping slot. No client geometry changes.
    fn fit_previews(&mut self, source: impl Fn(WindowId) -> Option<Rect>) {
        let scale = self
            .items
            .iter()
            .filter_map(|item| {
                let OverviewTarget::Window(id) = item.target else {
                    return None;
                };
                let frame = source(id)?;
                let dest = item.preview?;
                Some(
                    (dest.width as f64 / frame.width.max(1) as f64)
                        .min(dest.height as f64 / frame.height.max(1) as f64),
                )
            })
            .fold(1.0, f64::min);
        for item in &mut self.items {
            let OverviewTarget::Window(id) = item.target else {
                continue;
            };
            let Some(dest) = item.preview else { continue };
            let preview = source(id).map_or(dest, |frame| {
                let width = (frame.width as f64 * scale).round().max(1.0) as i32;
                let height = (frame.height as f64 * scale).round().max(1.0) as i32;
                Rect::new(
                    dest.x + (dest.width - width) / 2,
                    dest.y + (dest.height - height) / 2,
                    width,
                    height,
                )
            });
            let width = preview.width.max(120).min(item.rect.width);
            item.rect = Rect::new(
                dest.x + (dest.width - width) / 2,
                preview.y,
                width,
                preview.height + 40,
            )
            .intersection(self.output)
            .unwrap_or(preview);
            item.preview = Some(preview);
        }
    }

    pub fn hit(&self, x: f64, y: f64) -> Option<OverviewTarget> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        self.items
            .iter()
            .find(|i| {
                x >= f64::from(i.rect.x)
                    && y >= f64::from(i.rect.y)
                    && x < f64::from(i.rect.right())
                    && y < f64::from(i.rect.bottom())
            })
            .map(|i| i.target)
    }
}

pub(super) fn fit(source: Rect, dest: Rect) -> Rect {
    let scale = (dest.width as f64 / source.width.max(1) as f64)
        .min(dest.height as f64 / source.height.max(1) as f64)
        .min(1.0);
    let width = (source.width as f64 * scale).round().max(1.0) as i32;
    let height = (source.height as f64 * scale).round().max(1.0) as i32;
    Rect::new(
        dest.x + (dest.width - width) / 2,
        dest.y + (dest.height - height) / 2,
        width,
        height,
    )
}

// Choose resolution before drawing any destination, so a strip tile cannot
// prepare a low-resolution texture that is subsequently enlarged for a card.
#[derive(Default)]
struct PreviewSizes {
    required: BTreeMap<WindowId, (i32, i32)>,
    endpoints: BTreeMap<WindowId, (i32, i32)>,
    planned: BTreeMap<WindowId, (i32, i32)>,
}

fn include_size(sizes: &mut BTreeMap<WindowId, (i32, i32)>, id: WindowId, dest: Rect) {
    let size = sizes.entry(id).or_default();
    size.0 = size.0.max(dest.width);
    size.1 = size.1.max(dest.height);
}

fn fitted_size(source: Rect, requested: (i32, i32)) -> (i32, i32) {
    let bounded = fit(
        source,
        Rect::new(
            0,
            0,
            requested.0.clamp(1, MAX_TEXTURE_EDGE),
            requested.1.clamp(1, MAX_TEXTURE_EDGE),
        ),
    );
    (bounded.width, bounded.height)
}

fn texture_bytes(size: (i32, i32)) -> usize {
    size.0 as usize * size.1 as usize * 4
}

impl PreviewSizes {
    fn include(&mut self, id: WindowId, dest: Rect) {
        include_size(&mut self.required, id, dest);
    }

    fn include_endpoint(&mut self, id: WindowId, dest: Rect) {
        include_size(&mut self.endpoints, id, dest);
    }

    fn texture_size(&self, id: WindowId, source: Rect) -> (i32, i32) {
        self.planned
            .get(&id)
            .copied()
            .unwrap_or_else(|| fitted_size(source, self.required[&id]))
    }

    /// Reserve every visible requirement first. Retention/prewarming may use only headroom.
    fn plan(&mut self, source: impl Fn(WindowId) -> Option<(Rect, Option<(i32, i32)>)>) {
        let mut candidates = Vec::with_capacity(self.required.len());
        let mut required_bytes = 0;
        for (&id, &required) in &self.required {
            let Some((source, previous)) = source(id) else {
                continue;
            };
            let required = fitted_size(source, required);
            required_bytes += texture_bytes(required);
            self.planned.insert(id, required);
            let endpoint = self.endpoints.get(&id).copied().unwrap_or(required);
            let preferred = fitted_size(
                source,
                (endpoint.0.max(required.0), endpoint.1.max(required.1)),
            );
            let retained = previous
                .filter(|old| old.0 >= required.0 && old.1 >= required.1)
                .unwrap_or(required);
            candidates.push((id, retained, preferred));
        }
        let mut headroom = MAX_BYTES.saturating_sub(required_bytes);
        // Keep existing sufficient allocations before admitting new endpoint upgrades.
        for (id, retained, _) in &candidates {
            let extra = texture_bytes(*retained) - texture_bytes(self.planned[id]);
            if extra <= headroom {
                self.planned.insert(*id, *retained);
                headroom -= extra;
            }
        }
        for (id, _, preferred) in candidates {
            let current = self.planned[&id];
            if preferred.0 <= current.0 && preferred.1 <= current.1 {
                continue;
            }
            let extra = texture_bytes(preferred).saturating_sub(texture_bytes(current));
            if extra <= headroom {
                self.planned.insert(id, preferred);
                headroom -= extra;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ThumbnailAppearance {
    image: WindowImageIdentity,
    size: (i32, i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PreviewDestination {
    Card,
    Miniature(WorkspaceId),
    FullscreenMiniature,
    Ghost,
    Zoom,
}

struct DestinationIds {
    body: Id,
    generation: u64,
}

struct Thumbnail {
    image: WindowImage,
    appearance: ThumbnailAppearance,
    last_use: u64,
    bytes: usize,
    destinations: BTreeMap<PreviewDestination, DestinationIds>,
}

struct CachedLabel {
    texture: GlesTexture,
    destinations: BTreeMap<(u8, u64), (Id, u64)>,
    // Hold prepared identity so allocator address reuse cannot alias an old label.
    _icon: Option<Arc<crate::runtime::titlebar::TitlebarImage>>,
}

#[derive(Default)]
pub(super) struct OverviewCache {
    thumbnails: BTreeMap<WindowId, Thumbnail>,
    labels: BTreeMap<(String, i32, Option<usize>), CachedLabel>,
    generation: u64,
    theme: Option<Theme>,
    output: Option<(crate::core::OutputId, Rect)>,
}

impl OverviewCache {
    pub fn clear(&mut self) {
        self.thumbnails.clear();
        self.labels.clear();
        self.theme = None;
        self.output = None;
    }

    fn label(
        &mut self,
        renderer: &mut GlesRenderer,
        titlebars: &mut TitlebarCache,
        text: &str,
        destination: Option<OverviewTarget>,
        rect: Rect,
        icon: Option<&Arc<crate::runtime::titlebar::TitlebarImage>>,
        opacity: f32,
    ) -> Result<Option<SceneElement>, GlesError> {
        if rect.width <= 0 || rect.height < 32 {
            return Ok(None);
        }
        let text: String = text.chars().take(256).collect();
        let width = rect.width.min(1024);
        let key = (text, width, icon.map(|i| Arc::as_ptr(i) as usize));
        if !self.labels.contains_key(&key) {
            while self.labels.len() >= MAX_LABELS
                || self
                    .labels
                    .keys()
                    .map(|k| k.1 as usize * 32 * 4)
                    .sum::<usize>()
                    + width as usize * 32 * 4
                    > MAX_LABEL_BYTES
            {
                self.labels.pop_first();
            }
            let pixels = titlebars.overview_label(
                &key.0,
                width,
                icon.map(|i| i.as_ref()),
                [235, 239, 248, 255],
            );
            let texture =
                renderer.import_memory(&pixels, Fourcc::Abgr8888, (width, 32).into(), false)?;
            self.labels.insert(
                key.clone(),
                CachedLabel {
                    texture,
                    destinations: BTreeMap::new(),
                    _icon: icon.cloned(),
                },
            );
        }
        let destination = match destination {
            Some(OverviewTarget::Window(id)) => (0, id.0),
            Some(OverviewTarget::Workspace(id)) => (1, id.0),
            Some(OverviewTarget::Fullscreen) => (3, 0),
            None => (2, 0),
        };
        let cached = self.labels.get_mut(&key).expect("prepared label");
        let identity = cached
            .destinations
            .entry(destination)
            .or_insert_with(|| (Id::new(), self.generation));
        identity.1 = self.generation;
        Ok(Some(SceneElement::Wallpaper(
            TextureRenderElement::from_static_texture(
                identity.0.clone(),
                renderer.context_id(),
                (rect.x as f64, rect.y as f64),
                cached.texture.clone(),
                1,
                Transform::Normal,
                Some(opacity),
                None,
                None,
                None,
                Kind::Unspecified,
            ),
        )))
    }

    /// Compose desktop appearance, bake its mask, then resize the complete image.
    /// No popup, scene mutation, client configure, readback, or optical filter occurs.
    fn thumbnail(
        &mut self,
        state: &Compositor,
        renderer: &mut GlesRenderer,
        titlebars: &mut TitlebarCache,
        id: WindowId,
        destination: PreviewDestination,
        dest: Rect,
        sizes: &PreviewSizes,
        shaders: &RoundedShaders,
        composer: &mut WindowImageComposer,
        output_clip: Rect,
        opacity: f32,
        offset: usize,
        filters: &mut Vec<SceneGroup>,
    ) -> Result<Vec<SceneElement>, GlesError> {
        if opacity <= 0.0 {
            return Ok(Vec::new());
        }
        // A committed surface cannot change midway through this synchronous render.
        // Largest-destination planning makes every destination share this validation.
        if !self
            .thumbnails
            .get(&id)
            .is_some_and(|cached| cached.last_use == self.generation)
        {
            let Some(prepared) = PreparedWindowImage::prepare(
                state,
                renderer,
                titlebars,
                id,
                state.runtime.desktop.focused_window() == Some(id),
            )?
            else {
                return Ok(Vec::new());
            };
            let source = prepared.identity.source;
            // Main cards and miniatures share a cache planned for the largest destination.
            let size = sizes.texture_size(id, source);
            let appearance = ThumbnailAppearance {
                image: prepared.identity.clone(),
                size,
            };
            let changed = self
                .thumbnails
                .get(&id)
                .is_none_or(|t| t.appearance != appearance);
            let bytes = texture_bytes(size);
            if changed {
                let reusable = self
                    .thumbnails
                    .remove(&id)
                    .filter(|old| old.image.size() == size)
                    .map(|old| old.image.texture);
                while self.thumbnails.len() >= MAX_ENTRIES
                    || self.thumbnails.values().map(|t| t.bytes).sum::<usize>() + bytes > MAX_BYTES
                {
                    let Some(oldest) = self
                        .thumbnails
                        .iter()
                        .min_by_key(|(id, t)| (sizes.planned.contains_key(id), t.last_use))
                        .map(|(id, _)| *id)
                    else {
                        return Ok(Vec::new());
                    };
                    self.thumbnails.remove(&oldest);
                }
                let image = composer.compose(renderer, shaders, &prepared, size, reusable)?;
                let bytes = image.bytes();
                self.thumbnails.insert(
                    id,
                    Thumbnail {
                        image,
                        appearance,
                        last_use: self.generation,
                        bytes,
                        destinations: BTreeMap::new(),
                    },
                );
            }
        }
        let theme = &state.runtime.config.theme;
        let thumbnail = self.thumbnails.get_mut(&id).expect("prepared thumbnail");
        thumbnail.last_use = self.generation;
        let identities = thumbnail
            .destinations
            .entry(destination)
            .or_insert_with(|| DestinationIds {
                body: Id::new(),
                generation: self.generation,
            });
        identities.generation = self.generation;
        let body_id = identities.body.clone();
        let dest = fit(thumbnail.image.source, dest);
        let outline = thumbnail.image.outline;
        let selected = state
            .overview_render_session()
            .is_some_and(|session| session.selected == OverviewTarget::Window(id))
            || state
                .overview_drag
                .as_ref()
                .is_some_and(|drag| drag.active && drag.window == id);
        let mut elements = Vec::with_capacity(if selected { 2 } else { 1 });
        if selected
            && let Some(body) = thumbnail.image.texture_element_identified(
                Id::new(),
                logical(dest).to_f64(),
                opacity,
            )
        {
            elements.push(SceneElement::RoundedSurface(shaders.outline_texture(
                body,
                outline.scaled(dest).selection(2),
                fade_color(theme.active_border, state.overview_progress() as f32),
                state.host_size.h,
            )));
        }
        if let Some(body) = thumbnail.image.element_identified(
            body_id,
            logical(dest).to_f64(),
            opacity,
            output_clip,
        ) {
            // Filter this destination's lower scene, never the shared client image.
            // Keep selection rings outside the optical coverage of the body.
            if theme.blur_radius > 0.0 || theme.liquid_glass.enabled {
                let start = offset + elements.len();
                filters.push(SceneGroup {
                    elements: start..start + 1,
                    clip: Rectangle::new(
                        (output_clip.x, output_clip.y).into(),
                        (output_clip.width, output_clip.height).into(),
                    ),
                    mask: Some(BlurMask::AnimatedWindow {
                        source: outline.outer,
                        destination: logical(dest).to_f64(),
                        alpha: opacity,
                    }),
                });
            }
            elements.push(SceneElement::Titlebar(body));
        }
        Ok(elements)
    }

    /// Complete overview scene, with destination-local backdrop groups.
    pub fn elements(
        &mut self,
        state: &Compositor,
        renderer: &mut GlesRenderer,
        titlebars: &mut TitlebarCache,
        shaders: &RoundedShaders,
        wallpapers: &mut WallpaperCache,
        composer: &mut WindowImageComposer,
    ) -> Result<Scene, GlesError> {
        let Some(session) = state.overview_render_session() else {
            self.clear();
            return Ok(Scene {
                elements: Vec::new(),
                groups: Vec::new(),
            });
        };
        let Some(layout) = state
            .overview_sampled_layout()
            .or_else(|| state.overview_layout())
        else {
            self.clear();
            return Ok(Scene {
                elements: Vec::new(),
                groups: Vec::new(),
            });
        };
        if self.theme.as_ref() != Some(&state.runtime.config.theme)
            || self.output != Some((session.output, layout.output))
        {
            self.clear();
            self.theme = Some(state.runtime.config.theme.clone());
            self.output = Some((session.output, layout.output));
        }
        self.generation = self.generation.wrapping_add(1);
        wallpapers.begin_previews();
        self.thumbnails.retain(|id, cached| {
            state.windows.get(id).is_some_and(|entry| {
                if !entry.mapped {
                    return false;
                }
                let size = entry.window.geometry().size;
                let height = size.h.max(1).saturating_add(if entry.uses_ssd() {
                    state.runtime.config.theme.titlebar.height
                } else {
                    0
                });
                let frame = Rect::new(0, 0, size.w.max(1), height);
                cached.appearance.image.source
                    == entry.outline(frame, &state.runtime.config.theme).outer.rect
            })
        });
        let mut sizes = PreviewSizes::default();
        let zoom = state.overview_workspace_zoom();
        if let Some(zoom) = &zoom {
            for (id, rect, _) in &zoom.windows {
                sizes.include(*id, *rect);
            }
        }
        for item in &layout.items {
            if let Some(preview) = item.preview {
                match item.target {
                    OverviewTarget::Window(id) => {
                        if !state
                            .overview_drag
                            .as_ref()
                            .is_some_and(|drag| drag.active && drag.window == id)
                        {
                            sizes.include(id, preview);
                        }
                    }
                    target @ (OverviewTarget::Workspace(_) | OverviewTarget::Fullscreen) => {
                        for (window, dest) in
                            state.overview_target_miniatures(target, preview, layout.output)
                        {
                            sizes.include(window, dest);
                        }
                    }
                }
            }
        }
        if let Some(drag) = &state.overview_drag
            && let Some(rect) = drag.ghost(layout.output)
        {
            sizes.include(drag.window, rect);
        }
        for (id, rect) in state.overview_preview_endpoints() {
            sizes.include_endpoint(id, rect);
        }
        if let Some(drag) = &state.overview_drag
            && drag.active
        {
            sizes.include_endpoint(drag.window, drag.preview);
            sizes.include_endpoint(
                drag.window,
                fit(
                    drag.preview,
                    Rect::new(
                        0,
                        0,
                        layout.output.width.min(240),
                        layout.output.height.min(160),
                    ),
                ),
            );
        }
        sizes.plan(|id| {
            let entry = state.windows.get(&id).filter(|entry| entry.mapped)?;
            let geometry = entry.window.geometry().size;
            let height = geometry.h.max(1).saturating_add(if entry.uses_ssd() {
                state.runtime.config.theme.titlebar.height
            } else {
                0
            });
            let frame = Rect::new(0, 0, geometry.w.max(1), height);
            let source = entry.outline(frame, &state.runtime.config.theme).outer.rect;
            Some((
                source,
                self.thumbnails.get(&id).map(|cached| cached.image.size()),
            ))
        });
        // Reclaim capacity changes together before admission, so an early
        // endpoint upgrade cannot evict a later visible image awaiting shrink.
        self.thumbnails.retain(|id, cached| {
            sizes
                .planned
                .get(id)
                .is_none_or(|size| cached.image.size() == *size)
        });
        let mut elements = Vec::new();
        let mut filters = Vec::new();
        let region = state
            .outputs
            .iter()
            .find(|r| r.id == session.output)
            .expect("overview output");
        if let Some(zoom) = &zoom {
            for (id, rect, opacity) in &zoom.windows {
                match self.thumbnail(
                    state,
                    renderer,
                    titlebars,
                    *id,
                    PreviewDestination::Zoom,
                    *rect,
                    &sizes,
                    shaders,
                    composer,
                    zoom.canvas,
                    *opacity,
                    elements.len(),
                    &mut filters,
                ) {
                    Ok(preview) => elements.extend(preview),
                    Err(error) => tracing_fallback(error),
                }
            }
            desktop_backing(
                &mut elements,
                wallpapers,
                renderer,
                state,
                &region.output.name(),
                layout.output,
                WallpaperDestination::Zoom,
                zoom.canvas,
                zoom.opacity,
            );
        }
        // Keep the drop target outline above the floating ghost so a center-grab
        // cannot cover every pixel of the eligible desktop's highlight.
        if let Some(drag) = &state.overview_drag
            && drag.active
            && let Some(destination) = drag.destination
            && let Some(preview) = layout
                .items
                .iter()
                .find(|i| i.target == OverviewTarget::Workspace(destination))
                .and_then(|i| i.preview)
        {
            ring(
                &mut elements,
                preview,
                3,
                state.runtime.config.theme.active_border,
                layout.output,
            );
        }
        if let Some(drag) = &state.overview_drag
            && let Some(rect) = drag.ghost(layout.output)
        {
            let start = elements.len();
            match self.thumbnail(
                state,
                renderer,
                titlebars,
                drag.window,
                PreviewDestination::Ghost,
                rect,
                &sizes,
                shaders,
                composer,
                layout.output,
                state.overview_window_opacity(drag.window),
                elements.len(),
                &mut filters,
            ) {
                Ok(preview) => elements.extend(preview),
                Err(error) => tracing_fallback(error),
            }
            if elements.len() == start {
                // Keep an outline fallback when the client tree cannot render.
                ring(
                    &mut elements,
                    rect,
                    2,
                    state.runtime.config.theme.active_border,
                    layout.output,
                );
            }
        }
        for item in &layout.items {
            if state.overview_drag.as_ref().is_some_and(|drag| {
                drag.active && item.target == OverviewTarget::Window(drag.window)
            }) {
                continue;
            }
            let label = match item.target {
                target @ (OverviewTarget::Workspace(_) | OverviewTarget::Fullscreen) => {
                    if let Some(preview) = item.preview
                        && zoom.as_ref().is_none_or(|zoom| zoom.target != target)
                    {
                        // Small desktop previews use remembered frames, not new
                        // policy placements or thumbnail-sized configures.
                        for (window, dest) in
                            state.overview_target_miniatures(target, preview, layout.output)
                        {
                            match self.thumbnail(
                                state,
                                renderer,
                                titlebars,
                                window,
                                match target {
                                    OverviewTarget::Workspace(id) => {
                                        PreviewDestination::Miniature(id)
                                    }
                                    _ => PreviewDestination::FullscreenMiniature,
                                },
                                dest,
                                &sizes,
                                shaders,
                                composer,
                                preview
                                    .intersection(layout.output)
                                    .expect("visible miniature"),
                                state.overview_progress() as f32,
                                elements.len(),
                                &mut filters,
                            ) {
                                Ok(preview) => elements.extend(preview),
                                Err(error) => {
                                    self.thumbnails.remove(&window);
                                    tracing_fallback(error);
                                }
                            }
                        }
                        desktop_backing(
                            &mut elements,
                            wallpapers,
                            renderer,
                            state,
                            &region.output.name(),
                            layout.output,
                            match target {
                                OverviewTarget::Workspace(id) => {
                                    WallpaperDestination::Miniature(id)
                                }
                                _ => WallpaperDestination::FullscreenMiniature,
                            },
                            preview,
                            state.overview_progress() as f32,
                        );
                        ring(
                            &mut elements,
                            preview,
                            2,
                            fade_color(
                                if target == session.desktop_target() {
                                    state.runtime.config.theme.active_border
                                } else {
                                    [0.35, 0.38, 0.44, 0.8]
                                },
                                state.overview_progress() as f32,
                            ),
                            layout.output,
                        );
                    }
                    match target {
                        OverviewTarget::Workspace(id) => state
                            .runtime
                            .desktop
                            .workspace(id)
                            .map(|w| w.name.clone())
                            .unwrap_or_default(),
                        _ => "Fullscreen".into(),
                    }
                }
                OverviewTarget::Window(id) => {
                    if let Some(preview) = item.preview {
                        let start = elements.len();
                        match self.thumbnail(
                            state,
                            renderer,
                            titlebars,
                            id,
                            PreviewDestination::Card,
                            preview,
                            &sizes,
                            shaders,
                            composer,
                            layout.output,
                            state.overview_window_opacity(id),
                            elements.len(),
                            &mut filters,
                        ) {
                            Ok(preview) => elements.extend(preview),
                            Err(error) => {
                                self.thumbnails.remove(&id);
                                tracing_fallback(error);
                            }
                        }
                        if elements.len() == start && item.target == session.selected {
                            ring(
                                &mut elements,
                                preview,
                                2,
                                state.runtime.config.theme.active_border,
                                layout.output,
                            );
                        }
                    }
                    state
                        .runtime
                        .desktop
                        .window(id)
                        .map(|w| {
                            format!(
                                "{}{}",
                                if w.minimized { "Minimized · " } else { "" },
                                if !w.title.is_empty() {
                                    &w.title
                                } else if !w.app_id.is_empty() {
                                    &w.app_id
                                } else {
                                    "Untitled"
                                }
                            )
                        })
                        .unwrap_or_default()
                }
            };
            let label_rect = Rect::new(
                item.rect.x,
                item.preview.map_or(item.rect.y, |p| p.bottom() + 4),
                item.rect.width.min(360),
                32,
            )
            .intersection(layout.output);
            let icon = match item.target {
                OverviewTarget::Window(id) => state
                    .runtime
                    .desktop
                    .window(id)
                    .and_then(|w| state.runtime.titlebar_assets.app_icon(&w.app_id)),
                _ => None,
            };
            if let Some(mut label_rect) = label_rect {
                label_rect.x = item.rect.x + (item.rect.width - label_rect.width) / 2;
                if let Some(label) = self.label(
                    renderer,
                    titlebars,
                    &label,
                    Some(item.target),
                    label_rect,
                    icon,
                    state.overview_progress() as f32,
                )? {
                    elements.push(label);
                }
                if matches!(item.target, OverviewTarget::Window(_)) {
                    solid(
                        &mut elements,
                        label_rect,
                        [0.025, 0.03, 0.045, 0.85 * state.overview_progress() as f32],
                    );
                }
            }
        }
        if session.windows.is_empty() || layout.pages > 1 {
            let status = if session.windows.is_empty() {
                "Empty workspace".to_owned()
            } else {
                format!(
                    "Page {} of {} · Tab / arrows to select",
                    layout.page + 1,
                    layout.pages
                )
            };
            let width = layout.canvas.width.min(320);
            let status_rect = Rect::new(
                layout.canvas.x + (layout.canvas.width - width) / 2,
                layout.canvas.bottom() - 36,
                width,
                32,
            )
            .intersection(layout.output);
            if let Some(rect) = status_rect {
                if let Some(label) = self.label(
                    renderer,
                    titlebars,
                    &status,
                    None,
                    rect,
                    None,
                    state.overview_progress() as f32,
                )? {
                    elements.push(label);
                }
                solid(
                    &mut elements,
                    rect,
                    [0.025, 0.03, 0.045, 0.85 * state.overview_progress() as f32],
                );
            }
        }
        desktop_backing(
            &mut elements,
            wallpapers,
            renderer,
            state,
            &region.output.name(),
            layout.output,
            WallpaperDestination::Canvas,
            layout.canvas,
            1.0,
        );
        for region in &state.outputs {
            // Cover the ordinary scene with wallpaper before dimming it: panels
            // and full-sized windows must not leak through the overview backdrop.
            solid(
                &mut elements,
                region.rect,
                [
                    0.015,
                    0.02,
                    0.035,
                    if region.id == session.output {
                        0.65 * state.overview_progress() as f32
                    } else {
                        0.85 * state.overview_progress() as f32
                    },
                ],
            );
            desktop_backing(
                &mut elements,
                wallpapers,
                renderer,
                state,
                &region.output.name(),
                region.rect,
                WallpaperDestination::Background,
                region.rect,
                1.0,
            );
        }
        wallpapers.finish_previews();
        for label in self.labels.values_mut() {
            label
                .destinations
                .retain(|_, (_, generation)| *generation == self.generation);
        }
        for thumbnail in self.thumbnails.values_mut() {
            thumbnail
                .destinations
                .retain(|_, ids| ids.generation == self.generation);
        }
        Ok(overview_scene(
            elements,
            filters,
            Rectangle::from_size((state.host_size.w, state.host_size.h).into()),
        ))
    }
}

// Cover all unfiltered spans as well: backdrop composition walks groups in reverse.
fn overview_scene(
    elements: Vec<SceneElement>,
    filters: Vec<SceneGroup>,
    clip: Rectangle<i32, Physical>,
) -> Scene {
    let mut groups = Vec::with_capacity(filters.len() * 2 + 1);
    let mut start = 0;
    for filter in filters {
        debug_assert!(filter.elements.start >= start && filter.elements.end <= elements.len());
        if start < filter.elements.start {
            groups.push(SceneGroup {
                elements: start..filter.elements.start,
                clip,
                mask: None,
            });
        }
        start = filter.elements.end;
        groups.push(filter);
    }
    if start < elements.len() {
        groups.push(SceneGroup {
            elements: start..elements.len(),
            clip,
            mask: None,
        });
    }
    Scene { elements, groups }
}

fn tracing_fallback(error: GlesError) {
    // No thumbnail is required for policy/input to remain usable.
    eprintln!("clear: overview thumbnail unavailable: {error}");
}

pub(super) fn miniature_frame(frame: Rect, output: Rect, preview: Rect) -> Option<Rect> {
    let frame = frame.intersection(output)?;
    let sx = preview.width as f64 / output.width.max(1) as f64;
    let sy = preview.height as f64 / output.height.max(1) as f64;
    Rect::new(
        preview.x + ((frame.x - output.x) as f64 * sx).round() as i32,
        preview.y + ((frame.y - output.y) as f64 * sy).round() as i32,
        (frame.width as f64 * sx).round().max(1.0) as i32,
        (frame.height as f64 * sy).round().max(1.0) as i32,
    )
    .intersection(preview)
}

fn desktop_backing(
    elements: &mut Vec<SceneElement>,
    wallpapers: &mut WallpaperCache,
    renderer: &mut GlesRenderer,
    state: &Compositor,
    output: &str,
    source: Rect,
    destination: WallpaperDestination,
    dest: Rect,
    opacity: f32,
) {
    if dest.width <= 0 || dest.height <= 0 {
        return;
    }
    let fullscreen = destination == WallpaperDestination::FullscreenMiniature
        || (destination == WallpaperDestination::Canvas
            && state
                .overview_render_session()
                .is_some_and(|s| s.fullscreen_selected))
        || (destination == WallpaperDestination::Zoom
            && state
                .overview_workspace_zoom()
                .is_some_and(|z| z.target == OverviewTarget::Fullscreen));
    if fullscreen {
        solid(elements, dest, [0.0, 0.0, 0.0, opacity]);
        return;
    }
    if let Some(element) = wallpapers.preview_element(
        renderer,
        &state.runtime.wallpapers,
        output,
        source,
        destination,
        dest,
        opacity,
    ) {
        elements.push(SceneElement::Wallpaper(element));
    }
    let mut background = state.runtime.config.theme.background;
    background[3] = opacity;
    solid(elements, dest, background);
}

fn fade_color(mut color: [f32; 4], opacity: f32) -> [f32; 4] {
    color[3] *= opacity;
    color
}

fn ring(elements: &mut Vec<SceneElement>, rect: Rect, width: i32, color: [f32; 4], clip: Rect) {
    for edge in [
        Rect::new(
            rect.x - width,
            rect.y - width,
            rect.width + 2 * width,
            width,
        ),
        Rect::new(rect.x - width, rect.bottom(), rect.width + 2 * width, width),
        Rect::new(rect.x - width, rect.y, width, rect.height),
        Rect::new(rect.right(), rect.y, width, rect.height),
    ] {
        if let Some(edge) = edge.intersection(clip) {
            solid(elements, edge, color);
        }
    }
}

fn solid(elements: &mut Vec<SceneElement>, rect: Rect, color: [f32; 4]) {
    if rect.width <= 0 || rect.height <= 0 {
        return;
    }
    use smithay::backend::renderer::element::solid::{SolidColorBuffer, SolidColorRenderElement};
    let buffer = SolidColorBuffer::new((rect.width, rect.height), premultiply(color));
    elements.push(SceneElement::Border(SolidColorRenderElement::from_buffer(
        &buffer,
        (rect.x, rect.y),
        1.0,
        1.0,
        Kind::Unspecified,
    )));
}

impl Compositor {
    pub(super) fn overview_target_miniatures(
        &self,
        target: OverviewTarget,
        preview: Rect,
        output: Rect,
    ) -> Vec<(WindowId, Rect)> {
        match target {
            OverviewTarget::Workspace(id) => self.overview_miniatures(id, preview, output),
            OverviewTarget::Fullscreen => {
                let Some(slot) = self.runtime.desktop.fullscreen_slot() else {
                    return Vec::new();
                };
                let bounds = slot
                    .output
                    .and_then(|o| self.runtime.desktop.output(o))
                    .map_or(output, |o| o.bounds);
                std::iter::once(slot.window)
                    .chain(
                        self.runtime
                            .desktop
                            .windows()
                            .filter(|w| {
                                !w.minimized && self.runtime.desktop.is_fullscreen_transient(w.id)
                            })
                            .map(|w| w.id),
                    )
                    .take(4)
                    .filter_map(|id| {
                        let entry = self.windows.get(&id).filter(|e| e.mapped)?;
                        let frame = if id == slot.window {
                            bounds
                        } else {
                            entry
                                .last_frame
                                .get()
                                .unwrap_or(self.runtime.desktop.window(id)?.floating_rect)
                        };
                        miniature_frame(frame, bounds, preview).map(|rect| (id, rect))
                    })
                    .collect()
            }
            OverviewTarget::Window(_) => Vec::new(),
        }
    }
    /// The same bounded miniature list drives rendering and client visibility.
    pub fn overview_miniatures(
        &self,
        workspace: WorkspaceId,
        preview: Rect,
        output: Rect,
    ) -> Vec<(WindowId, Rect)> {
        let Some(workspace) = self.runtime.desktop.workspace(workspace) else {
            return Vec::new();
        };
        workspace
            .windows()
            .iter()
            .filter(|id| {
                self.runtime.desktop.window(**id).is_some_and(|w| {
                    w.role == crate::core::WindowRole::Normal
                        && !w.minimized
                        && !w.fullscreen
                        && !self.runtime.desktop.is_fullscreen_transient(w.id)
                })
            })
            .take(4)
            .filter_map(|id| {
                let entry = self.windows.get(id).filter(|e| e.mapped)?;
                let window = self.runtime.desktop.window(*id)?;
                let home = self
                    .outputs
                    .iter()
                    .find(|r| Some(r.id) == window.output)
                    .map_or(output, |r| r.rect);
                let frame = entry.last_frame.get().unwrap_or(window.floating_rect);
                miniature_frame(frame, home, preview).map(|dest| (*id, dest))
            })
            .collect()
    }

    /// Only clients represented by a visible card, miniature or drag ghost are live.
    pub fn overview_live_windows(&self) -> BTreeSet<WindowId> {
        let Some(layout) = self
            .overview_sampled_layout()
            .or_else(|| self.overview_layout())
        else {
            return BTreeSet::new();
        };
        let mut windows = BTreeSet::new();
        if let Some(zoom) = self.overview_workspace_zoom() {
            windows.extend(zoom.windows.into_iter().map(|(id, _, _)| id));
        }
        for item in &layout.items {
            if let Some(preview) = item.preview {
                match item.target {
                    OverviewTarget::Window(id) => {
                        windows.insert(id);
                    }
                    target @ (OverviewTarget::Workspace(_) | OverviewTarget::Fullscreen) => {
                        windows.extend(
                            self.overview_target_miniatures(target, preview, layout.output)
                                .into_iter()
                                .map(|(id, _)| id),
                        );
                    }
                }
            }
        }
        if let Some(drag) = &self.overview_drag
            && drag.ghost(layout.output).is_some()
        {
            windows.insert(drag.window);
        }
        windows.retain(|id| self.windows.get(id).is_some_and(|entry| entry.mapped));
        windows
    }

    pub fn overview_layout(&self) -> Option<OverviewLayout> {
        self.overview_displayed_layout().or_else(|| {
            self.overview_render_session()
                .and_then(|session| self.overview_target_layout(session))
        })
    }

    pub fn overview_target_layout(&self, session: &OverviewSession) -> Option<OverviewLayout> {
        let region = self.outputs.iter().find(|r| r.id == session.output)?;
        let mut layout = OverviewLayout::new(session, region.rect);
        layout.fit_previews(|id| {
            let entry = self.windows.get(&id)?;
            let size = entry.window.geometry().size;
            let height = size.h.max(1).saturating_add(if entry.uses_ssd() {
                self.runtime.config.theme.titlebar.height
            } else {
                0
            });
            Some(
                entry
                    .outline(
                        Rect::new(0, 0, size.w.max(1), height),
                        &self.runtime.config.theme,
                    )
                    .outer
                    .rect,
            )
        });
        Some(layout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::{Desktop, OutputId},
        runtime::OverviewNavigation,
    };
    use smithay::backend::renderer::utils::CommitCounter;

    fn session(windows: usize) -> (Desktop, OverviewSession) {
        let mut d = Desktop::new();
        d.add_output(OutputId(1), "test".into(), Rect::new(319, 11, 641, 481));
        for i in 1..=windows {
            d.add_window(WindowId(i as u64), "".into(), "".into());
        }
        let s = OverviewSession::new(&d).unwrap();
        (d, s)
    }

    #[test]
    fn overview_filter_groups_preserve_unfiltered_spans_and_destination_masks() {
        let clip = Rectangle::from_size((640, 480).into());
        let miniature_clip = Rectangle::new((70, 6).into(), (45, 34).into());
        let source = super::super::rounded::RoundedShape {
            rect: Rect::new(0, 32, 640, 448),
            radii: [24.0, 0.0, 16.0, 0.0],
        };
        let mut elements = Vec::new();
        for _ in 0..7 {
            solid(&mut elements, Rect::new(0, 0, 1, 1), [1.0; 4]);
        }
        let filters = vec![
            SceneGroup {
                elements: 1..2,
                clip,
                mask: Some(BlurMask::AnimatedWindow {
                    source,
                    destination: logical(Rect::new(100, 110, 417, 292)).to_f64(),
                    alpha: 0.4,
                }),
            },
            SceneGroup {
                elements: 4..5,
                clip: miniature_clip,
                mask: Some(BlurMask::AnimatedWindow {
                    source,
                    destination: logical(Rect::new(70, 8, 45, 32)).to_f64(),
                    alpha: 1.0,
                }),
            },
        ];
        let scene = overview_scene(elements, filters, clip);
        assert_eq!(
            scene
                .groups
                .iter()
                .map(|g| g.elements.clone())
                .collect::<Vec<_>>(),
            vec![0..1, 1..2, 2..4, 4..5, 5..7]
        );
        assert_eq!(scene.groups[3].clip, miniature_clip);
        assert!(matches!(
            scene.groups[1].mask,
            Some(BlurMask::AnimatedWindow { alpha: 0.4, .. })
        ));
        for index in [0, 2, 4] {
            assert!(scene.groups[index].mask.is_none());
        }
        // Disabled optics still cover every backing/label/ring in one group.
        let plain = overview_scene(scene.elements, Vec::new(), clip);
        assert_eq!(plain.groups.len(), 1);
        assert_eq!(plain.groups[0].elements, 0..7);
        assert!(plain.groups[0].mask.is_none());
    }

    #[test]
    fn card_and_workspace_hits_share_half_open_output_local_boxes() {
        let (_, s) = session(3);
        let output = Rect::new(319, 11, 641, 481);
        let layout = OverviewLayout::new(&s, output);
        for item in &layout.items {
            assert_eq!(item.rect.intersection(output), Some(item.rect));
            assert_eq!(
                layout.hit(item.rect.x as f64 + 0.5, item.rect.y as f64 + 0.5),
                Some(item.target)
            );
            assert_ne!(
                layout.hit(item.rect.right() as f64, item.rect.bottom() as f64),
                Some(item.target)
            );
        }
        assert_eq!(layout.hit(318.9, 50.0), None);
        assert_eq!(layout.hit(f64::NAN, 50.0), None);
        assert_eq!(layout.hit(500.0, f64::INFINITY), None);
    }

    #[test]
    fn desktop_strip_is_compact_and_frames_preserve_relative_sizes() {
        let (_, s) = session(2);
        let output = Rect::new(319, 11, 1280, 720);
        let mut layout = OverviewLayout::new(&s, output);
        layout.fit_previews(|id| {
            Some(if id == WindowId(1) {
                Rect::new(0, 0, 800, 600)
            } else {
                Rect::new(0, 0, 400, 300)
            })
        });
        let cards: Vec<_> = layout
            .items
            .iter()
            .filter(|i| matches!(i.target, OverviewTarget::Window(_)))
            .collect();
        let large = cards[0].preview.unwrap();
        let small = cards[1].preview.unwrap();
        assert!((large.width - 2 * small.width).abs() <= 1);
        assert!((large.height - 2 * small.height).abs() <= 1);
        assert!(cards[0].rect.intersection(cards[1].rect).is_none());
        assert_eq!(large.intersection(layout.canvas), Some(large));
        assert_eq!(small.intersection(layout.canvas), Some(small));
        for item in &layout.items {
            if matches!(item.target, OverviewTarget::Workspace(_)) {
                assert!(item.rect.width < output.width / 10);
                assert!(item.rect.bottom() < layout.canvas.y);
            }
            let preview = item.preview.unwrap();
            assert_eq!(
                layout.hit(
                    (preview.x + preview.width / 2) as f64,
                    (preview.y + preview.height / 2) as f64
                ),
                Some(item.target)
            );
        }
    }

    #[test]
    fn remembered_miniature_frames_are_bounded_even_when_floating_off_output() {
        let output = Rect::new(319, 11, 641, 481);
        let preview = Rect::new(50, 10, 80, 60);
        for frame in [
            Rect::new(319, 11, 641, 481),
            Rect::new(-100, -100, 800, 600),
            Rect::new(900, 450, 800, 600),
        ] {
            let dest = miniature_frame(frame, output, preview).unwrap();
            assert_eq!(dest.intersection(preview), Some(dest));
        }
        assert!(miniature_frame(Rect::new(-1000, -1000, 20, 20), output, preview).is_none());
    }

    #[test]
    fn pages_keep_every_selected_window_and_workspace_accessible() {
        let (d, mut s) = session(100);
        for _ in 0..101 {
            let layout = OverviewLayout::new(&s, Rect::new(319, 11, 641, 481));
            assert!(layout.items.iter().any(|i| i.target == s.selected));
            assert!(
                layout
                    .items
                    .iter()
                    .filter(|i| matches!(i.target, OverviewTarget::Window(_)))
                    .count()
                    <= 12
            );
            s.navigate(&d, OverviewNavigation::Next, layout.columns);
        }
        for _ in 0..9 {
            s.navigate(&d, OverviewNavigation::WorkspaceNext, 1);
            let layout = OverviewLayout::new(&s, Rect::new(0, 0, 90, 80));
            assert!(
                layout
                    .items
                    .iter()
                    .any(|i| i.target == OverviewTarget::Workspace(s.workspace))
            );
        }
    }

    #[test]
    fn tiny_outputs_never_draw_or_hit_a_neighbouring_output() {
        let (_, s) = session(3);
        for width in [1, 2, 8, 40, 100] {
            for height in [1, 2, 8, 40, 100] {
                let output = Rect::new(319, 11, width, height);
                let layout = OverviewLayout::new(&s, output);
                for item in layout.items {
                    if item.rect.width > 0 && item.rect.height > 0 {
                        assert_eq!(item.rect.intersection(output), Some(item.rect));
                    }
                    if let Some(preview) = item.preview {
                        assert_eq!(preview.intersection(output), Some(preview));
                    }
                }
            }
        }
    }

    #[test]
    fn preview_resolution_uses_the_largest_destination_without_upscaling_sources() {
        let id = WindowId(1);
        let source = Rect::new(0, 0, 1600, 1000);
        let mut sizes = PreviewSizes::default();
        sizes.include(id, Rect::new(0, 0, 64, 40));
        sizes.include(id, Rect::new(0, 0, 960, 600));
        sizes.include(id, Rect::new(0, 0, 240, 150));
        assert_eq!(sizes.texture_size(id, source), (960, 600));
        sizes.include(id, Rect::new(0, 0, 2000, 1250));
        assert_eq!(sizes.texture_size(id, source), (1600, 1000));
        assert_eq!(
            sizes.texture_size(id, Rect::new(0, 0, 32768, 32768)),
            (1250, 1250)
        );
        sizes.include(id, Rect::new(0, 0, 32768, 32768));
        let size = sizes.texture_size(id, source);
        assert_eq!(size, (1600, 1000));
        assert_eq!(
            sizes.texture_size(id, Rect::new(0, 0, 32768, 32768)),
            (MAX_TEXTURE_EDGE, MAX_TEXTURE_EDGE)
        );
    }

    #[test]
    fn transition_endpoints_keep_resolution_stable_across_sampled_sizes() {
        let id = WindowId(1);
        let source = Rect::new(0, 0, 1600, 1000);
        for width in [64, 320, 800, 1280, 320] {
            let mut sizes = PreviewSizes::default();
            sizes.include(id, Rect::new(0, 0, width, width * 5 / 8));
            sizes.include_endpoint(id, Rect::new(0, 0, 1280, 800));
            sizes.plan(|_| Some((source, None)));
            assert_eq!(sizes.texture_size(id, source), (1280, 800));
        }
        let mut sizes = PreviewSizes::default();
        sizes.include(id, Rect::new(0, 0, 240, 150));
        sizes.plan(|_| Some((source, Some((1280, 800)))));
        assert_eq!(sizes.texture_size(id, source), (1280, 800));
        sizes.include(id, Rect::new(0, 0, 1440, 900));
        sizes.plan(|_| Some((source, Some((1280, 800)))));
        assert_eq!(sizes.texture_size(id, source), (1440, 900));
    }

    #[test]
    fn endpoint_upgrades_reserve_all_visible_requirements_before_using_headroom() {
        let source = Rect::new(0, 0, 32768, 32768);
        let mut sizes = PreviewSizes::default();
        for window in 1..=12 {
            let id = WindowId(window);
            sizes.include(id, Rect::new(0, 0, 320, 320));
            sizes.include_endpoint(id, source);
        }
        sizes.plan(|_| Some((source, None)));
        assert!(
            sizes
                .planned
                .values()
                .copied()
                .map(texture_bytes)
                .sum::<usize>()
                <= MAX_BYTES
        );
        assert!(
            sizes
                .planned
                .values()
                .any(|&size| size == (MAX_TEXTURE_EDGE, MAX_TEXTURE_EDGE))
        );
        assert!(sizes.planned.values().any(|&size| size == (320, 320)));
        for size in sizes.planned.values() {
            assert!(size.0 >= 320 && size.1 >= 320);
            assert!(size.0 <= MAX_TEXTURE_EDGE && size.1 <= MAX_TEXTURE_EDGE);
        }
        let previous = sizes.planned.clone();
        // Small cards after a transition retain only capacity that still fits
        // alongside every visible requirement, instead of evicting one another.
        let mut next = PreviewSizes::default();
        for window in 1..=12 {
            next.include(WindowId(window), Rect::new(0, 0, 160, 160));
        }
        next.plan(|id| Some((source, previous.get(&id).copied())));
        assert_eq!(next.planned, previous);
    }

    #[test]
    fn retained_capacity_yields_to_growing_visible_requirements() {
        let source = Rect::new(0, 0, 2048, 2048);
        let mut sizes = PreviewSizes::default();
        for window in 1..=12 {
            sizes.include(WindowId(window), Rect::new(0, 0, 1024, 1024));
        }
        sizes.plan(|_| Some((source, Some((2048, 2048)))));
        assert!(
            sizes
                .planned
                .values()
                .copied()
                .map(texture_bytes)
                .sum::<usize>()
                <= MAX_BYTES
        );
        assert!(sizes.planned.values().any(|&size| size == (1024, 1024)));
        assert!(
            sizes
                .planned
                .values()
                .all(|size| size.0 >= 1024 && size.1 >= 1024)
        );
    }

    #[test]
    fn oversized_visible_working_set_receives_no_optional_capacity_or_under_resolution() {
        let source = Rect::new(0, 0, 2048, 2048);
        let mut sizes = PreviewSizes::default();
        for window in 1..=12 {
            let id = WindowId(window);
            sizes.include(id, Rect::new(0, 0, 1600, 1600));
            sizes.include_endpoint(id, source);
        }
        sizes.plan(|_| Some((source, Some((2048, 2048)))));
        assert!(
            sizes
                .planned
                .values()
                .copied()
                .map(texture_bytes)
                .sum::<usize>()
                > MAX_BYTES
        );
        assert!(sizes.planned.values().all(|&size| size == (1600, 1600)));
    }

    #[test]
    fn thumbnails_keep_aspect_ratio_and_texture_limits_for_oversized_frames() {
        for source in [
            Rect::new(0, 0, 32768, 32768),
            Rect::new(0, 0, 20000, 32),
            Rect::new(0, 0, 20, 32000),
        ] {
            let dest = Rect::new(10, 20, 512, 320);
            let fitted = fit(source, dest);
            assert_eq!(fitted.intersection(dest), Some(fitted));
            assert!(fitted.width <= 512 && fitted.height <= 320);
        }
    }

    fn unchanged_damage_preview() -> ThumbnailAppearance {
        ThumbnailAppearance {
            image: WindowImageIdentity {
                effective_revision: 1,
                signature: vec![(Id::new(), CommitCounter::default())],
                mappings: Vec::new(),
                geometry_origin: (0, 0),
                border_color: [0.2, 0.3, 0.4, 1.0],
                source: Rect::new(0, 0, 640, 480),
                frame: Rect::new(0, 0, 640, 480),
                outlines: [(Rect::new(0, 0, 640, 480), [0.0; 4]); 2],
                committed_fullscreen: false,
            },
            size: (320, 240),
        }
    }

    #[test]
    fn newly_attached_undamaged_buffer_invalidates_preview_effective_revision() {
        let previous = unchanged_damage_preview();
        let mut replaced = previous.clone();
        replaced.image.effective_revision += 1;
        assert_eq!(previous.image.signature, replaced.image.signature);
        assert_eq!(previous.image.mappings, replaced.image.mappings);
        assert_eq!(previous.image.source, replaced.image.source);
        assert_ne!(previous, replaced);
    }

    #[test]
    fn fullscreen_root_commit_invalidates_preview_without_buffer_damage() {
        let ordinary = unchanged_damage_preview();
        let mut fullscreen = ordinary.clone();
        fullscreen.image.committed_fullscreen = true;
        // A CSD root can accept fullscreen while keeping its current buffer.
        // With zero border width, source/texture geometry and damage are unchanged.
        assert_eq!(ordinary.image.signature, fullscreen.image.signature);
        assert_eq!(ordinary.image.source, fullscreen.image.source);
        assert_ne!(ordinary, fullscreen);
        fullscreen.image.committed_fullscreen = false;
        assert_eq!(ordinary, fullscreen);
    }
}
