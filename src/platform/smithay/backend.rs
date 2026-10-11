use super::{
    blur::BackdropBlur,
    fps::FpsOverlay,
    frame_scheduler::{CallbackClock, FrameScheduler, HostTiming},
    overview::OverviewCache,
    render_demand::Reason,
    rounded::RoundedShaders,
    scene::premultiply,
    scene::{Scene, SceneElement},
    state::{Compositor, OutputRegion},
    titlebar::TitlebarCache,
    wallpaper::WallpaperCache,
    window_image::WindowImageComposer,
};
use crate::core::{OutputId, Rect};
use smithay::{
    backend::{
        allocator::Fourcc,
        egl::{context::GlAttributes, ffi::egl},
        renderer::{
            ExportMem, Frame, Renderer,
            damage::OutputDamageTracker,
            gles::{GlesRenderer, GlesTarget},
        },
        winit::{self, WinitEvent},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::{EventLoop, ping::make_ping},
        winit::{dpi::LogicalSize, window::WindowAttributes},
    },
    utils::{Physical, Rectangle, Size, Transform},
};
use std::{cell::RefCell, rc::Rc, time::Duration};

#[derive(Default, serde::Serialize)]
struct Stats {
    requests: u64,
    redraws: u64,
    submissions: u64,
    callback_only: u64,
    diagnostic_frames: u64,
    loop_turns: u64,
    work_submissions: u64,
    last_scene_submission_ms: Option<u128>,
    damage_skipped: u64,
    partial_frames: u64,
    partial_captures: u64,
    uncertain_frames: u64,
    last_uncertain_submission_ms: Option<u128>,
    full_frames: u64,
}

struct Control {
    scheduler: FrameScheduler,
    callbacks: CallbackClock,
    fps: FpsOverlay,
    capture_deadline: Option<Duration>,
    stats: Stats,
}

pub(super) struct BackendHandle {
    backend: Rc<RefCell<winit::WinitGraphicsBackend<GlesRenderer>>>,
    control: Rc<RefCell<Control>>,
}

impl BackendHandle {
    /// Service protocol-only deadlines and queue at most one host draw.
    /// Returns the next explicit absolute deadline, or none for indefinite sleep.
    pub fn service(&self, state: &mut Compositor) -> Option<Duration> {
        let now = state.start.elapsed();
        let mut control = self.control.borrow_mut();
        control.stats.loop_turns += 1;
        control.fps.set_enabled(state.runtime.config.show_fps);
        if control
            .capture_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            control.capture_deadline = None;
            state.render_demand.invalidate(Reason::Capture);
        }
        let motion = state.has_animation_demand();
        if motion
            || (state.render_demand.visual_pending() && state.render_demand.is_nondiagnostic())
        {
            control.fps.visual_work(now);
        } else if control.fps.deadline(now) {
            state.render_demand.invalidate(Reason::Diagnostic);
        }
        let visual = motion || state.render_demand.visual_pending();
        let mut deadline = control.capture_deadline;
        if visual {
            let backend = self.backend.borrow();
            let size = backend.window_size();
            if size.w > 0 && size.h > 0 && control.scheduler.request_redraw() {
                control.stats.requests += 1;
                backend.window().request_redraw();
            }
        } else if let Some(eligible) = state.next_frame_callback_deadline(now) {
            if eligible <= now && control.callbacks.due(now) {
                let delivered = state.frame_callbacks(now);
                control.callbacks.delivered(now);
                if delivered > 0 {
                    control.stats.callback_only += 1;
                }
            }
            if let Some(eligible) = state.next_frame_callback_deadline(now) {
                deadline = earliest(
                    deadline,
                    Some(control.callbacks.deadline(now).max(eligible)),
                );
            }
        }
        // A host opportunity may be pending indefinitely. Do not repeatedly
        // rearm overdue callback/HUD work while waiting for it.
        if !visual {
            deadline = earliest(deadline, control.fps.next_deadline(now));
        }
        deadline
    }

    pub fn report(&self) {
        if std::env::var_os("CLEAR_RENDER_STATS").is_some_and(|value| value == "1") {
            let control = self.control.borrow();
            let s = &control.stats;
            eprintln!(
                "clear: render stats {}",
                serde_json::to_string(s).expect("bounded render statistics")
            );
        }
    }
}

fn earliest(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

pub(super) fn init(
    event_loop: &EventLoop<Compositor>,
    state: &mut Compositor,
    mut capture: Option<std::path::PathBuf>,
    capture_after: std::time::Duration,
) -> Result<BackendHandle, Box<dyn std::error::Error>> {
    let width: i32 = state.runtime.config.outputs.iter().map(|o| o.width).sum();
    let height = state
        .runtime
        .config
        .outputs
        .iter()
        .map(|o| o.height)
        .max()
        .unwrap_or(600);
    let partial_capture =
        std::env::var_os("CLEAR_PARTIAL_FRAME_CAPTURE").map(std::path::PathBuf::from);
    let attributes = WindowAttributes::default()
        .with_title("Clear — nested compositor")
        .with_surface_size(LogicalSize::new(f64::from(width), f64::from(height)))
        .with_visible(true);
    let (backend, events) = winit::init_from_attributes_with_gl_attr::<GlesRenderer>(
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
    let scheduler = FrameScheduler::new(
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
    let mut window_images = WindowImageComposer::default();
    let mut rounded = None;
    let mut blur = None;
    let backend = Rc::new(RefCell::new(backend));
    let control = Rc::new(RefCell::new(Control {
        scheduler,
        callbacks: CallbackClock::new(host_timing),
        fps: FpsOverlay::default(),
        capture_deadline: capture.as_ref().map(|_| capture_after),
        stats: Stats::default(),
    }));
    let (ping, source) = make_ping()?;
    state.render_demand.set_ping(ping);
    event_loop.handle().insert_source(source, |_, _, _| {})?;
    let handle = BackendHandle {
        backend: backend.clone(),
        control: control.clone(),
    };
    event_loop
        .handle()
        .insert_source(events, move |event, _, state| {
            let mut backend = backend.borrow_mut();
            let mut control = control.borrow_mut();
            match event {
                WinitEvent::Resized { size, .. } if size.w > 0 && size.h > 0 => {
                    state.window_animations.reset();
                    state.clear_overview_animation();
                    window_images.clear();
                    overview.clear();
                    resize(state, size, host_timing.refresh_millihertz);
                    damage = OutputDamageTracker::new(size, 1.0, Transform::Flipped180);
                    state.render_demand.invalidate(Reason::Resize);
                }
                WinitEvent::Input(event) => state.process_input(event),
                WinitEvent::Focus(focused) => {
                    state.host_focused = focused;
                    if !focused {
                        state.end_drag();
                        state.titlebar_press = None;
                        state.titlebar_drag = None;
                        state.overview_press = None;
                        state.take_overview_drag();
                        state.runtime.cancel_overview();
                        state.clear_overview_animation();
                        state.window_animations.reset();
                        window_images.clear();
                        overview.clear();
                    }
                    state.dirty = true;
                    state.render_demand.invalidate(Reason::Exposure);
                }
                WinitEvent::Redraw => {
                    let now = state.start.elapsed();
                    control.stats.redraws += 1;
                    // An unsolicited redraw is host exposure; a requested redraw already
                    // carries its visual reason or active motion.
                    if !control.scheduler.redraw_pending() {
                        state.render_demand.invalidate(Reason::Exposure);
                    }
                    control.fps.set_enabled(state.runtime.config.show_fps);
                    let current_timing = self::host_timing(backend.window());
                    if current_timing != host_timing {
                        host_timing = current_timing;
                        report_timing(host_timing);
                        update_output_refresh(state, host_timing.refresh_millihertz);
                    }
                    control.scheduler.update(
                        host_timing,
                        state.runtime.config.animations.frame_rate,
                        now,
                    );
                    control.callbacks.update(host_timing, now);
                    let timing = control.scheduler.redraw(now);
                    state.frame_time = timing.now;
                    state.animation_sample_time = timing.animation_sample;
                    let size = backend.window_size();
                    if size.w <= 0 || size.h <= 0 {
                        return;
                    }
                    state.reconcile();
                    state.sample_overview_animation();
                    state.sample_overview_drag();
                    let generation = state.render_demand.generation();
                    let motion = state.has_animation_demand();
                    let scene_work = motion || state.render_demand.is_scene_work();
                    let uncertain = state.render_demand.has_reason(Reason::UncertainCommit);
                    let full_frame = motion
                        || state.render_demand.requires_full_frame()
                        || state.runtime.config.theme.blur_radius > 0.0
                        || state.runtime.config.theme.liquid_glass.enabled;
                    let diagnostic =
                        !state.render_demand.is_nondiagnostic() && !state.has_animation_demand();
                    if !diagnostic {
                        control.fps.visual_work(now);
                    }
                    if !state.overview_present() && !state.runtime.config.animations.enabled {
                        window_images.clear();
                    }
                    let mut partial_captured = false;
                    let result = (|| -> Result<Option<Vec<Rectangle<i32, Physical>>>, String> {
                        let age = current_window_age(&mut backend)?;
                        let age = if full_frame { 0 } else { age };
                        let (renderer, mut framebuffer) = backend
                            .bind()
                            .map_err(|e| format!("bind framebuffer: {e}"))?;
                        let radius = state.runtime.config.theme.blur_radius;
                        if !radius.is_finite() || !(0.0..=32.0).contains(&radius) {
                            return Err("blur radius must be finite and in 0..=32".into());
                        }
                        if (state.runtime.config.theme.corner_radius.is_rounded()
                            || state.overview_present()
                            || state.runtime.config.animations.enabled)
                            && rounded.is_none()
                        {
                            rounded = Some(
                                RoundedShaders::new(renderer)
                                    .map_err(|e| format!("compile rounded window shaders: {e}"))?,
                            );
                        }
                        if let Some(shaders) = rounded.as_ref() {
                            state
                                .prepare_window_animations(
                                    renderer,
                                    &mut titlebars,
                                    shaders,
                                    &mut window_images,
                                )
                                .map_err(|e| format!("prepare window animations: {e}"))?;
                        }
                        if state.overview_present() {
                            titlebars.retain(|id| {
                                state.windows.get(&id).is_some_and(|entry| entry.uses_ssd())
                            });
                            wallpapers.retain(&state.runtime.wallpapers);
                        }
                        let background = premultiply(state.runtime.config.theme.background);
                        let overview_scene = if let Some(shaders) = rounded.as_ref() {
                            overview
                                .elements(
                                    state,
                                    renderer,
                                    &mut titlebars,
                                    shaders,
                                    &mut wallpapers,
                                    &mut window_images,
                                )
                                .map_err(|e| format!("compose overview: {e}"))?
                        } else {
                            overview.clear();
                            Scene {
                                elements: Vec::new(),
                                groups: Vec::new(),
                            }
                        };
                        // Overview supplies opaque backing for every output. Preparing
                        // hidden desktop trees would import and filter invisible pixels.
                        let mut scene = if !overview_scene.elements.is_empty() {
                            overview_scene
                        } else {
                            state
                                .scene_elements(
                                    renderer,
                                    &mut wallpapers,
                                    &mut titlebars,
                                    rounded.as_ref(),
                                    radius > 0.0 || state.runtime.config.theme.liquid_glass.enabled,
                                )
                                .map_err(|e| format!("compose scene: {e}"))?
                        };
                        if state.overview_shell_reveal().is_some() {
                            // Shell filters sample the live overview scene, whose own
                            // preview filters have already composited their backdrops.
                            scene.prepend(state.overview_shell_scene(renderer));
                        }
                        let mut final_elements = Vec::new();
                        if (radius > 0.0 || state.runtime.config.theme.liquid_glass.enabled)
                            && scene.groups.iter().any(|group| group.mask.is_some())
                        {
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
                        if let Some(element) = control
                            .fps
                            .element(renderer, &mut titlebars, size)
                            .map_err(|e| format!("prepare FPS overlay: {e}"))?
                        {
                            final_elements.insert(0, element);
                        }
                        // Restore the window target after all thumbnail/blur offscreen work.
                        let repaired = damage
                            .render_output(
                                renderer,
                                &mut framebuffer,
                                age,
                                &final_elements,
                                background,
                            )
                            .map_err(|e| format!("present scene: {e}"))?
                            .damage
                            .cloned();
                        if repaired.is_none() {
                            return Ok(None);
                        }
                        // The actual window render makes its EGL target current.
                        synchronize_swaps()?;
                        if timing.now >= capture_after {
                            if let Some(path) = capture.take() {
                                capture_framebuffer(renderer, &mut framebuffer, size, &path)?;
                                eprintln!("clear: captured {}", path.display());
                            }
                        }
                        if !full_frame
                            && control.stats.partial_captures < 16
                            && repaired.as_ref().is_some_and(|rects| {
                                rects.as_slice() != [Rectangle::from_size(size)]
                            })
                        {
                            if let Some(path) = partial_capture.as_ref() {
                                let temporary = path.with_extension("partial.tmp");
                                capture_framebuffer(renderer, &mut framebuffer, size, &temporary)?;
                                std::fs::rename(&temporary, path)
                                    .map_err(|e| format!("publish partial capture: {e}"))?;
                                partial_captured = true;
                            }
                        }
                        if uncertain && control.stats.uncertain_frames < 16 {
                            if let Some(path) = partial_capture.as_ref() {
                                let recovery = path.with_extension("uncertain.ppm");
                                let temporary = path.with_extension("uncertain.tmp");
                                capture_framebuffer(renderer, &mut framebuffer, size, &temporary)?;
                                std::fs::rename(&temporary, recovery)
                                    .map_err(|e| format!("publish recovery capture: {e}"))?;
                            }
                        }
                        Ok(repaired)
                    })();
                    let repaired = match result {
                        Ok(Some(damage)) => damage,
                        Ok(None) => {
                            // No changed pixels: the tracker proves this scene is
                            // already submitted. Motion/recovery is forced full,
                            // so this never acknowledges a terminal pose.
                            state.render_demand.submitted(generation);
                            control.stats.damage_skipped += 1;
                            let now = state.start.elapsed();
                            if control.callbacks.due(now)
                                && state
                                    .next_frame_callback_deadline(now)
                                    .is_some_and(|eligible| eligible <= now)
                            {
                                let delivered = state.frame_callbacks(now);
                                control.callbacks.delivered(now);
                                if delivered > 0 {
                                    control.stats.callback_only += 1;
                                }
                            }
                            return;
                        }
                        Err(error) => {
                            state.fail(error);
                            return;
                        }
                    };
                    if let Err(error) = backend.submit(Some(&repaired)) {
                        state.fail(format!("submit frame: {error}"));
                        return;
                    }
                    let submitted_at = state.start.elapsed();
                    control.fps.submitted(submitted_at);
                    control.callbacks.delivered(submitted_at);
                    control.stats.submissions += 1;
                    if uncertain {
                        control.stats.uncertain_frames += 1;
                        control.stats.last_uncertain_submission_ms = Some(submitted_at.as_millis());
                    }
                    if partial_captured {
                        control.stats.partial_captures += 1;
                    }
                    if repaired == [Rectangle::from_size(size)] {
                        control.stats.full_frames += 1;
                    } else {
                        control.stats.partial_frames += 1;
                    }
                    if scene_work {
                        control.stats.work_submissions += 1;
                        control.stats.last_scene_submission_ms = Some(submitted_at.as_millis());
                    }
                    if diagnostic {
                        control.stats.diagnostic_frames += 1;
                    }
                    state.render_demand.submitted(generation);
                    state.acknowledge_window_animations();
                    state.mark_overview_presented();
                    state.mark_overview_drag_presented();
                    state.refresh_pointer_animation_focus();
                    state.frame_callbacks(submitted_at);
                }
                WinitEvent::CloseRequested => state.loop_signal.stop(),
                _ => {}
            }
        })?;
    Ok(handle)
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

/// Resize/bind before latching the back buffer, then query its actual EGL age.
fn current_window_age(
    backend: &mut winit::WinitGraphicsBackend<GlesRenderer>,
) -> Result<usize, String> {
    {
        // The pin handles native-surface resize here. Gles bind itself only
        // constructs a target; it does not make that window surface current.
        let _ = backend
            .bind()
            .map_err(|e| format!("bind for buffer age: {e}"))?;
    }
    let surface = backend.egl_surface().get_surface_handle();
    let context = backend.renderer().egl_context();
    // Equivalent to EGLContext::make_current_with_surface. Copying the live raw
    // surface handle avoids borrowing backend's surface and renderer together.
    // Both are owned by this backend, alive throughout this same-thread call;
    // no resize/offscreen pass intervenes and the context is never shared across
    // threads. This makes the correct window target current without an extra
    // empty GPU frame/fence merely to obtain the age.
    let result = unsafe {
        egl::MakeCurrent(
            **context.display().get_display_handle(),
            surface,
            surface,
            context.get_context_handle(),
        )
    };
    if result == egl::FALSE {
        let error = unsafe { egl::GetError() };
        return Err(format!(
            "make window current for buffer age: EGL error {error:#x}"
        ));
    }
    Ok(backend.buffer_age().unwrap_or(0))
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

/// Read a complete repaired back buffer and restore its window target before swap.
fn capture_framebuffer(
    renderer: &mut GlesRenderer,
    framebuffer: &mut GlesTarget<'_>,
    size: Size<i32, Physical>,
    path: &std::path::Path,
) -> Result<(), String> {
    use std::io::Write;
    let mapping = renderer
        .copy_framebuffer(
            framebuffer,
            Rectangle::from_size((size.w, size.h).into()),
            Fourcc::Abgr8888,
        )
        .map_err(|e| format!("capture framebuffer: {e}"))?;
    let pixels = renderer
        .map_texture(&mapping)
        .map_err(|e| format!("map capture: {e}"))?;
    let mut file = std::io::BufWriter::new(
        std::fs::File::create(path).map_err(|e| format!("create capture: {e}"))?,
    );
    write!(file, "P6\n{} {}\n255\n", size.w, size.h).map_err(|e| e.to_string())?;
    // EGL readback starts at the bottom row; PPM starts at the top.
    for row in pixels.chunks_exact(size.w as usize * 4).rev() {
        for pixel in row.chunks_exact(4) {
            file.write_all(&pixel[..3]).map_err(|e| e.to_string())?;
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    // Mapping makes the context surfaceless. An empty frame restores the window
    // target without changing the repaired pixels or submitted damage.
    renderer
        .render(framebuffer, size, Transform::Flipped180)
        .map_err(|e| format!("restore capture target: {e}"))?
        .finish()
        .map_err(|e| format!("finish capture target: {e}"))?
        .wait()
        .map_err(|e| format!("wait for capture target: {e}"))?;
    Ok(())
}
