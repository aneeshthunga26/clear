//! Strict, bounded selection rules; renderer capability does not change requested policy.

use std::collections::BTreeSet;

use serde::Deserialize;

use crate::core::FullscreenMode;

/// Fullscreen entry defaults and exact, case-sensitive application rules.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FullscreenConfig {
    /// Fallback for an entry without an explicit mode or exact rule.
    pub default_mode: FullscreenMode,
    /// At most 256 exact app-ID rules.
    pub rules: Vec<FullscreenRule>,
}

/// One exact app-ID match. No wildcard or content-type inference is performed.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullscreenRule {
    /// Exact, case-sensitive Wayland app ID.
    pub app_id: String,
    /// Mode selected on future entries.
    pub mode: FullscreenMode,
}

impl FullscreenConfig {
    /// Select policy for a future entry, without modifying an existing slot.
    pub fn mode_for(&self, app_id: &str) -> FullscreenMode {
        self.rules
            .iter()
            .find(|r| r.app_id == app_id)
            .map_or(self.default_mode, |r| r.mode)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        if self.rules.len() > 256 {
            return Err("fullscreen supports at most 256 rules".into());
        }
        let mut ids = BTreeSet::new();
        for rule in &self.rules {
            if rule.app_id.trim().is_empty()
                || rule.app_id.len() > 255
                || rule.app_id.contains('\0')
                || !ids.insert(&rule.app_id)
            {
                return Err(
                    "fullscreen rules need unique, nonblank, NUL-free app IDs of at most 255 bytes"
                        .into(),
                );
            }
        }
        Ok(())
    }
}
