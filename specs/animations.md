# Animation engine and configuration

## Implemented scope

Clear implements a backend-independent animation clock, bounded presentation pose
tracks, and strict animation TOML preferences. The platform does not yet create
these tracks or render any of the eight visual effects. Workspace switching,
window creation/destruction, tiled reflow, minimize/restore, maximize/restore,
overview, and fullscreen/restore therefore keep their current instant visual
behavior, including when `animations.enabled = true`. Fullscreen desktop policy
is a separate [desktop](desktop.md) contract.

The [implementation plan](../docs/animations-design.md) describes subsequent
GPU composition, input transforms, snapshots, effect triggers, and acceptance
work. Those proposed behaviors and image budgets are not implemented guarantees.
Animation defaults MUST remain off until the visual/input acceptance gates pass.

## Configuration

`[animations]` and its eight effect tables MUST reject unknown fields and malformed
typed values. Omitted fields/tables MUST retain defaults. Disabled preferences
MUST still be validated.

| Global field | Default | Accepted values |
| --- | --- | --- |
| `enabled` | `false` | Boolean; opts the engine into motion |
| `reduced_motion` | `false` | Boolean; settles transitions instantly |
| `speed` | `1.0` | Finite number in `0.1..=10`; higher is faster |
| `frame_rate` | `"refresh-rate"` | This exact string or integer `30`, `60`, `120` |

| Effect table | Default kind | Parameters |
| --- | --- | --- |
| `workspace_switch` | `spring` | `stiffness = 1000.0` |
| `window_open` | `easing` | `duration_ms = 150`, `curve = "ease-out-expo"`, `scale = 1.04` |
| `window_close` | `easing` | `duration_ms = 150`, `curve = "ease-out-quad"`, `scale = 1.04` |
| `window_movement` | `spring` | `stiffness = 800.0` |
| `minimize` | `easing` | `duration_ms = 250`, `curve = "ease-out-cubic"` |
| `maximize` | `spring` | `stiffness = 800.0` |
| `overview` | `spring` | `stiffness = 800.0` |
| `fullscreen` | `spring` | `stiffness = 800.0` |

Each effect accepts `enabled` (default `true`) and `kind = "easing" | "spring"`.
Naming the default kind or omitting kind inherits that effect's default parameters.
Changing kind uses default easing `250` ms/`ease-out-cubic` or spring stiffness
`800.0`, then applies explicitly supplied parameters.

Easing accepts integer `duration_ms` in `0..=2000` and `curve = "linear" |
"ease-out-quad" | "ease-out-cubic" | "ease-out-expo"`; duration zero is instant.
Spring accepts finite `stiffness` in `1..=10000` and is always critically damped.
Fields belonging to the other kind MUST be rejected even when disabled. Only
`window_open` and `window_close` accept `scale`, finite in `1..=1.25`, independent
of kind. These scales are preferences for future visual effects; the engine's
rectangle target is supplied explicitly by its caller.

The frame-rate preference controls selection of animation sampling opportunities;
it MUST NOT publish a different display mode. The current backend's timing source,
nominal fallback, and synchronization limits are described in
[platform timing](platform.md#nested-frame-timing). Visual tracks are not yet
sampled by that backend.

## Clock and sampling

`AnimationClock` MUST map injected monotonic timestamps to animation seconds using
`animation_epoch + (real_time - real_epoch) * speed`. Sampling MUST be pure and
MUST NOT advance a shared clock; different outputs may predict different future
timestamps without changing each other's phase. Predictions before the current
rebase epoch clamp to that epoch. This API samples current/future presentation,
not historical poses across previous speed epochs.

A speed change MUST rebase the current phase before replacing speed. A rebase
before the previous epoch or an invalid speed MUST fail without changing the
clock. Runtime converts absolute `Instant` timestamps into its own elapsed time
domain with `animation_time_at`; adapter-relative timestamps MUST be converted
through their corresponding absolute origin before engine sampling.

Easing and critical springs MUST be evaluated analytically from elapsed time,
not accumulated simulation steps. Sampling at 30/60/120/refresh-rate or skipping
frames MUST produce the same pose at the same animation time. Critical springs
use `omega = sqrt(stiffness)` and
`target + (c1 + c2*t)*exp(-omega*t)`, with analytic velocity for interruption.
The expo curve is normalized to reach its destination continuously at duration.

## Tracks, interruption, and completion

`AnimationEngine` retains at most 256 ID-keyed tracks, including completed tracks
awaiting explicit retirement. Retargeting an existing ID MUST replace its track,
never append a queued transition. Allocation of a new ID when full MUST return an
error; the presentation caller must settle that effect instantly. The engine
allocates no GPU images and does not own compositor surfaces or desktop state.

A pose stores `f64` logical origin/dimensions and opacity. Supplied values MUST be
finite, geometry components within magnitude 2147483648 (covering core's i32
range without overflow in spring algebra), dimensions positive, and opacity in
`0..=1`. Physical output scale MUST be
finite and positive. Intermediate spring dimensions MUST remain positive and
opacity MUST clamp to `0..=1`; unsafe overshoot velocity at a clamp is cleared.
Spring retargeting MUST reject a polynomial/derivative envelope that could
overflow before its settling deadline, retaining the previous track. Tolerance
checks divide their thresholds by physical scale to avoid overflow from extreme
but finite positive scales. Positive endpoint dimensions below machine epsilon
MUST be preserved exactly; only nonpositive intermediate dimensions are clamped.
Final completion MUST return the exact supplied destination.

Retargeting starts from the old track's analytic pose at the supplied timestamp;
initial caller pose is used only for a new ID. A spring-to-spring retarget MUST
preserve analytic velocity. Easing retargets preserve pose without promising
velocity continuity. Future presentation integration must choose its retarget
timestamp consistently with the pose last shown when capped/skipped frames matter.

The caller supplies a transition group ID and one common timestamp for related
tracks. The engine exposes group membership; it does not infer causes, coordinate
different parameter laws, or wait for client commits. Related tracks with the
same law/timestamp MUST share progress.

A spring completes when both rectangle corners are within 0.25 physical pixels
and corner velocities within 1 physical pixel per animation second, with opacity
error/velocity at most 0.001. It MUST settle after at most two animation seconds
regardless of tolerance. At speed 0.1 this bounds a spring to twenty real seconds.
Timed easing MUST settle by its duration. Global disable, reduced motion, effect
disable, or explicit `settle` MUST expose the exact final pose immediately.
Completed tracks remain retained until `remove`; no resource retirement is implied
by a pure sample.

## Reload

The [configuration reload](configuration.md#reload) preparation contract applies.
Animation validation/resource failure MUST retain previous configuration and
clock/track behavior. Successful reload MUST rebase speed after candidate resources
are prepared. Active tracks retain their captured curve, duration, and stiffness;
new tracks capture the new preferences. Disabling global motion or an individual
effect, or enabling reduced motion, settles affected tracks. Re-enabling MUST NOT
replay previously settled operations. Changing frame-rate preferences MUST NOT
reset engine phase or pose.

## Implementation and evidence

- [Schema/defaults](../src/config/animations.rs), [config integration](../src/config/mod.rs).
- [Pure clock and tracks](../src/runtime/animation.rs),
  [runtime time conversion and reload](../src/runtime/mod.rs).
- [Animation tests](../tests/animations.rs) cover strict defaults/tags/ranges,
  skipped sampling, speed rebasing, captured parameters, spring interruption,
  group progress, bounded completion/tracks, instant settlement, and atomic reload.
  These are CPU contracts, not evidence of GPU effects, physical input, refresh-rate
  presentation, or double-buffer operation.
