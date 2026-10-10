//! Compositor-owned card layout, hits, and bounded renderer-local resources.

use super::{
    rounded::RoundedShaders,
    scene::{SceneElement, logical, premultiply},
    state::Compositor,
    titlebar::TitlebarCache,
    wallpaper::WallpaperCache,
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
    utils::Transform,
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
        let workspace_index = session
            .workspaces
            .iter()
            .position(|w| *w == session.workspace)
            .unwrap_or(0);
        let start = workspace_index
            .saturating_sub(slots / 2)
            .min(session.workspaces.len().saturating_sub(slots));
        let visible = session
            .workspaces
            .iter()
            .skip(start)
            .take(slots)
            .count()
            .max(1);
        let mut items = Vec::new();
        let strip_width = visible as i32 * (tile_width + gap) - gap;
        let strip_left = output.x + (output.width - strip_width) / 2;
        for (i, workspace) in session
            .workspaces
            .iter()
            .skip(start)
            .take(slots)
            .enumerate()
        {
            let preview = Rect::new(
                strip_left + i as i32 * (tile_width + gap),
                output.y + strip_top,
                tile_width,
                strip_height,
            );
            items.push(OverviewItem {
                target: OverviewTarget::Workspace(*workspace),
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
struct PreviewSizes(BTreeMap<WindowId, (i32, i32)>);

impl PreviewSizes {
    fn include(&mut self, id: WindowId, dest: Rect) {
        let size = self.0.entry(id).or_default();
        size.0 = size.0.max(dest.width);
        size.1 = size.1.max(dest.height);
    }

    fn texture_size(&self, id: WindowId, source: Rect) -> (i32, i32) {
        let requested = self.0[&id];
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
}

#[derive(Debug, Clone, PartialEq)]
struct ThumbnailAppearance {
    image: WindowImageIdentity,
    size: (i32, i32),
}

struct Thumbnail {
    image: WindowImage,
    appearance: ThumbnailAppearance,
    last_use: u64,
    bytes: usize,
}

struct CachedLabel {
    texture: GlesTexture,
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
                    _icon: icon.cloned(),
                },
            );
        }
        Ok(Some(SceneElement::Wallpaper(
            TextureRenderElement::from_static_texture(
                Id::new(),
                renderer.context_id(),
                (rect.x as f64, rect.y as f64),
                self.labels[&key].texture.clone(),
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
        dest: Rect,
        sizes: &PreviewSizes,
        shaders: &RoundedShaders,
        composer: &mut WindowImageComposer,
        output_clip: Rect,
        opacity: f32,
    ) -> Result<Vec<SceneElement>, GlesError> {
        if opacity <= 0.0 {
            return Ok(Vec::new());
        }
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
        let theme = &state.runtime.config.theme;
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
        let bytes = size.0 as usize * size.1 as usize * 4;
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
                    .min_by_key(|(_, t)| t.last_use)
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
                },
            );
        }
        let thumbnail = self.thumbnails.get_mut(&id).expect("prepared thumbnail");
        thumbnail.last_use = self.generation;
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
            && let Some(body) = thumbnail
                .image
                .texture_element(logical(dest).to_f64(), opacity)
        {
            elements.push(SceneElement::RoundedSurface(shaders.outline_texture(
                body,
                outline.scaled(dest).selection(2),
                fade_color(theme.active_border, state.overview_progress() as f32),
                state.host_size.h,
            )));
        }
        if let Some(body) = thumbnail
            .image
            .element(logical(dest).to_f64(), opacity, output_clip)
        {
            elements.push(SceneElement::Titlebar(body));
        }
        Ok(elements)
    }

    /// Final compositor pass, front-to-back, covering all ordinary layer clients.
    pub fn elements(
        &mut self,
        state: &Compositor,
        renderer: &mut GlesRenderer,
        titlebars: &mut TitlebarCache,
        shaders: &RoundedShaders,
        wallpapers: &mut WallpaperCache,
        composer: &mut WindowImageComposer,
    ) -> Result<Vec<SceneElement>, GlesError> {
        let Some(session) = state.overview_render_session() else {
            self.clear();
            return Ok(Vec::new());
        };
        let Some(layout) = state
            .overview_sampled_layout()
            .or_else(|| state.overview_layout())
        else {
            self.clear();
            return Ok(Vec::new());
        };
        if self.theme.as_ref() != Some(&state.runtime.config.theme)
            || self.output != Some((session.output, layout.output))
        {
            self.clear();
            self.theme = Some(state.runtime.config.theme.clone());
            self.output = Some((session.output, layout.output));
        }
        self.generation = self.generation.wrapping_add(1);
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
                    OverviewTarget::Workspace(id) => {
                        for (window, dest) in state.overview_miniatures(id, preview, layout.output)
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
        let mut elements = Vec::new();
        let region = state
            .outputs
            .iter()
            .find(|r| r.id == session.output)
            .expect("overview output");
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
                rect,
                &sizes,
                shaders,
                composer,
                layout.output,
                state.overview_window_opacity(drag.window),
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
                OverviewTarget::Workspace(id) => {
                    if let Some(preview) = item.preview {
                        // Small desktop previews use remembered frames, not new
                        // policy placements or thumbnail-sized configures.
                        for (window, dest) in state.overview_miniatures(id, preview, layout.output)
                        {
                            match self.thumbnail(
                                state,
                                renderer,
                                titlebars,
                                window,
                                dest,
                                &sizes,
                                shaders,
                                composer,
                                layout.output,
                                state.overview_progress() as f32,
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
                            preview,
                            state.overview_progress() as f32,
                        );
                        ring(
                            &mut elements,
                            preview,
                            2,
                            fade_color(
                                if id == session.workspace {
                                    state.runtime.config.theme.active_border
                                } else {
                                    [0.35, 0.38, 0.44, 0.8]
                                },
                                state.overview_progress() as f32,
                            ),
                            layout.output,
                        );
                    }
                    state
                        .runtime
                        .desktop
                        .workspace(id)
                        .map(|w| w.name.clone())
                        .unwrap_or_default()
                }
                OverviewTarget::Window(id) => {
                    if let Some(preview) = item.preview {
                        let start = elements.len();
                        match self.thumbnail(
                            state,
                            renderer,
                            titlebars,
                            id,
                            preview,
                            &sizes,
                            shaders,
                            composer,
                            layout.output,
                            state.overview_window_opacity(id),
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
                region.rect,
                1.0,
            );
        }
        Ok(elements)
    }
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
    dest: Rect,
    opacity: f32,
) {
    if dest.width <= 0 || dest.height <= 0 {
        return;
    }
    if let Some(element) = wallpapers.preview_element(
        renderer,
        &state.runtime.wallpapers,
        output,
        source,
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
    /// The same bounded miniature list drives rendering and client visibility.
    fn overview_miniatures(
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
                self.runtime
                    .desktop
                    .window(**id)
                    .is_some_and(|w| w.role == crate::core::WindowRole::Normal && !w.minimized)
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
        for item in &layout.items {
            if let Some(preview) = item.preview {
                match item.target {
                    OverviewTarget::Window(id) => {
                        windows.insert(id);
                    }
                    OverviewTarget::Workspace(id) => {
                        windows.extend(
                            self.overview_miniatures(id, preview, layout.output)
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

    #[test]
    fn fullscreen_root_commit_invalidates_preview_without_buffer_damage() {
        let ordinary = ThumbnailAppearance {
            image: WindowImageIdentity {
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
        };
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
