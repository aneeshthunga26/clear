//! Bounded, panel-owned committed window captures for optional shell clients.

use super::{
    rounded::RoundedShaders,
    state::Compositor,
    titlebar::TitlebarCache,
    window_image::{PreparedWindowImage, WindowImageComposer},
};
use crate::{
    core::{WindowId, WindowRole},
    shell::{
        self, WindowPreview,
        server::{ClientId, Server},
    },
};
use image::ImageEncoder;
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{ExportMem, TextureMapping, gles::GlesRenderer},
    },
    utils::Rectangle,
};
use std::time::Duration;

const INTERVAL: Duration = Duration::from_millis(200);
const LEASE: Duration = Duration::from_millis(450);
const MAX_WINDOWS: usize = 4;

/// One pending batch and one short callback lease; owns no persistent GPU images.
#[derive(Default)]
pub(super) struct ShellPreviews {
    pending: Option<(ClientId, u32, Vec<WindowId>)>,
    last: Option<Duration>,
    live: Option<(ClientId, Vec<WindowId>, Duration)>,
}

impl Compositor {
    /// Validate panel ownership and the entire batch before accepting bounded capture work.
    pub fn queue_shell_previews(
        &mut self,
        server: &Server,
        client: ClientId,
        request: u32,
        windows: &[String],
    ) -> Result<(), String> {
        if !server
            .peer_credentials(client)
            .is_some_and(|peer| self.owns_shell_panel(peer))
        {
            return Err("window previews require a mapped panel owned by this process".into());
        }
        if windows.is_empty() || windows.len() > MAX_WINDOWS {
            return Err("window previews require one to four distinct normal windows".into());
        }
        let mut ids = Vec::new();
        for target in windows {
            let id = self
                .runtime
                .desktop
                .windows()
                .find(|w| w.id.0.to_string() == *target && w.role == WindowRole::Normal)
                .map(|w| w.id)
                .ok_or("unknown or ineligible preview window")?;
            if ids.contains(&id) {
                return Err("duplicate preview window".into());
            }
            ids.push(id);
        }
        let now = self.start.elapsed();
        if self.shell_previews.pending.is_some()
            || self
                .shell_previews
                .last
                .is_some_and(|last| now.saturating_sub(last) < INTERVAL)
        {
            return Err("window preview rate limit; retry after 200ms".into());
        }
        self.shell_previews.last = Some(now);
        self.shell_previews.live = Some((client, ids.clone(), now + LEASE));
        self.shell_previews.pending = Some((client, request, ids));
        self.dirty = true;
        self.render_demand
            .invalidate(super::render_demand::Reason::Policy);
        Ok(())
    }

    /// Return the active callback lease deadline for the common scheduler.
    pub fn shell_preview_expiry(&self) -> Option<Duration> {
        self.shell_previews
            .live
            .as_ref()
            .map(|(_, _, until)| *until)
    }

    /// Drop callback eligibility after expiry, disconnect or panel ownership loss.
    pub fn expire_shell_previews(&mut self) {
        if self
            .shell_previews
            .live
            .as_ref()
            .is_some_and(|(client, _, until)| {
                self.start.elapsed() >= *until
                    || !self.shell_server.as_ref().is_some_and(|s| {
                        s.is_connected(*client)
                            && s.peer_credentials(*client)
                                .is_some_and(|p| self.owns_shell_panel(p))
                    })
            })
        {
            self.shell_previews.live = None;
            self.dirty = true;
        }
    }

    /// Return leased window IDs without changing desktop visibility or focus.
    pub fn shell_preview_windows(&self) -> Vec<WindowId> {
        self.shell_previews
            .live
            .as_ref()
            .filter(|(_, _, until)| self.start.elapsed() < *until)
            .map(|(_, ids, _)| ids.clone())
            .unwrap_or_default()
    }

    /// Report whether the next host draw must service one capture batch.
    pub fn shell_previews_pending(&self) -> bool {
        self.shell_previews.pending.is_some()
    }

    /// Reply with committed images or per-window unavailability; never fail the desktop.
    pub fn prepare_shell_previews(
        &mut self,
        renderer: &mut GlesRenderer,
        titlebars: &mut TitlebarCache,
        shaders: &RoundedShaders,
        composer: &mut WindowImageComposer,
    ) {
        let Some((client, request, ids)) = self.shell_previews.pending.take() else {
            return;
        };
        let authorized = self
            .shell_server
            .as_ref()
            .and_then(|s| s.peer_credentials(client))
            .is_some_and(|peer| self.owns_shell_panel(peer));
        if !authorized {
            if let Some(server) = self.shell_server.as_mut() {
                server.reply(
                    client,
                    shell::encode_error(Some(request), "preview panel is no longer mapped"),
                );
            }
            return;
        }
        let frames: Vec<_> = ids
            .into_iter()
            .map(|id| WindowPreview {
                window: id.0.to_string(),
                image: self
                    .capture_shell_preview(renderer, titlebars, shaders, composer, id)
                    .ok()
                    .flatten(),
            })
            .collect();
        if let Some(server) = self.shell_server.as_mut() {
            server.reply(client, shell::encode_window_previews(request, &frames));
        }
    }

    fn capture_shell_preview(
        &self,
        renderer: &mut GlesRenderer,
        titlebars: &mut TitlebarCache,
        shaders: &RoundedShaders,
        composer: &mut WindowImageComposer,
        id: WindowId,
    ) -> Result<Option<String>, String> {
        if self
            .runtime
            .desktop
            .window(id)
            .is_none_or(|w| w.role != WindowRole::Normal)
        {
            return Ok(None);
        }
        let Some(prepared) = PreparedWindowImage::prepare(
            self,
            renderer,
            titlebars,
            id,
            self.runtime.desktop.focused_window() == Some(id),
        )
        .map_err(|e| e.to_string())?
        else {
            return Ok(None);
        };
        let source = prepared.identity.source;
        let scale = (256.0 / f64::from(source.width.max(1)))
            .min(144.0 / f64::from(source.height.max(1)))
            .min(1.0);
        let size = (
            (f64::from(source.width) * scale).round().max(1.0) as i32,
            (f64::from(source.height) * scale).round().max(1.0) as i32,
        );
        let image = composer
            .compose(renderer, shaders, &prepared, size, None)
            .map_err(|e| e.to_string())?;
        let mapping = renderer
            .copy_texture(
                &image.texture,
                Rectangle::from_size(size.into()),
                Fourcc::Abgr8888,
            )
            .map_err(|e| e.to_string())?;
        let flipped = mapping.flipped();
        let bytes = renderer.map_texture(&mapping).map_err(|e| e.to_string())?;
        let stride = size.0 as usize * 4;
        let mut rgba = vec![0; stride * size.1 as usize];
        for y in 0..size.1 as usize {
            // Normal offscreen composition is top-first in the GL readback.
            let src_y = if flipped { y } else { size.1 as usize - 1 - y };
            for (src, dst) in bytes[src_y * stride..(src_y + 1) * stride]
                .chunks_exact(4)
                .zip(rgba[y * stride..(y + 1) * stride].chunks_exact_mut(4))
            {
                dst.copy_from_slice(src);
                // PNG uses straight alpha; the shared GPU image is premultiplied.
                for channel in &mut dst[..3] {
                    *channel = if src[3] == 0 {
                        0
                    } else {
                        (u32::from(*channel) * 255 / u32::from(src[3])).min(255) as u8
                    };
                }
            }
        }
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(
                &rgba,
                size.0 as u32,
                size.1 as u32,
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
        Ok(Some(format!("data:image/png;base64,{}", base64(&png))))
    }
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        output.push(ALPHABET[((bits >> 18) & 63) as usize] as char);
        output.push(ALPHABET[((bits >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 {
            ALPHABET[((bits >> 6) & 63) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            ALPHABET[(bits & 63) as usize] as char
        } else {
            '='
        });
    }
    output
}

#[cfg(test)]
mod tests {
    #[test]
    fn png_transport_base64_padding() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(super::base64(input.as_bytes()), expected);
        }
    }
}
