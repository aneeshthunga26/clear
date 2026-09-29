//! Bounded local CPU icon preparation; accessors never perform filesystem IO.

use std::{
    collections::BTreeMap,
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::Arc,
};

use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
use resvg::{tiny_skia, usvg};

use crate::decoration::TitlebarTheme;

/// Maximum uncompressed SVG source size (SVGZ and embedded images are unsupported).
pub const MAX_SVG_BYTES: u64 = 256 * 1024;
/// Maximum encoded PNG/JPEG app icon size.
pub const MAX_RASTER_BYTES: u64 = 2 * 1024 * 1024;
/// Positive and negative app-ID entries retained until the next reload.
pub const MAX_APP_ICONS: usize = 256;
const MAX_SOURCE_DIMENSION: u32 = 1024;
const MAX_DECODE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DESKTOP_BYTES: u64 = 64 * 1024;
const MAX_ROOTS: usize = 8;
const MAX_PATH_BYTES: usize = 4096;
const MAX_LOOKUP_PROBES: usize = 256;
const MAX_TOTAL_PROBES: usize = 4096;
const MAX_LOOKUP_BYTES: u64 = 16 * 1024 * 1024;

/// Premultiplied RGBA8 pixels in an aspect-preserving, transparent square icon box.
#[derive(Clone, Debug, PartialEq)]
pub struct TitlebarImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Prepared controls and a bounded positive/negative desktop-app icon cache.
#[derive(Debug)]
pub struct TitlebarAssets {
    controls: BTreeMap<&'static str, Arc<TitlebarImage>>,
    apps: BTreeMap<String, Option<Arc<TitlebarImage>>>,
    icon_size: u32,
    show_icon: bool,
    data_dirs: Vec<PathBuf>,
    legacy_icons: Option<PathBuf>,
    remaining_probes: usize,
    remaining_bytes: u64,
}

impl TitlebarAssets {
    /// Prepare explicit SVGs as one candidate set. Reload always rereads the files.
    pub fn prepare(theme: &TitlebarTheme) -> Result<Self, String> {
        theme.validate()?;
        let mut assets = Self::builtins(theme);
        let mut images = BTreeMap::new();
        for (name, path) in theme.controls.paths() {
            if let Some(path) = path {
                let image = if let Some(image) = images.get(path) {
                    Arc::clone(image)
                } else {
                    let image = read_regular(path, MAX_SVG_BYTES)
                        .and_then(|bytes| raster_svg(&bytes, assets.icon_size))
                        .map_err(|error| {
                            format!("titlebar control {name} {}: {error}", path.display())
                        })?;
                    let image = Arc::new(image);
                    images.insert(path.to_owned(), image.clone());
                    image
                };
                assets.controls.insert(name, image);
            }
        }
        Ok(assets)
    }

    /// Empty control overrides preserve built-in glyphs while retaining app-icon policy.
    pub(crate) fn builtins(theme: &TitlebarTheme) -> Self {
        let (data_dirs, legacy_icons) = icon_roots();
        Self {
            controls: BTreeMap::new(),
            apps: BTreeMap::new(),
            icon_size: theme.icon_size(),
            show_icon: theme.show_icon,
            data_dirs,
            legacy_icons,
            remaining_probes: MAX_TOTAL_PROBES,
            remaining_bytes: MAX_LOOKUP_BYTES,
        }
    }

    /// A custom control, or `None` for the built-in glyph (also for unknown names).
    pub fn control(&self, name: &str) -> Option<&Arc<TitlebarImage>> {
        self.controls.get(name)
    }

    /// A previously prepared app icon, or `None` for the generic renderer fallback.
    pub fn app_icon(&self, app_id: &str) -> Option<&Arc<TitlebarImage>> {
        self.apps.get(app_id).and_then(Option::as_ref)
    }

    /// Lookup once per app ID on classification/reload, never from the render loop.
    /// Missing/invalid icons are cached too; a full cache stops new lookups until reload.
    pub fn prepare_app_icon(&mut self, app_id: &str) {
        if !self.show_icon
            || !safe_name(app_id)
            || self.apps.contains_key(app_id)
            || self.apps.len() >= MAX_APP_ICONS
        {
            return;
        }
        let image = self.lookup_app_icon(app_id).map(Arc::new);
        self.apps.insert(app_id.to_owned(), image);
    }

    fn lookup_app_icon(&mut self, app_id: &str) -> Option<TitlebarImage> {
        let mut probes = MAX_LOOKUP_PROBES;
        let desktop_name = if app_id.ends_with(".desktop") {
            app_id.to_owned()
        } else {
            format!("{app_id}.desktop")
        };
        let mut icon = None;
        // Exact desktop-file IDs only: no recursive directory walk, fuzzy matching,
        // Exec/TryExec processing, or icon-theme inheritance graph traversal.
        for directory in self.data_dirs.clone() {
            let path = directory.join("applications").join(&desktop_name);
            if let Some(bytes) = self.lookup_read(&path, MAX_DESKTOP_BYTES, &mut probes) {
                // A higher-priority desktop entry shadows lower-priority entries even
                // when it has no usable Icon key or is Hidden=true.
                icon = desktop_icon(&bytes);
                break;
            }
        }
        let icon = icon?;
        if Path::new(&icon).is_absolute() {
            return self.lookup_image(Path::new(&icon), &mut probes);
        }
        if !safe_name(&icon) {
            return None;
        }
        let names = if Path::new(&icon)
            .extension()
            .is_some_and(|ext| matches!(ext.to_str(), Some("svg" | "png" | "jpg" | "jpeg")))
        {
            vec![icon]
        } else {
            vec![
                format!("{icon}.svg"),
                format!("{icon}.png"),
                format!("{icon}.jpg"),
            ]
        };
        let mut roots = Vec::new();
        if let Some(directory) = &self.legacy_icons {
            roots.push((directory.clone(), false));
        }
        roots.extend(
            self.data_dirs
                .iter()
                .map(|directory| (directory.clone(), true)),
        );
        for (directory, data_root) in roots {
            let icons = if data_root {
                directory.join("icons")
            } else {
                directory.clone()
            };
            for size in [
                "scalable", "16x16", "24x24", "32x32", "48x48", "64x64", "128x128", "256x256",
            ] {
                for name in &names {
                    if let Some(image) = self.lookup_image(
                        &icons.join("hicolor").join(size).join("apps").join(name),
                        &mut probes,
                    ) {
                        return Some(image);
                    }
                }
            }
            for name in &names {
                if let Some(image) = self.lookup_image(&icons.join(name), &mut probes) {
                    return Some(image);
                }
                if data_root {
                    if let Some(image) =
                        self.lookup_image(&directory.join("pixmaps").join(name), &mut probes)
                    {
                        return Some(image);
                    }
                }
            }
        }
        None
    }

    fn lookup_read(&mut self, path: &Path, limit: u64, probes: &mut usize) -> Option<Vec<u8>> {
        if *probes == 0
            || self.remaining_probes == 0
            || self.remaining_bytes == 0
            || path.as_os_str().len() > MAX_PATH_BYTES
        {
            return None;
        }
        *probes -= 1;
        self.remaining_probes -= 1;
        let limit = limit.min(self.remaining_bytes);
        // Charge the full read allowance even for malformed/unreadable existing files.
        // Missing candidates consume only probes, so ordinary icon search stays useful.
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_file() || metadata.len() > limit {
            return None;
        }
        self.remaining_bytes -= limit;
        let bytes = read_regular(path, limit).ok()?;
        self.remaining_bytes += limit - bytes.len() as u64;
        Some(bytes)
    }

    fn lookup_image(&mut self, path: &Path, probes: &mut usize) -> Option<TitlebarImage> {
        let svg = match path.extension()?.to_str()? {
            "svg" => true,
            "png" | "jpg" | "jpeg" => false,
            _ => return None,
        };
        let bytes = self.lookup_read(
            path,
            if svg { MAX_SVG_BYTES } else { MAX_RASTER_BYTES },
            probes,
        )?;
        if svg {
            raster_svg(&bytes, self.icon_size).ok()
        } else {
            raster_image(&bytes, self.icon_size).ok()
        }
    }
}

fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(format!("expected a regular file of at most {limit} bytes"));
    }
    let file = File::open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(format!("expected a regular file of at most {limit} bytes"));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > limit {
        return Err(format!("file exceeds {limit} bytes"));
    }
    Ok(bytes)
}

fn raster_svg(bytes: &[u8], size: u32) -> Result<TitlebarImage, String> {
    let source = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
    // Parse without DTD/entity expansion, then pass this checked tree directly to usvg.
    let doc = usvg::roxmltree::Document::parse_with_options(
        source,
        usvg::roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 4096,
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())?;
    for node in doc.descendants().filter(|node| node.is_element()) {
        if node.ancestors().take(34).count() > 33 {
            return Err("SVG nesting exceeds 32 levels".into());
        }
        // Shallow sibling definitions can expand exponentially through references,
        // regardless of XML depth or raster size. Reject definitions by local name
        // before usvg conversion, including unused/namespaced ones, so CSS and href
        // references cannot bypass this bounded path/gradient subset.
        let name = node.tag_name().name();
        if matches!(
            name,
            "use" | "pattern" | "marker" | "mask" | "clipPath" | "filter"
        ) {
            return Err(format!("SVG {name} is unsupported for titlebar icons"));
        }
    }
    let options = usvg::Options {
        resources_dir: None,
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    let tree = usvg::Tree::from_xmltree(&doc, &options).map_err(|error| error.to_string())?;
    let dimensions = tree.size();
    if dimensions.width() > MAX_SOURCE_DIMENSION as f32
        || dimensions.height() > MAX_SOURCE_DIMENSION as f32
    {
        return Err(format!("SVG dimensions exceed {MAX_SOURCE_DIMENSION}"));
    }
    let scale = (size as f32 / dimensions.width()).min(size as f32 / dimensions.height());
    if !scale.is_finite() {
        return Err("SVG dimensions are too small to rasterize safely".into());
    }
    let x = (size as f32 - dimensions.width() * scale) / 2.0;
    let y = (size as f32 - dimensions.height() * scale) / 2.0;
    let mut pixmap = tiny_skia::Pixmap::new(size, size).ok_or("invalid icon raster size")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, x, y),
        &mut pixmap.as_mut(),
    );
    Ok(TitlebarImage {
        width: size,
        height: size,
        pixels: pixmap.take(),
    })
}

fn raster_image(bytes: &[u8], size: u32) -> Result<TitlebarImage, String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| error.to_string())?;
    if !matches!(reader.format(), Some(ImageFormat::Png | ImageFormat::Jpeg)) {
        return Err("only SVG, PNG and JPEG app icons are supported".into());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_DIMENSION);
    limits.max_image_height = Some(MAX_SOURCE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits);
    let decoder = reader.into_decoder().map_err(|error| error.to_string())?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || width > MAX_SOURCE_DIMENSION
        || height > MAX_SOURCE_DIMENSION
        || decoder.total_bytes() > MAX_DECODE_BYTES
    {
        return Err("app icon exceeds decoded image limits".into());
    }
    let mut rgba = image::DynamicImage::from_decoder(decoder)
        .map_err(|error| error.to_string())?
        .into_rgba8();
    // Premultiply before filtering to avoid colored fringes around transparent pixels.
    for pixel in rgba.pixels_mut() {
        for channel in 0..3 {
            pixel[channel] = ((u16::from(pixel[channel]) * u16::from(pixel[3]) + 127) / 255) as u8;
        }
    }
    let scale = (size as f64 / width as f64).min(size as f64 / height as f64);
    let width = (width as f64 * scale).round().clamp(1.0, size as f64) as u32;
    let height = (height as f64 * scale).round().clamp(1.0, size as f64) as u32;
    let resized =
        image::imageops::resize(&rgba, width, height, image::imageops::FilterType::Triangle);
    let mut pixels = image::RgbaImage::new(size, size);
    image::imageops::replace(
        &mut pixels,
        &resized,
        i64::from((size - width) / 2),
        i64::from((size - height) / 2),
    );
    Ok(TitlebarImage {
        width: size,
        height: size,
        pixels: pixels.into_raw(),
    })
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn desktop_icon(bytes: &[u8]) -> Option<String> {
    let source = std::str::from_utf8(bytes).ok()?;
    let mut desktop_group = false;
    let mut icon = None;
    for line in source.lines().map(str::trim) {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            desktop_group = line == "[Desktop Entry]";
        } else if desktop_group {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            if key.trim() == "Hidden" && value == "true" {
                return None;
            }
            if key.trim() == "Icon"
                && !value.is_empty()
                && value.len() <= MAX_PATH_BYTES
                && !value.contains('\0')
            {
                icon = Some(value.to_owned());
            }
        }
    }
    icon
}

fn icon_roots() -> (Vec<PathBuf>, Option<PathBuf>) {
    let absolute_env = |key| {
        std::env::var_os(key)
            .filter(|value| value.len() <= MAX_PATH_BYTES)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let home = absolute_env("HOME");
    let data_home = absolute_env("XDG_DATA_HOME")
        .or_else(|| home.as_ref().map(|path| path.join(".local/share")));
    let mut roots: Vec<_> = data_home.into_iter().collect();
    let dirs = std::env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty() && value.len() <= MAX_PATH_BYTES * MAX_ROOTS)
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    for path in std::env::split_paths(&dirs).take(MAX_ROOTS) {
        if roots.len() == MAX_ROOTS {
            break;
        }
        if path.is_absolute() && path.as_os_str().len() <= MAX_PATH_BYTES && !roots.contains(&path)
        {
            roots.push(path);
        }
    }
    (roots, home.map(|path| path.join(".icons")))
}
