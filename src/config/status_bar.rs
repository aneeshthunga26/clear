use serde::Deserialize;
use smithay::wayland::shell::wlr_layer::{Anchor, Layer};

/// Config for the layer-shell status bar.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct StatusBarConfig {
    /// Shell command used to launch the status bar.
    pub command: String,
    /// Layer-shell namespace used to recognize the bar surface.
    pub namespace: String,
    /// Preferred edge for the status bar.
    pub position: StatusBarPosition,
    /// Layer-shell layer used by the bar.
    pub layer: StatusBarLayer,
    /// Whether the bar should reserve normal window space.
    pub exclusive: bool,
}

impl StatusBarConfig {
    /// Command that should be spawned for the status bar.
    pub fn command(&self) -> &str {
        self.command.trim()
    }

    /// Namespace expected from the status bar layer surface.
    pub fn namespace(&self) -> &str {
        self.namespace.trim()
    }
}

impl Default for StatusBarConfig {
    fn default() -> Self {
        Self {
            command: "waybar".to_string(),
            namespace: "waybar".to_string(),
            position: StatusBarPosition::Top,
            layer: StatusBarLayer::Top,
            exclusive: true,
        }
    }
}

/// Screen edge where the status bar should appear.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StatusBarPosition {
    Top,
    Bottom,
    Left,
    Right,
}

impl StatusBarPosition {
    /// Layer-shell anchors corresponding to the configured bar edge.
    pub fn anchors(self) -> Anchor {
        match self {
            Self::Top => Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
            Self::Bottom => Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            Self::Left => Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM,
            Self::Right => Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM,
        }
    }
}

impl Default for StatusBarPosition {
    fn default() -> Self {
        Self::Top
    }
}

/// Layer-shell layer for the status bar surface.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusBarLayer {
    Background,
    Bottom,
    Top,
    Overlay,
}

impl From<StatusBarLayer> for Layer {
    fn from(layer: StatusBarLayer) -> Self {
        match layer {
            StatusBarLayer::Background => Self::Background,
            StatusBarLayer::Bottom => Self::Bottom,
            StatusBarLayer::Top => Self::Top,
            StatusBarLayer::Overlay => Self::Overlay,
        }
    }
}

impl Default for StatusBarLayer {
    fn default() -> Self {
        Self::Top
    }
}
