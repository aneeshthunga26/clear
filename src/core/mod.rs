//! Platform-independent desktop state and commands. Coordinates are logical pixels.

mod desktop;
mod geometry;
mod resize;
mod types;

pub use desktop::Desktop;
pub use geometry::Rect;
pub use resize::{LayoutSizing, ResizeEdges, ResizeSession};
pub use types::*;
