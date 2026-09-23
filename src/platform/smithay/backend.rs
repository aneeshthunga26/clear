use super::{
    scene::premultiply,
    state::{Compositor, OutputRegion},
};
use crate::core::{OutputId, Rect};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{ExportMem, Frame, Renderer, damage::OutputDamageTracker, gles::GlesRenderer},
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::EventLoop,
        winit::{dpi::LogicalSize, window::WindowAttributes},
    },
    utils::{Physical, Rectangle, Size, Transform},
};

pub(super) fn init(
    event_loop: &EventLoop<Compositor>,
    state: &mut Compositor,
    mut capture: Option<std::path::PathBuf>,
    capture_after: std::time::Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let width: i32 = state.runtime.config.outputs.iter().map(|o| o.width).sum();
    let height = state
        .runtime
        .config
        .outputs
        .iter()
        .map(|o| o.height)
        .max()
        .unwrap_or(600);
    let attributes = WindowAttributes::default()
        .with_title("Clear — nested compositor")
        .with_surface_size(LogicalSize::new(f64::from(width), f64::from(height)))
        .with_visible(true);
    let (mut backend, events) = winit::init_from_attributes::<GlesRenderer>(attributes)?;
    for (index, config) in state.runtime.config.outputs.iter().enumerate() {
        let output = Output::new(
            config.name.clone(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "Clear".into(),
                model: "Virtual output".into(),
                serial_number: config.name.clone(),
            },
        );
        output.create_global::<Compositor>(&state.display_handle);
        state.outputs.push(OutputRegion {
            id: OutputId(index as u64 + 1),
            output,
            rect: Rect::new(0, 0, 1, 1),
        });
    }
    resize(state, backend.window_size());
    state.runtime.configure_output_modes();
    let mut damage = OutputDamageTracker::new(backend.window_size(), 1.0, Transform::Flipped180);
    event_loop
        .handle()
        .insert_source(events, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } if size.w > 0 && size.h > 0 => {
                resize(state, size);
                damage = OutputDamageTracker::new(size, 1.0, Transform::Flipped180);
            }
            WinitEvent::Input(event) => state.process_input(event),
            WinitEvent::Focus(focused) => {
                state.host_focused = focused;
                if !focused {
                    state.end_drag();
                }
                state.dirty = true;
            }
            WinitEvent::Redraw => {
                let size = backend.window_size();
                if size.w <= 0 || size.h <= 0 {
                    return;
                }
                state.reconcile();
                let result = (|| -> Result<(), String> {
                    let (renderer, mut framebuffer) = backend
                        .bind()
                        .map_err(|e| format!("bind framebuffer: {e}"))?;
                    let elements = state.scene_elements(renderer);
                    damage
                        .render_output(
                            renderer,
                            &mut framebuffer,
                            0,
                            &elements,
                            premultiply(state.runtime.config.theme.background),
                        )
                        .map_err(|e| format!("render frame: {e}"))?;
                    if state.start.elapsed() >= capture_after {
                        if let Some(path) = capture.take() {
                            use std::io::Write;
                            let mapping = renderer
                                .copy_framebuffer(
                                    &framebuffer,
                                    Rectangle::from_size((size.w, size.h).into()),
                                    Fourcc::Abgr8888,
                                )
                                .map_err(|e| format!("capture framebuffer: {e}"))?;
                            let pixels = renderer
                                .map_texture(&mapping)
                                .map_err(|e| format!("map capture: {e}"))?;
                            let mut file = std::io::BufWriter::new(
                                std::fs::File::create(&path)
                                    .map_err(|e| format!("create capture: {e}"))?,
                            );
                            write!(file, "P6\n{} {}\n255\n", size.w, size.h)
                                .map_err(|e| e.to_string())?;
                            // EGL readback starts at the bottom row; PPM starts at the top.
                            for row in pixels.chunks_exact(size.w as usize * 4).rev() {
                                for pixel in row.chunks_exact(4) {
                                    file.write_all(&pixel[..3]).map_err(|e| e.to_string())?;
                                }
                            }
                            file.flush().map_err(|e| e.to_string())?;
                            eprintln!("clear: captured {}", path.display());
                            // map_texture makes the context surfaceless. Restore the window
                            // target before EGL swaps it; an empty frame preserves its pixels.
                            renderer
                                .render(&mut framebuffer, size, Transform::Flipped180)
                                .map_err(|e| format!("restore capture target: {e}"))?
                                .finish()
                                .map_err(|e| format!("finish capture target: {e}"))?
                                .wait()
                                .map_err(|e| format!("wait for capture target: {e}"))?;
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    state.fail(error);
                    return;
                }
                if let Err(error) = backend.submit(Some(&[Rectangle::from_size(size)])) {
                    state.fail(format!("submit frame: {error}"));
                    return;
                }
                state.frame_callbacks();
                backend.window().request_redraw();
            }
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            _ => {}
        })?;
    Ok(())
}

fn resize(state: &mut Compositor, size: Size<i32, Physical>) {
    state.host_size = (size.w, size.h).into();
    let total: i64 = state
        .runtime
        .config
        .outputs
        .iter()
        .map(|o| i64::from(o.width))
        .sum();
    let max_height = state
        .runtime
        .config
        .outputs
        .iter()
        .map(|o| o.height)
        .max()
        .unwrap_or(1);
    let mut weight = 0_i64;
    for (region, config) in state.outputs.iter_mut().zip(&state.runtime.config.outputs) {
        let x = (weight * i64::from(size.w) / total) as i32;
        weight += i64::from(config.width);
        let right = (weight * i64::from(size.w) / total) as i32;
        let height = (i64::from(size.h) * i64::from(config.height) / i64::from(max_height)) as i32;
        let rect = Rect::new(x, 0, (right - x).max(1), height.max(1));
        let mode = Mode {
            size: (rect.width, rect.height).into(),
            refresh: 60_000,
        };
        region.output.change_current_state(
            Some(mode),
            Some(Transform::Normal),
            None,
            Some((rect.x, rect.y).into()),
        );
        region.output.set_preferred(mode);
        region.rect = rect;
        state.space.map_output(&region.output, (rect.x, rect.y));
        state
            .runtime
            .desktop
            .add_output(region.id, config.name.clone(), rect);
    }
    state.dirty = true;
}
