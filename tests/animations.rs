use clear::{
    config::{
        AnimationCurve, AnimationEffect, AnimationFrameRate, AnimationKind, AnimationsConfig,
        Config,
    },
    runtime::{
        Runtime,
        animation::{
            AnimationClock, AnimationEngine, AnimationPose, AnimationRect, MAX_ANIMATION_TRACKS,
        },
    },
};
use std::{fs, time::Duration};

fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value)
}
fn pose(x: f64, width: f64, opacity: f64) -> AnimationPose {
    AnimationPose {
        rect: AnimationRect {
            x,
            y: 20.0,
            width,
            height: 80.0,
        },
        opacity,
    }
}
fn enabled() -> AnimationsConfig {
    AnimationsConfig {
        enabled: true,
        ..Default::default()
    }
}
fn engine() -> AnimationEngine {
    AnimationEngine::new(enabled(), Duration::ZERO).unwrap()
}
fn start(engine: &mut AnimationEngine, id: u64, effect: AnimationEffect) {
    engine
        .retarget(
            id,
            7,
            effect,
            pose(0.0, 100.0, 0.0),
            pose(200.0, 200.0, 1.0),
            Duration::ZERO,
            1.0,
        )
        .unwrap();
}

#[test]
fn animation_defaults_are_complete_but_visible_effects_are_opt_in() {
    let config = Config::from_source("").unwrap().animations;
    assert_eq!(config, AnimationsConfig::default());
    assert!(!config.enabled);
    assert!(!config.reduced_motion);
    assert_eq!(config.frame_rate, AnimationFrameRate::RefreshRate);
    assert_eq!(
        config.window_open.kind,
        AnimationKind::Easing {
            duration_ms: 150,
            curve: AnimationCurve::EaseOutExpo
        }
    );
    assert_eq!(config.window_open.scale, 1.04);
    assert_eq!(
        config.workspace_switch.kind,
        AnimationKind::Spring { stiffness: 1000.0 }
    );
    assert_eq!(
        Config::from_source("[animations.window_open]\nenabled=false")
            .unwrap()
            .animations
            .window_open
            .kind,
        config.window_open.kind
    );
}

#[test]
fn animation_schema_accepts_caps_and_each_effect_kind() {
    for rate in ["'refresh-rate'", "30", "60", "120"] {
        let config = Config::from_source(&format!("[animations]\nframe_rate={rate}")).unwrap();
        assert_eq!(
            config.animations.frame_rate,
            if rate.starts_with('\'') {
                AnimationFrameRate::RefreshRate
            } else {
                AnimationFrameRate::Fixed(rate.parse().unwrap())
            }
        );
    }
    for effect in [
        "workspace_switch",
        "window_open",
        "window_close",
        "window_movement",
        "minimize",
        "maximize",
        "overview",
        "fullscreen",
    ] {
        for kind in [
            "kind='easing'\nduration_ms=0\ncurve='linear'",
            "kind='spring'\nstiffness=1.0",
        ] {
            Config::from_source(&format!("[animations.{effect}]\n{kind}")).unwrap();
        }
    }
    for speed in [0.1, 10.0] {
        Config::from_source(&format!("[animations]\nspeed={speed}")).unwrap();
    }
    for scale in [1.0, 1.25] {
        Config::from_source(&format!("[animations.window_open]\nscale={scale}")).unwrap();
    }
}

#[test]
fn animation_schema_rejects_unknown_mixed_and_nonfinite_fields_even_when_disabled() {
    for invalid in [
        "speed=nan",
        "speed=inf",
        "speed=0.0",
        "speed=10.1",
        "frame_rate=59",
        "frame_rate=60.0",
        "frame_rate='60'",
        "frame_rate='refresh'",
        "enabled='yes'",
        "typo=true",
        "[animations.window_open]\nkind='easing'\nstiffness=20",
        "[animations.window_open]\nduration_ms=-1",
        "[animations.window_open]\nduration_ms=2001",
        "[animations.window_open]\ncurve='bouncy'",
        "[animations.window_open]\nscale=nan",
        "[animations.window_close]\nscale=1.26",
        "[animations.window_movement]\nscale=1.0",
        "[animations.overview]\nkind='spring'\nduration_ms=20",
        "[animations.maximize]\nstiffness=inf",
        "[animations.fullscreen]\nstiffness=0.0",
        "[animations.minimize]\nkind='bounce'",
        "[animations.workspace_switch]\nstiffness=10001.0",
        "[animations.overview]\nunknown=1",
    ] {
        assert!(
            Config::from_source(&format!("[animations]\nenabled=false\n{invalid}")).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn epoch_rebase_preserves_pose_and_predictions_do_not_advance_time() {
    let mut clock = AnimationClock::new(Duration::ZERO, 1.0).unwrap();
    assert_eq!(clock.sample(seconds(20.0)), 20.0);
    assert_eq!(clock.sample(seconds(0.1)), 0.1);
    clock.rebase(seconds(0.1), 2.0).unwrap();
    assert_eq!(clock.sample(seconds(0.1)), 0.1);
    assert!((clock.sample(seconds(0.2)) - 0.3).abs() < 1e-12);
    assert!(clock.rebase(Duration::ZERO, 1.0).is_err());
    assert!(clock.rebase(seconds(0.2), f64::NAN).is_err());
    assert!((clock.sample(seconds(0.2)) - 0.3).abs() < 1e-12);
}

#[test]
fn poses_are_independent_of_rate_and_skipped_samples() {
    for effect in [AnimationEffect::WindowOpen, AnimationEffect::WindowMovement] {
        let mut reference = engine();
        start(&mut reference, 1, effect);
        let expected = reference.sample(1, seconds(0.123)).unwrap();
        for fps in [30, 60, 120, 144] {
            let mut sampled = engine();
            start(&mut sampled, 1, effect);
            for frame in 0..fps {
                sampled.sample(1, seconds(f64::from(frame) / f64::from(fps)));
            }
            assert_eq!(sampled.sample(1, seconds(0.123)), Some(expected));
        }
    }
}

#[test]
fn speed_reload_preserves_active_pose_and_keeps_captured_curve() {
    let mut engine = engine();
    start(&mut engine, 1, AnimationEffect::WindowOpen);
    let before = engine.sample(1, seconds(0.05)).unwrap();
    let mut config = enabled();
    config.speed = 2.0;
    config.window_open.kind = AnimationKind::Easing {
        duration_ms: 2000,
        curve: AnimationCurve::Linear,
    };
    engine.apply_config(config.clone(), seconds(0.05)).unwrap();
    assert_eq!(engine.sample(1, seconds(0.05)), Some(before));
    assert!(engine.sample(1, seconds(0.1)).unwrap().finished);
    engine
        .retarget(
            2,
            8,
            AnimationEffect::WindowOpen,
            pose(0.0, 100.0, 0.0),
            pose(200.0, 200.0, 1.0),
            seconds(0.1),
            1.0,
        )
        .unwrap();
    assert!(!engine.sample(2, seconds(0.2)).unwrap().finished);
    config.speed = f64::NAN;
    let before = engine.sample(2, seconds(0.2));
    assert!(engine.apply_config(config, seconds(0.2)).is_err());
    assert_eq!(engine.sample(2, seconds(0.2)), before);
}

#[test]
fn repeated_spring_retargeting_preserves_position_and_velocity() {
    let mut engine = engine();
    start(&mut engine, 1, AnimationEffect::WindowMovement);
    let t = seconds(0.1);
    let current = engine.sample(1, t).unwrap().pose;
    let dt = 0.000001;
    let old_velocity =
        (engine.sample(1, seconds(0.1 + dt)).unwrap().pose.rect.x - current.rect.x) / dt;
    engine
        .retarget(
            1,
            9,
            AnimationEffect::WindowMovement,
            pose(-999.0, 100.0, 0.0),
            pose(-200.0, 50.0, 0.0),
            t,
            1.0,
        )
        .unwrap();
    assert_eq!(engine.sample(1, t).unwrap().pose, current);
    let new_velocity =
        (engine.sample(1, seconds(0.1 + dt)).unwrap().pose.rect.x - current.rect.x) / dt;
    assert!((old_velocity - new_velocity).abs() < 0.3);
    assert_eq!(engine.group(1), Some(9));
    for step in 1..20 {
        let t = seconds(0.1 + f64::from(step) * 0.02);
        let current = engine.sample(1, t).unwrap().pose;
        let target = pose(
            if step % 2 == 0 { 200.0 } else { -200.0 },
            50.0,
            if step % 2 == 0 { 1.0 } else { 0.0 },
        );
        engine
            .retarget(
                1,
                9,
                AnimationEffect::WindowMovement,
                current,
                target,
                t,
                1.0,
            )
            .unwrap();
        assert_eq!(engine.sample(1, t).unwrap().pose, current);
    }
    let final_pose = engine.sample(1, seconds(10.0)).unwrap();
    assert!(final_pose.finished);
    assert_eq!(final_pose.pose, pose(-200.0, 50.0, 0.0));
}

#[test]
fn grouped_tracks_share_progress_and_settle_exactly_by_elapsed_time() {
    let mut engine = engine();
    start(&mut engine, 1, AnimationEffect::WindowMovement);
    engine
        .retarget(
            2,
            7,
            AnimationEffect::WindowMovement,
            pose(20.0, 200.0, 0.0),
            pose(420.0, 400.0, 1.0),
            Duration::ZERO,
            1.0,
        )
        .unwrap();
    let a = engine.sample(1, seconds(0.1)).unwrap().pose;
    let b = engine.sample(2, seconds(0.1)).unwrap().pose;
    assert!((b.rect.x - (a.rect.x * 2.0 + 20.0)).abs() < 1e-10);
    let mut slow = enabled();
    slow.speed = 0.1;
    slow.window_movement.kind = AnimationKind::Spring { stiffness: 1.0 };
    let mut engine = AnimationEngine::new(slow, Duration::ZERO).unwrap();
    start(&mut engine, 1, AnimationEffect::WindowMovement);
    assert!(!engine.sample(1, seconds(19.9)).unwrap().finished);
    assert_eq!(
        engine.sample(1, seconds(20.0)).unwrap(),
        clear::runtime::animation::AnimationSample {
            pose: pose(200.0, 200.0, 1.0),
            finished: true
        }
    );
}

#[test]
fn instant_reduced_motion_and_effect_disable_settle_without_replay() {
    for config in [
        AnimationsConfig::default(),
        AnimationsConfig {
            reduced_motion: true,
            ..enabled()
        },
        {
            let mut c = enabled();
            c.window_open.kind = AnimationKind::Easing {
                duration_ms: 0,
                curve: AnimationCurve::Linear,
            };
            c
        },
    ] {
        let mut engine = AnimationEngine::new(config, Duration::ZERO).unwrap();
        start(&mut engine, 1, AnimationEffect::WindowOpen);
        assert!(engine.sample(1, Duration::ZERO).unwrap().finished);
    }
    let mut engine = engine();
    start(&mut engine, 1, AnimationEffect::WindowOpen);
    start(&mut engine, 2, AnimationEffect::WindowMovement);
    let mut config = enabled();
    config.window_open.enabled = false;
    engine.apply_config(config, seconds(0.01)).unwrap();
    assert!(engine.sample(1, seconds(0.01)).unwrap().finished);
    assert!(!engine.sample(2, seconds(0.01)).unwrap().finished);
    engine.apply_config(enabled(), seconds(0.02)).unwrap();
    assert!(engine.sample(1, seconds(0.02)).unwrap().finished);
    engine.settle(2);
    assert!(engine.sample(2, seconds(0.02)).unwrap().finished);
    engine.remove(1);
    engine.remove(2);
    assert!(engine.is_empty());
}

#[test]
fn track_budget_replaces_ids_and_rejects_invalid_geometry() {
    let mut engine = engine();
    for id in 0..MAX_ANIMATION_TRACKS as u64 {
        start(&mut engine, id, AnimationEffect::WindowOpen);
    }
    start(&mut engine, 0, AnimationEffect::WindowOpen);
    assert_eq!(engine.len(), MAX_ANIMATION_TRACKS);
    assert!(
        engine
            .retarget(
                999,
                1,
                AnimationEffect::WindowOpen,
                pose(0.0, 100.0, 0.0),
                pose(1.0, 100.0, 1.0),
                Duration::ZERO,
                1.0
            )
            .is_err()
    );
    engine.remove(0);
    assert!(
        engine
            .retarget(
                999,
                1,
                AnimationEffect::WindowOpen,
                pose(f64::NAN, 100.0, 0.0),
                pose(1.0, 100.0, 1.0),
                Duration::ZERO,
                1.0
            )
            .is_err()
    );
    assert!(
        engine
            .retarget(
                999,
                1,
                AnimationEffect::WindowOpen,
                pose(0.0, 0.0, 0.0),
                pose(1.0, 100.0, 1.0),
                Duration::ZERO,
                1.0
            )
            .is_err()
    );
}

#[test]
fn animation_reload_is_atomic_with_other_config_and_resource_failures() {
    let dir = std::env::temp_dir().join(format!("clear-animations-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    fs::write(&path, "gaps=12\n[animations]\nenabled=true\nspeed=2.0").unwrap();
    let mut runtime = Runtime::load(Some(path.clone())).unwrap();
    for source in [
        "gaps=99\n[animations]\nspeed=nan",
        "gaps=99\nscript='missing.rhai'\n[animations]\nspeed=3.0",
    ] {
        fs::write(&path, source).unwrap();
        assert!(runtime.reload().is_err());
        assert_eq!(runtime.config.gaps, 12);
        assert_eq!(runtime.config.animations.speed, 2.0);
    }
    fs::write(&path, "gaps=13\n[animations]\nspeed=0.5\nframe_rate=30").unwrap();
    runtime.reload().unwrap();
    assert_eq!(runtime.config.gaps, 13);
    assert_eq!(runtime.config.animations.speed, 0.5);
    assert_eq!(
        runtime.config.animations.frame_rate,
        AnimationFrameRate::Fixed(30)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn example_animation_config_parses() {
    for source in [
        include_str!("../examples/config.toml"),
        include_str!("../examples/liquid-glass.toml"),
    ] {
        Config::from_source(source).unwrap();
    }
}

#[test]
fn extreme_finite_endpoints_are_rejected_without_replacing_a_track() {
    let mut engine = engine();
    start(&mut engine, 1, AnimationEffect::WindowMovement);
    let before = engine.sample(1, seconds(0.1));
    for effect in [AnimationEffect::WindowOpen, AnimationEffect::WindowMovement] {
        for (from, to) in [
            (pose(1e308, 100.0, 0.0), pose(-1e308, 100.0, 1.0)),
            (pose(0.0, 100.0, 0.0), pose(0.0, 1e308, 1.0)),
            (pose(0.0, 100.0, 0.0), pose(-1e308, 100.0, 1.0)),
        ] {
            assert!(
                engine
                    .retarget(1, 9, effect, from, to, seconds(0.1), 1.0)
                    .is_err()
            );
            assert_eq!(engine.sample(1, seconds(0.1)), before);
        }
    }
}

#[test]
fn extreme_physical_scales_and_repeated_reversals_keep_samples_finite() {
    for scale in [f64::MIN_POSITIVE, f64::MAX] {
        let mut engine = engine();
        for step in 0..256 {
            let now = seconds(f64::from(step) * 0.003);
            let to = pose(
                if step % 2 == 0 {
                    2_147_483_648.0
                } else {
                    -2_147_483_648.0
                },
                2_147_483_648.0,
                if step % 2 == 0 { 1.0 } else { 0.0 },
            );
            engine
                .retarget(
                    1,
                    7,
                    AnimationEffect::WindowMovement,
                    pose(0.0, 100.0, 0.0),
                    to,
                    now,
                    scale,
                )
                .unwrap();
            let sample = engine.sample(1, now + seconds(0.002)).unwrap().pose;
            assert!(sample.rect.x.is_finite() && sample.rect.y.is_finite());
            assert!(sample.rect.width.is_finite() && sample.rect.width > 0.0);
            assert!(sample.rect.height.is_finite() && sample.rect.height > 0.0);
            assert!(sample.opacity.is_finite() && (0.0..=1.0).contains(&sample.opacity));
        }
    }
}

#[test]
fn tiny_positive_destination_dimensions_settle_exactly() {
    let mut engine = engine();
    let mut to = pose(1.0, 1e-300, 1.0);
    to.rect.height = f64::from_bits(1);
    engine
        .retarget(
            1,
            7,
            AnimationEffect::WindowMovement,
            pose(0.0, 100.0, 0.0),
            to,
            Duration::ZERO,
            1.0,
        )
        .unwrap();
    assert_eq!(engine.sample(1, seconds(2.0)).unwrap().pose, to);
}

#[test]
fn capped_display_phase_preserves_spring_velocity_across_speed_rebase() {
    let mut engine = engine();
    start(&mut engine, 1, AnimationEffect::WindowMovement);
    let displayed = engine.sample(1, seconds(0.05)).unwrap().pose;
    let displayed_phase = engine.clock.sample(seconds(0.05));
    let expected_velocity =
        (engine.sample(1, seconds(0.050001)).unwrap().pose.rect.x - displayed.rect.x) / 0.000001;
    let mut config = enabled();
    config.speed = 2.0;
    engine.apply_config(config, seconds(0.1)).unwrap();
    engine
        .retarget_presented(
            1,
            7,
            AnimationEffect::WindowMovement,
            displayed,
            pose(-200.0, 100.0, 1.0),
            seconds(0.11),
            1.0,
            displayed_phase,
        )
        .unwrap();
    assert_eq!(engine.sample(1, seconds(0.11)).unwrap().pose, displayed);
    let actual_velocity =
        (engine.sample(1, seconds(0.1100005)).unwrap().pose.rect.x - displayed.rect.x) / 0.000001;
    assert!(
        (expected_velocity - actual_velocity).abs() < 0.3,
        "expected {expected_velocity}, got {actual_velocity}"
    );
    for invalid_phase in [f64::NAN, -1.0, 99.0] {
        assert!(
            engine
                .retarget_presented(
                    1,
                    7,
                    AnimationEffect::WindowMovement,
                    displayed,
                    pose(200.0, 100.0, 1.0),
                    seconds(0.11),
                    1.0,
                    invalid_phase
                )
                .is_err()
        );
    }
}

#[test]
fn unsafe_inherited_overshoot_clamps_geometry_but_ordinary_motion_keeps_velocity() {
    use clear::runtime::animation::MAX_POSE_COMPONENT;
    let mut engine = engine();
    let from = pose(MAX_POSE_COMPONENT - 1000.0, 100.0, 1.0);
    let to = pose(MAX_POSE_COMPONENT - 10.0, 100.0, 1.0);
    engine
        .retarget(
            1,
            7,
            AnimationEffect::WindowMovement,
            from,
            to,
            seconds(0.0),
            1.0,
        )
        .unwrap();
    let shown = engine.sample(1, seconds(0.02)).unwrap().pose;
    let mut config = enabled();
    config.window_movement.kind = AnimationKind::Spring { stiffness: 1.0 };
    engine.apply_config(config, seconds(0.02)).unwrap();
    engine
        .retarget_presented(
            1,
            7,
            AnimationEffect::WindowMovement,
            shown,
            pose(MAX_POSE_COMPONENT - 2000.0, 100.0, 1.0),
            seconds(0.02),
            1.0,
            0.02,
        )
        .unwrap();
    let mut reached_bound = false;
    for step in 1..=100 {
        let sample = engine
            .sample(1, seconds(0.02 + f64::from(step) * 0.01))
            .unwrap()
            .pose;
        assert!(sample.rect.x <= MAX_POSE_COMPONENT);
        if sample.rect.x == MAX_POSE_COMPONENT {
            reached_bound = true;
        }
    }
    assert!(reached_bound);
}
