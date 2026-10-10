# Animation engine and configuration

## Implemented scope

Clear implements a backend-independent animation clock, bounded presentation pose
tracks, a pure snapshot/transition planner, and strict animation TOML preferences.
The planner produces geometry/opacity frames for workspace switching, opening,
closing, reflow, minimize/restore, maximize/restore, and fullscreen/restore.
The platform does not yet consume those frames or render any of the eight visual
effects. Workspace switching, window creation/destruction, tiled reflow, minimize/restore, maximize/restore,
overview, and fullscreen/restore therefore keep their current instant visual
behavior, including when `animations.enabled = true`. Fullscreen desktop policy
is a separate [desktop](desktop.md) contract.

The [implementation plan](../docs/animations-design.md) describes subsequent
remaining scene integration, transformed hit discovery, lifecycle snapshot
capture, effect triggers, and acceptance work. Shared GPU composition and inverse
coordinate delivery are implemented foundations described by
[rendering](rendering.md) and [input](input.md); animation frames do not yet drive
them. Proposed visual behaviors and image budgets are not implemented guarantees.
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
`0..=1`. Physical output scale MUST be finite and positive. Intermediate geometry MUST clamp to the same magnitude
bound; nonpositive spring dimensions clamp to machine epsilon and opacity clamps
to `0..=1`. The affected component's velocity MUST clear at these safety bounds;
ordinary in-range spring motion retains its analytic velocity.
Spring retargeting MUST reject a polynomial/derivative envelope that could
overflow before its settling deadline, retaining the previous track. Tolerance
checks divide their thresholds by physical scale to avoid overflow from extreme
but finite positive scales. Positive endpoint dimensions below machine epsilon
MUST be preserved exactly; only nonpositive intermediate dimensions use the
minimum-dimension clamp.
Final completion MUST return the exact supplied destination.

Retargeting starts from the old track's analytic pose at the supplied timestamp;
initial caller pose is used only for a new ID. A spring-to-spring retarget MUST
preserve analytic velocity. Easing retargets preserve pose without promising
velocity continuity. `retarget_presented` instead starts from an explicitly
acknowledged pose and inherits spring velocity at its captured animation phase.
That phase MUST be finite, nonnegative, and no later than current phase. Capturing
phase alongside the pose MUST preserve velocity across speed rebases even when
the last displayed real timestamp predates the new epoch.

The caller supplies a transition group ID and one common timestamp for related
tracks. The engine exposes group membership; it does not infer causes, coordinate
different parameter laws, or wait for client commits. Related tracks with the
same timestamp MUST share animation phase. Their normalized progress additionally
depends on captured endpoints, initial spring velocity, and completion tolerances.

A spring completes when both rectangle corners are within 0.25 physical pixels
and corner velocities within 1 physical pixel per animation second, with opacity
error/velocity at most 0.001. It MUST settle after at most two animation seconds
regardless of tolerance. At speed 0.1 this bounds a spring to twenty real seconds.
Timed easing MUST settle by its duration. Global disable, reduced motion, effect
disable, or explicit `settle` MUST expose the exact final pose immediately.
Completed tracks remain retained until `remove`; no resource retirement is implied
by a pure sample.

## Pure presentation planning

`PresentationPlanner` consumes complete backend-neutral snapshots of normal
windows, including hidden/minimized windows, live/mapped generations, state flags,
home outputs/workspaces, desired visible placements/clips, and optional minimize
anchors. Outputs provide full bounds, physical scale, and optional mapped-panel
fallback bounds. Groups describe current workspace ownership; the planner MUST
NOT change that ownership, focus, layout geometry, or any desktop state.

Snapshots MUST have one record per window ID, unique mapped generation keys,
unique output/group IDs, and unique presented workspace ownership across groups.
Groups MUST cover connected outputs exactly once. All supplied rectangles MUST
be nonempty and normalized without integer coordinate overflow. Mapped windows
MUST be live; supplied visible placements MUST belong to mapped, live,
non-minimized windows on their home output's presented workspace. Hidden records
may retain other workspace ownership. Invalid snapshots/directions MUST fail
before changing planner state.

Reconciliation MUST classify disappearance/unmapping as closing, minimization as
minimize, restoration as its reverse, state-flag changes as maximize/fullscreen,
and other visible rectangle changes as movement. An ordinary hidden-workspace
record MUST NOT produce a closing image. A workspace presentation change produces
outgoing/incoming workspace slides; ambiguous operations MAY provide explicit
per-window or per-group causes. A close hint alone MUST NOT remove a still-live
visible window. Direct manipulation and an explicit settle/reset MUST suppress
motion and expose final policy placements immediately.

Opening starts transparent and scaled around its destination center by the
configured open scale. Closing ends transparent and enlarged by close scale.
Minimize moves toward an output-local icon rectangle when supplied, otherwise
the mapped panel's center, otherwise the home output's bottom center. Targets
preserve source aspect ratio, shrink without enlarging, and fade to zero. Anchors
are intersected with full output bounds; stale off-output anchors fall back.
The bottom-center fallback MUST use the exact logical center, including odd
output widths, and its nominal 16-by-16 rectangle MUST shrink to fit tiny outputs.
Restore reverses from an interrupted outgoing pose and captured spring velocity,
or the anchor after the old minimize's terminal frame has been acknowledged.
Reflow/maximize/fullscreen use captured unrounded rectangles and their
corresponding configured laws.

Workspace motion defaults to horizontal direction according to workspace ID
ordering when no explicit direction is supplied. An explicit direction vector
MUST be finite, nonzero, and within `-1..=1` on each axis; the pure API also accepts
vertical vectors for future adapter/layout work. Distances come from the
affected group's output union. Frames expose incoming/outgoing offsets and a
progress descriptor for future wallpaper composition. Incoming and outgoing
scene offsets use separate tracks with a shared timestamp and motion law. A
reversal MUST reuse the matching workspace scene's last acknowledged offset and
captured spring velocity, including across a speed rebase. Progress calculation
MUST remain finite for subnormal nonzero direction vectors. Superseding a
workspace slide releases previous outgoing records rather than chaining scenes;
reappearing window tracks retain captured pose and spring velocity.
Workspace ownership stays at its already reconciled policy value throughout.

Frames MUST contain every final live policy placement, even when the shared
engine's 256-track budget is exhausted. New failed/budget-limited tracks settle
instantly. Existing tracks may retarget at the limit. Workspace motion MUST
settle the whole effect when both group tracks cannot be allocated, including
window tracks that otherwise could retarget at the limit. Outgoing records are
bounded by that same engine budget and MUST never receive input; incoming
workspace tracks defer input eligibility until they settle. Other live records
retain eligibility for the adapter's transformed hit testing. Frozen source
ownership is represented only by opaque retained IDs and mapping-generation keys.
Retention requests MUST pin an already owned prior committed image; adapters
MUST NOT access a destroyed/unmapped surface to fulfill them. GPU allocation,
image budgets, decoration/client commit synchronization, popup ownership, actual
stack composition, and overview card/wallpaper motion are subsequent adapter work.

Sampling MUST be immutable, use one common timestamp/animation phase, and perform
no commands, scripts, capture, IO, or resource releases. `mark_presented` records
only visuals actually drawn by the adapter; callers MUST remove failed/unavailable
draws from the acknowledged frame. It MUST reject unknown/stale/duplicate source
identities, stale/duplicate groups, poses/offsets outside the engine's bounded
geometry domain, and backwards common or per-window/group timestamp/phase
values without changing state. Sampled frames carry opaque per-track revision
and terminal-status metadata; callers MUST preserve it when filtering unavailable
draws. Missing live draws MUST NOT acknowledge their track's completion. Whether
a submitted frame was physically presented remains the adapter's timing-source limitation, not a planner guarantee.

Interrupted tracks MUST restart from the last acknowledged pose and captured
phase, preserving critical-spring velocity. Analytic completion alone MUST NOT
discard the last drawn pose, workspace offsets, or spring track before the
terminal pose/scene absence is acknowledged. Thus a reversal after a skipped or
capped final frame still starts from the actual shown phase.
`is_animating` MUST remain true while a terminal frame still needs acknowledgement.
Retargeting or explicit/config settlement changes the track revision: an old
mid-motion acknowledgement MUST NOT retroactively count as terminal after
disable or reduced-motion reload. Once the current revision's terminal frame is
acknowledged, retirement releases completed tracks, discards completed hidden
poses so later restores start at the anchor, and returns retained IDs to the adapter;
group identity repair, output loss, direct manipulation, and reset cancel stale
records. Global/effect disable and reduced motion use exact engine settlement;
re-enabling with an unchanged snapshot MUST NOT replay old operations. The
planner avoids other callers' engine IDs and clears only its own tracks.

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
- [Presentation planner](../src/runtime/presentation.rs) and
  [presentation tests](../tests/presentation.rs) cover snapshot validation,
  effect classification, mapped generations, retained-source requests, interrupted
  motion and velocity across speed rebases, workspace offset continuity, tiny
  direction/output safety, whole-workspace budget fallback, and terminal
  acknowledgement/cleanup including missed frames and configuration settlement.
- [Animation tests](../tests/animations.rs) cover strict defaults/tags/ranges,
  skipped sampling, speed rebasing, captured parameters, spring interruption,
  group progress, bounded completion/tracks, unsafe overshoot clamping, instant
  settlement, and atomic reload.
  These are CPU contracts, not evidence of GPU effects, physical input, refresh-rate
  presentation, or double-buffer operation.
