use super::{
    blur::BackdropBlur,
    frame_scheduler::{FrameScheduler, HostTiming},
    overview::OverviewCache,
    rounded::RoundedShaders,
    scene::SceneElement,
    scene::premultiply,
    state::{Compositor, OutputRegion},
    titlebar::TitlebarCache,
    wallpaper::WallpaperCache,
};
use crate::core::{OutputId, Rect};
use smithay::{
    backend::{
        allocator::Fourcc,
        egl::{context::GlAttributes, ffi::egl},
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
    let (mut backend, events) = winit::init_from_attributes_with_gl_attr::<GlesRenderer>(
        attributes,
        GlAttributes {
            version: (3, 0),
            profile: None,
            debug: cfg!(debug_assertions),
            vsync: true,
        },
    )?;
    let mut host_timing = host_timing(backend.window());
    report_timing(host_timing);
    let mut scheduler = FrameScheduler::new(
        host_timing,
        state.runtime.config.animations.frame_rate,
        state.start.elapsed(),
    );
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
    resize(state, backend.window_size(), host_timing.refresh_millihertz);
    state.runtime.configure_output_modes();
    let mut damage = OutputDamageTracker::new(backend.window_size(), 1.0, Transform::Flipped180);
    let mut wallpapers = WallpaperCache::default();
    let mut titlebars = TitlebarCache::default();
    let mut overview = OverviewCache::default();
    let mut rounded = None;
    let mut blur = None;
    if scheduler.request_redraw() {
        backend.window().request_redraw();
    }
    event_loop
        .handle()
        .insert_source(events, move |event, _, state| match event {
            WinitEvent::Resized { size, .. } if size.w > 0 && size.h > 0 => {
                resize(state, size, host_timing.refresh_millihertz);
                damage = OutputDamageTracker::new(size, 1.0, Transform::Flipped180);
                if scheduler.request_redraw() {
                    backend.window().request_redraw();
                }
            }
            WinitEvent::Input(event) => state.process_input(event),
            WinitEvent::Focus(focused) => {
                state.host_focused = focused;
                if !focused {
                    state.end_drag();
                    state.titlebar_press = None;
                    state.titlebar_drag = None;
                    state.overview_press = None;
                    state.overview_drag = None;
                    state.runtime.cancel_overview();
                    overview.clear();
                }
                state.dirty = true;
            }
            WinitEvent::Redraw => {
                let now = state.start.elapsed();
                let current_timing = self::host_timing(backend.window());
                if current_timing != host_timing {
                    host_timing = current_timing;
                    report_timing(host_timing);
                    update_output_refresh(state, host_timing.refresh_millihertz);
                }
                scheduler.update(host_timing, state.runtime.config.animations.frame_rate, now);
                let timing = scheduler.redraw(now);
                state.frame_time = timing.now;
                state.animation_sample_time = timing.animation_sample;
                let size = backend.window_size();
                if size.w <= 0 || size.h <= 0 {
                    return;
                }
                state.reconcile();
                let result = (|| -> Result<(), String> {
                    let (renderer, mut framebuffer) = backend
                        .bind()
                        .map_err(|e| format!("bind framebuffer: {e}"))?;
                    let radius = state.runtime.config.theme.blur_radius;
                    if !radius.is_finite() || !(0.0..=32.0).contains(&radius) {
                        return Err("blur radius must be finite and in 0..=32".into());
                    }
                    if (state.runtime.config.theme.corner_radius.is_rounded()
                        || state.runtime.overview.is_some())
                        && rounded.is_none()
                    {
                        rounded = Some(
                            RoundedShaders::new(renderer)
                                .map_err(|e| format!("compile rounded window shaders: {e}"))?,
                        );
                    }
                    let scene = state
                        .scene_elements(
                            renderer,
                            &mut wallpapers,
                            &mut titlebars,
                            rounded.as_ref(),
                            radius > 0.0 || state.runtime.config.theme.liquid_glass.enabled,
                        )
                        .map_err(|e| format!("compose scene: {e}"))?;
                    let background = premultiply(state.runtime.config.theme.background);
                    let mut final_elements = if let Some(shaders) = rounded.as_ref() {
                        overview
                            .elements(state, renderer, &mut titlebars, shaders, &mut wallpapers)
                            .map_err(|e| format!("compose overview: {e}"))?
                    } else {
                        overview.clear();
                        Vec::new()
                    };
                    if radius > 0.0 || state.runtime.config.theme.liquid_glass.enabled {
                        if blur.is_none() {
                            blur = Some(
                                BackdropBlur::new(renderer)
                                    .map_err(|e| format!("compile backdrop shaders: {e}"))?,
                            );
                        }
                        let composed = blur
                            .as_mut()
                            .expect("blur initialized")
                            .render(
                                renderer,
                                &scene,
                                size,
                                radius,
                                state.runtime.config.theme.blur_method,
                                state.runtime.config.theme.blur_passes,
                                state.runtime.config.theme.liquid_glass,
                                background,
                            )
                            .map_err(|e| format!("compose backdrop blur: {e}"))?;
                        final_elements.push(SceneElement::Wallpaper(composed));
                    } else {
                        final_elements.extend(scene.elements);
                    }
                    // Restore the window target after all thumbnail/blur offscreen work.
                    damage
                        .render_output(renderer, &mut framebuffer, 0, &final_elements, background)
                        .map_err(|e| format!("present scene: {e}"))?;
                    // GlesRenderer::bind only wraps a target. The full-damage
                    // window render above makes its EGL context/surface current.
                    synchronize_swaps()?;
                    if timing.now >= capture_after {
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
                // Client/layer callbacks and content still use continuous host
                // opportunities until every scene invalidation has a wakeup.
                if scheduler.request_redraw() {
                    backend.window().request_redraw();
                }
            }
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            _ => {}
        })?;
    Ok(())
}

fn resize(state: &mut Compositor, size: Size<i32, Physical>, refresh_millihertz: u32) {
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
            refresh: refresh_millihertz as i32,
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
            .add_output_with_bounds(region.id, config.name.clone(), rect, rect);
    }
    state.dirty = true;
}

fn host_timing(window: &dyn smithay::reexports::winit::window::Window) -> HostTiming {
    let mode = window.current_monitor().and_then(|monitor| {
        let mode = monitor.current_video_mode()?;
        Some((monitor.id(), mode.refresh_rate_millihertz()?.get()))
    });
    HostTiming::from_monitor(mode)
}

fn report_timing(timing: HostTiming) {
    eprintln!(
        "clear: nested refresh={} mHz source={}; synchronized EGL interval=1 requested",
        timing.refresh_millihertz,
        timing.source(),
    );
}

fn update_output_refresh(state: &mut Compositor, refresh_millihertz: u32) {
    for region in &state.outputs {
        if let Some(mut mode) = region.output.current_mode() {
            mode.refresh = refresh_millihertz as i32;
            region
                .output
                .change_current_state(Some(mode), None, None, None);
            region.output.set_preferred(mode);
        }
    }
}

fn synchronize_swaps() -> Result<(), String> {
    // The window render has made its live EGL display and surface current on
    // this thread. The pinned GlAttributes only selects a compatible config;
    // explicitly set interval 1 for the current (possibly recreated) surface.
    let result = unsafe { egl::SwapInterval(egl::GetCurrentDisplay(), 1) };
    if result == egl::FALSE {
        // Read the error on the same thread immediately after the failed call.
        let error = unsafe { egl::GetError() };
        return Err(format!(
            "enable synchronized EGL swaps: EGL error {error:#x}"
        ));
    }
    Ok(())
}
