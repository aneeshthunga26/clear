//! Backend-independent wallpaper selection; resource IO belongs to runtime.

use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Image placement within each full output rectangle.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallpaperMode {
    /// Preserve aspect ratio and crop to cover the output.
    #[default]
    Fill,
    /// Preserve aspect ratio and show the entire image over the theme background.
    Fit,
    /// Resize independently along each axis to cover the output.
    Stretch,
    /// Center at native pixel size, clipping any excess.
    Center,
}

/// Global image and optional connector-name overrides, independent of output topology.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WallpaperConfig {
    /// PNG/JPEG path. Omission uses the solid theme background.
    pub path: Option<PathBuf>,
    /// Global placement policy; defaults to fill.
    pub mode: WallpaperMode,
    /// Overrides inherit any omitted path/mode from the global selection.
    pub outputs: BTreeMap<String, WallpaperOverride>,
}

/// Partial image selection for one output.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WallpaperOverride {
    /// Optional replacement for the global image path.
    pub path: Option<PathBuf>,
    /// Optional replacement for the global placement policy.
    pub mode: Option<WallpaperMode>,
}

impl WallpaperConfig {
    /// Resolve one output's selection, inheriting omitted override values.
    pub fn for_output(&self, name: &str) -> (Option<&Path>, WallpaperMode) {
        let entry = self.outputs.get(name);
        (
            entry
                .and_then(|entry| entry.path.as_deref())
                .or(self.path.as_deref()),
            entry.and_then(|entry| entry.mode).unwrap_or(self.mode),
        )
    }

    pub(super) fn validate(&self, outputs: &BTreeSet<&str>) -> Result<(), String> {
        for name in self.outputs.keys() {
            if !outputs.contains(name.as_str()) {
                return Err(format!("wallpaper refers to unknown output {name:?}"));
            }
        }
        for path in self.path.iter().chain(
            self.outputs
                .values()
                .filter_map(|entry| entry.path.as_ref()),
        ) {
            if path.as_os_str().is_empty()
                || path.to_string_lossy().trim().is_empty()
                || path.as_os_str().as_encoded_bytes().contains(&0)
            {
                return Err("wallpaper paths must be nonempty and contain no NUL bytes".into());
            }
        }
        Ok(())
    }

    pub(super) fn resolve_paths(&mut self, directory: &Path) {
        for path in self.path.iter_mut().chain(
            self.outputs
                .values_mut()
                .filter_map(|entry| entry.path.as_mut()),
        ) {
            if path.is_relative() {
                *path = directory.join(&*path);
            }
        }
    }
}
