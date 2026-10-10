# Animations and presentation

## Implemented scope

Clear implements strict animation preferences, a monotonic analytical engine,
a pure transition planner, and visible nested-backend effects when
`animations.enabled = true`. Workspace switching slides windows and wallpaper;
creation fades/scales in; actual unmap/destruction reverses creation; layout
movement, minimize/restore, maximize/restore and fullscreen/restore transform
owned committed images. [Overview](overview.md) defines its card/wallpaper
transition and input ownership. Disabled or reduced-motion preferences retain
instant behavior. Fullscreen policy remains a separate [desktop](desktop.md)
contract; animation never changes requested placement or ownership.

The [implementation plan](../docs/animations-design.md) retains future work and
acceptance gates. Native DRM/KMS, presentation-feedback timing, vertical workspace
layout policy, and arbitrary script animation laws remain unsupported. The pure
workspace direction API can represent vertical motion, but current desktop
workspace navigation uses horizontal slides. Global animations remain disabled
by default while acceptance coverage is expanded.

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
of kind. The planner applies these scales about the window frame center; the engine's
rectangle target is supplied explicitly by its caller.

The frame-rate preference controls selection of animation sampling opportunities;
it MUST NOT publish a different display mode. The current backend's timing source,
nominal fallback, and synchronization limits are described in
[platform timing](platform.md#nested-frame-timing). One capped animation timestamp
MUST drive window, workspace and overview sampling. Caps hold presentation poses
between eligible host opportunities while client callbacks continue at host
cadence. Late frames MUST NOT change the configured cap or clamp cadence to a
smaller integer divisor; synchronization uses the host EGL swap interval.

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
image budgets, decoration/client commit synchronization, popup transforms and
stack composition belong to the adapter contract below. The pure planner itself
does not implement overview card/wallpaper motion.

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

## Visible window and workspace effects

The nested adapter MUST build the planner snapshot only after policy placements,
output bounds and advisory shell targets are reconciled. Normal windows are keyed
by a mapping generation which changes on every new buffered mapping. Launchers,
layers and their popups retain their existing independent paths. Closing begins
only at actual unmap/destruction; sending a close request alone is not a visual
trigger. Hidden workspace records MUST remain distinct from destroyed clients.

With motion enabled, the adapter prepares live normal-window images from
committed body/subsurface/SSD data using the shared
[image composer](rendering.md#committed-window-images), and remembers only images
whose source was included in a successfully submitted scene. A retention request
MUST pin that prior owned image before pruning a dead/unmapped generation. It
MUST never render from, import, or dereference a dead surface to fulfill closing
or workspace retention. A never-drawn window without a cached source settles
instantly. Retained outgoing images never receive input.

The animation cache MUST retain at most 128 distinct images and 64 MiB of RGBA8
texture storage, including current, last-submitted, retained and pending-source
references. Shared references count once. Individual sources preserve aspect
ratio while downscaling to at most 2048 pixels per edge. Composer scratch and
driver/transient allocation overhead are described separately in
[rendering](rendering.md#committed-window-images). Cache pressure may evict prior
unneeded source images; failure to prepare/admit a source MUST cancel its visual
track and use the ordinary instant path. A later destruction without that image
also settles instantly. Textures still owned by a submitted/retained image MUST
NOT be overwritten or reused as render targets.

The complete original border/rounded silhouette MUST scale relative to the
committed inner frame into the sampled `f64` frame. Output-group clips are applied
after that transform; panel reservations MUST NOT clip a minimize destination.
Live content commits update moving sources, preserving premultiplied alpha.
Fullscreen decoration suppression MUST follow the buffered root commit, never an
ACK or policy request alone. Once geometry motion finishes, a transformed source
MUST remain fitted to the final requested frame while the committed frame size or
fullscreen appearance still differs. A corresponding committed frame within one
logical pixel of the target permits return to ordinary rendering. There is no
mandatory old/new-buffer crossfade; source appearance changes with actual commits.

Workspace translations MUST move windows and per-output wallpaper crops by the
same group offsets while panels/layers remain stationary. A stretched group's
source wallpaper crops remain per-output images and may cross member output clips;
independent groups MUST remain clipped separately. Horizontal direction follows
workspace IDs unless the planner receives an explicit cause. Outgoing source
records and group offsets preserve the last submitted pose on reversal.

Scene composition MUST record only successfully included sources and workspace
metadata; frame acknowledgement MUST occur after successful backend submission.
A failed draw MUST NOT advance that source's acknowledged pose. The acknowledged
image, committed geometry origin, SSD inset/style and silhouette MUST drive inverse
input discovery and popup transforms, as specified by [input](input.md) and
[platform](platform.md#scene-and-hit-testing). Incoming workspace scenes remain
noninteractive until settlement; the pending policy switch also blocks application
input before its first animated draw. Paired application presses during workspace
motion are suppressed together. A successful frame refreshes stationary-pointer
focus without changing desktop focus.

Direct move/resize, an active pointer grab or paired-press owner, overview ownership,
renderer/topology reset and theme replacement MUST settle ordinary window/workspace
motion rather than letting policy/input geometry compete with a retained image.
Overview owns its card scene throughout opening/closing. Global disable and reduced
motion release the ordinary animation cache and render final policy state on the
next host redraw. Re-enabling an unchanged policy snapshot MUST NOT replay old
operations. The backend continues requesting host redraws for client/layer content
and exposure; animation completion does not suppress a required terminal redraw.

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
  These are CPU contracts, not evidence of GPU effects, physical input or
  presentation-feedback timing.
- [Nested bridge](../src/platform/smithay/animations.rs),
  [scene/input integration](../src/platform/smithay/scene.rs), and
  [backend submission](../src/platform/smithay/backend.rs) implement visible
  effects; bridge unit tests cover bounded image admission and original-frame
  scaling/committed geometry gates. GPU validation records belong in
  [VM testing](../docs/vm-testing.md).
- [Bounded animation GPU fixture](../scripts/vm-animation-smoke.py) exercises all
  eight effects plus restore/overview exit, fading backdrop coverage and held
  fullscreen commits. This verifies captured images and policy/configures, not
  delivered cadence or physical input.
