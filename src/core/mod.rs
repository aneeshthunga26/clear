//! Platform-independent desktop state and commands. Coordinates are logical pixels.

mod desktop;
mod geometry;
mod types;

pub use desktop::Desktop;
pub use geometry::Rect;
pub use types::*;
