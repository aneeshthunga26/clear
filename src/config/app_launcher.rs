use serde::Deserialize;

/// Config for the application launcher opened by the launcher shortcut.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct AppLauncherConfig {
    /// Shell command used to launch the app launcher.
    pub command: String,
    /// XDG app id used to recognize and center the launcher window.
    pub app_id: String,
}

impl AppLauncherConfig {
    /// Command that should be spawned for the launcher.
    pub fn command(&self) -> &str {
        self.command.trim()
    }

    /// App id expected from the launcher's XDG toplevel surface.
    pub fn app_id(&self) -> &str {
        self.app_id.trim()
    }
}

impl Default for AppLauncherConfig {
    fn default() -> Self {
        Self {
            command: "wofi --show drun --normal-window".to_string(),
            app_id: "wofi".to_string(),
        }
    }
}
