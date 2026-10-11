//! Optional, constant-space submitted-frame diagnostics; never controls pacing.

use super::{scene::SceneElement, titlebar::TitlebarCache};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem, Renderer,
            element::{Id, Kind, texture::TextureRenderElement},
            gles::{GlesError, GlesRenderer, GlesTexture},
        },
    },
    utils::{Physical, Size, Transform},
};
use std::time::Duration;

const WIDTH: i32 = 240;
const HEIGHT: i32 = 32;
const UPDATE_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Default)]
struct FrameRate {
    start: Option<Duration>,
    intervals: u64,
    reading: Option<(f64, f64)>,
}

impl FrameRate {
    fn submitted(&mut self, now: Duration) {
        let Some(start) = self.start else {
            self.start = Some(now);
            return;
        };
        self.intervals += 1;
        let elapsed = now.saturating_sub(start);
        if elapsed >= UPDATE_INTERVAL {
            let seconds = elapsed.as_secs_f64();
            self.reading = Some((
                self.intervals as f64 / seconds,
                seconds * 1000.0 / self.intervals as f64,
            ));
            self.start = Some(now);
            self.intervals = 0;
        }
    }

    fn label(&self) -> String {
        match self.reading {
            Some((fps, ms)) => format!("FPS: {fps:.1} | {ms:.1} ms"),
            None => "FPS: -- | -- ms".into(),
        }
    }
}

/// A single small texture, reused until the twice-per-second reading changes.
#[derive(Default)]
pub(super) struct FpsOverlay {
    enabled: bool,
    rate: FrameRate,
    cached: Option<(Option<(f64, f64)>, GlesTexture, Id)>,
}

impl FpsOverlay {
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled != self.enabled {
            *self = Self {
                enabled,
                ..Self::default()
            };
        }
    }

    /// Count only successful submissions, including time spent rendering and swapping.
    pub fn submitted(&mut self, now: Duration) {
        if self.enabled {
            self.rate.submitted(now);
        }
    }

    pub fn element(
        &mut self,
        renderer: &mut GlesRenderer,
        titlebars: &mut TitlebarCache,
        size: Size<i32, Physical>,
    ) -> Result<Option<SceneElement>, GlesError> {
        if !self.enabled {
            return Ok(None);
        }
        if self
            .cached
            .as_ref()
            .is_none_or(|(old, _, _)| old != &self.rate.reading)
        {
            let label = self.rate.label();
            let mut pixels = titlebars.overview_label(&label, WIDTH, None, [245, 248, 255, 255]);
            for pixel in pixels.chunks_exact_mut(4) {
                // Text is premultiplied; put a dark backing behind it once on upload.
                let remaining = 255 - u32::from(pixel[3]);
                for (channel, background) in pixel.iter_mut().zip([8, 10, 14, 230]) {
                    *channel = (u32::from(*channel) + background * remaining / 255) as u8;
                }
            }
            let texture =
                renderer.import_memory(&pixels, Fourcc::Abgr8888, (WIDTH, HEIGHT).into(), false)?;
            self.cached = Some((self.rate.reading, texture, Id::new()));
        }
        Ok(Some(SceneElement::Wallpaper(
            TextureRenderElement::from_static_texture(
                self.cached
                    .as_ref()
                    .expect("FPS texture prepared")
                    .2
                    .clone(),
                renderer.context_id(),
                (
                    f64::from((size.w - WIDTH - 8).max(0)),
                    f64::from(8.min((size.h - HEIGHT).max(0))),
                ),
                self.cached
                    .as_ref()
                    .expect("FPS texture prepared")
                    .1
                    .clone(),
                1,
                Transform::Normal,
                Some(1.0),
                None,
                None,
                None,
                Kind::Unspecified,
            ),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_rate_includes_stalls_and_does_not_quantize_to_refresh_divisors() {
        let mut rate = FrameRate::default();
        for frame in 0..=59 {
            rate.submitted(Duration::from_secs_f64(frame as f64 / 59.0));
        }
        let (fps, ms) = rate.reading.unwrap();
        assert!((fps - 59.0).abs() < 0.001);
        assert!((ms - 1000.0 / 59.0).abs() < 0.001);
        rate.submitted(Duration::from_secs(3));
        assert!(rate.reading.unwrap().0 < 15.0);
    }

    #[test]
    fn disabling_and_reenabling_discards_the_previous_measurement_window() {
        let mut overlay = FpsOverlay::default();
        overlay.set_enabled(true);
        overlay.submitted(Duration::ZERO);
        overlay.submitted(Duration::from_secs(1));
        assert!(overlay.rate.reading.is_some());
        overlay.set_enabled(false);
        overlay.submitted(Duration::from_secs(20));
        overlay.set_enabled(true);
        overlay.submitted(Duration::from_secs(21));
        assert!(overlay.rate.reading.is_none());
        assert_eq!(overlay.rate.start, Some(Duration::from_secs(21)));
    }
}
