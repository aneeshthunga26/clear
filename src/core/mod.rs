//! Platform-independent desktop state and commands. Coordinates are logical pixels.

mod desktop;
mod fullscreen;
mod geometry;
mod resize;
mod snapping;
mod types;

pub use desktop::Desktop;
pub use fullscreen::*;
pub use geometry::Rect;
pub use resize::{LayoutSizing, ResizeEdges, ResizeSession};
pub use snapping::*;
pub use types::*;
