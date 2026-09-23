//! Declarative shell roles; protocol types are translated only by the adapter.

use serde::Deserialize;
use std::collections::BTreeSet;

/// Paint/input stratum requested by a named panel rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PanelLayer {
    Background,
    Bottom,
    Top,
    Overlay,
}

/// Override a layer-shell client's paint layer without changing its anchors or zone.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelRule {
    /// Exact layer-shell namespace, for example `waybar`.
    pub namespace: String,
    /// Compositor policy takes precedence over the client's requested layer.
    pub layer: PanelLayer,
}

/// Shell roles are metadata rules, not ownership of external application processes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShellConfig {
    /// XDG app IDs centered above normal windows, using their committed size.
    pub launcher_app_ids: Vec<String>,
    /// Namespace-specific panel layer overrides; exclusive zones remain client-owned.
    pub panels: Vec<PanelRule>,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            launcher_app_ids: vec!["wofi".into()],
            panels: vec![PanelRule {
                namespace: "waybar".into(),
                layer: PanelLayer::Top,
            }],
        }
    }
}

impl ShellConfig {
    /// Whether an exact XDG app ID should receive launcher placement policy.
    pub fn is_launcher(&self, app_id: &str) -> bool {
        !app_id.is_empty() && self.launcher_app_ids.iter().any(|id| id == app_id)
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        let mut names = BTreeSet::new();
        for name in &self.launcher_app_ids {
            if name.trim().is_empty() || !names.insert(name) {
                return Err("shell.launcher_app_ids requires unique, nonempty IDs".into());
            }
        }
        let mut names = BTreeSet::new();
        for panel in &self.panels {
            if panel.namespace.trim().is_empty() || !names.insert(&panel.namespace) {
                return Err("shell.panels requires unique, nonempty namespaces".into());
            }
        }
        Ok(())
    }
}
