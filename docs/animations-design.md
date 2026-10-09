# Compositor animations — implementation plan

**Status: foundations implemented; visual effects remain proposed.** The
[animation specification](../specs/animations.md) covers the strict settings,
clock and bounded analytic pose engine; [platform timing](../specs/platform.md#nested-frame-timing)
covers nested sampling and interval-1 swaps. Instant [fullscreen](../specs/desktop.md#fullscreen)
policy and XDG integration are implemented. GPU transforms, snapshots, effect
triggers, panel-icon hints and animated overview remain pending. The settings stay
off by default, and enabling them does not yet produce visual motion.

The [roadmap](../ROADMAP.md) tracks this work; the [specifications](../specs/README.md)
describe current behavior. This plan retains the full target architecture and
delivery sequence below; proposed API sketches are not current support claims.

## Intended result

Animations are compositor presentation state layered over authoritative desktop
state. A command applies its final policy once; rendering moves smoothly from
the currently displayed appearance to that result. Animation frames do not
repeatedly mutate layouts, saved floating rectangles, focus, or workspace
ownership, and do not send a client a new size configure every frame.

By default, animation samples target the current presentation refresh rate.
Users can select 30, 60, 120, or refresh-rate sampling and adjust overall speed.
Rendering uses a synchronized front/back presentation path. Missing a deadline
skips that presentation opportunity; it does not select a permanent half-rate
mode, slow animation time, or enqueue a backlog of obsolete frames.

All eight effects use the same clock, geometry transforms, damage rules, and
interruption model. Disabling motion settles to the same policy and visual
result without retaining animation textures or changing input authorization.

## Baseline and prerequisites

This table records the baseline when planning began; foundation progress follows
below it.

| Existing component | What is available | Required change |
| --- | --- | --- |
| [Core types](../src/core/types.rs), [desktop policy](../src/core/desktop.rs) | Stable IDs, desired placements, output groups, saved floating geometry, independent maximize/minimize flags | Keep animation geometry out of core; add fullscreen policy separately |
| [Runtime](../src/runtime/mod.rs) | Typed action routing, atomic configuration reload, built-in/Rhai placement computation | Add animation settings and explicit transition causes; keep scripts out of per-frame sampling |
| [Reconciliation and scene](../src/platform/smithay/scene.rs) | Desired placements, requested versus committed content, common output clips, SSD, rounded composition, blur/glass grouping | Separate policy reconciliation from sampled visual geometry; make rendering and input consume one presentation description |
| [XDG lifecycle](../src/platform/smithay/protocols.rs) | First-buffer mapping, null-buffer unmapping, destruction, maximize/minimize requests | Capture usable content before lifecycle cleanup; add fullscreen request/state handling |
| [Nested backend](../src/platform/smithay/backend.rs), [event loop](../src/platform/smithay/mod.rs) | EGL presentation, damage tracker, continuous redraw, a 16 ms event-loop polling timeout | Add host-aware frame scheduling, synchronized swaps, explicit wakeups and capture/deadline timers |
| [Overview renderer](../src/platform/smithay/overview.rs), [session](../src/runtime/overview.rs) | Live committed GPU cards, layout/hits, wallpaper canvas, bounded caches, instant entry/exit | Extract reusable window-image composition; add transition phases and continuous card/wallpaper geometry |
| [Shell model](../src/shell/mod.rs), [transport](../src/shell/server.rs), [adapter](../src/platform/smithay/shell.rs) | Optional bounded IPC, maximize/minimize commands, policy snapshots | Add optional, validated panel/icon geometry hints; no current request supplies them |
| [Quickshell dock](../examples/quickshell/AppDock.qml) | App groups, icon delegates, panel-local positions | Report actual visible icon bounds after layout and scrolling |

Foundation progress and remaining gaps:

- Virtual outputs now share the host monitor's advertised refresh metadata, or a
  reported nominal 60000 mHz fallback. The nested API does not expose actual
  presentation feedback or independent physical refresh for virtual outputs.
- Explicit GL attributes and a checked interval-1 EGL swap request are implemented.
  Physical synchronization still needs host/presentation measurement.
- The clock, bounded tracks, strict configuration and atomic reload are implemented;
  scene construction does not yet create or sample visual tracks.
- `dirty` controls reconciliation, not all rendering damage. Continuous repaint
  remains in place; explicit damage/idle wakeups and capture deadlines are pending.
- `SurfaceHit` describes an origin and cannot represent scaled client input.
  Reusable window-image composition and shared visual/input transforms are pending.
- Instant fullscreen now has core policy, separate full output bounds, XDG states,
  pre-map/output requests, committed decoration handling and configurable actions.
  Its animated transition remains pending.

The pinned backend reference is
[Smithay winit source](https://github.com/Smithay/smithay/blob/e1fb2496c9d7ec4d994cc50fe61b7439686ba8b9/src/backend/winit/mod.rs).
Native DRM/KMS remains separate roadmap work; do not make it a prerequisite for
the initial nested animation implementation.

## Niri reference and Clear's choices

Reference review used niri commit
[`2c82b77a7116a1241ea2fd41dd008ff00b609aae`](https://github.com/niri-wm/niri/tree/2c82b77a7116a1241ea2fd41dd008ff00b609aae).
These are architectural references, not a proposal to import niri's layout model.

- **Stable time within a frame.** Niri predicts presentation time, freezes its
  animation clock for rendering, and uses an adjustable clock for speed changes.
  Clear should similarly pass one immutable animation-time sample through scene,
  wallpaper, decorations, and overview. See
  [timing rationale](https://github.com/niri-wm/niri/wiki/Animation-Timing),
  [clock implementation](https://github.com/niri-wm/niri/blob/2c82b77a7116a1241ea2fd41dd008ff00b609aae/src/animation/clock.rs), and
  [redraw sampling](https://github.com/niri-wm/niri/blob/2c82b77a7116a1241ea2fd41dd008ff00b609aae/src/niri.rs).
- **Easing and springs serve different effects.** Use timed easing for opacity
  and opening/closing; critically damped springs for retargetable geometry.
  Niri exposes both, plus a global slowdown control. Clear will expose a speed
  multiplier with the explicit convention that larger values are faster. See
  [animation implementation](https://github.com/niri-wm/niri/blob/2c82b77a7116a1241ea2fd41dd008ff00b609aae/src/animation/mod.rs) and
  [configuration reference](https://github.com/niri-wm/niri/blob/2c82b77a7116a1241ea2fd41dd008ff00b609aae/docs/wiki/Configuration%3A-Animations.md).
- **Preserve images across asynchronous changes.** Niri retains resize snapshots
  and separate closing-window textures. Clear needs equivalent ownership without
  adopting global client transactions. See
  [tile resize/unmap snapshots](https://github.com/niri-wm/niri/blob/2c82b77a7116a1241ea2fd41dd008ff00b609aae/src/layout/tile.rs) and
  [closing-window rendering](https://github.com/niri-wm/niri/blob/2c82b77a7116a1241ea2fd41dd008ff00b609aae/src/layout/closing_window.rs).
- **Separate presentation prediction from animation math.** Niri's
  [frame clock](https://github.com/niri-wm/niri/blob/2c82b77a7116a1241ea2fd41dd008ff00b609aae/src/frame_clock.rs)
  predicts the next opportunity; its
  [native backend](https://github.com/niri-wm/niri/blob/2c82b77a7116a1241ea2fd41dd008ff00b609aae/src/backend/tty.rs)
  feeds presentation events back into scheduling. Clear should have the same
  boundary so a later native backend can replace the host timing source.
- **Do not assume nested timing is solved upstream.** Niri's
  [winit renderer](https://github.com/niri-wm/niri/blob/2c82b77a7116a1241ea2fd41dd008ff00b609aae/src/backend/winit.rs)
  reports unknown refresh in presentation feedback and documents an immediate
  redraw limitation. Clear's scheduler must be validated against its own pinned
  Smithay and host behavior.

Custom animation shaders, spring bounce, touchpad-driven progress, and native
VRR are follow-up work. The initial implementation should provide reusable
mechanics and the requested effects before expanding those interfaces.

## Ownership and data flow

Prefer small modules with explicit inputs:

| Proposed location | Responsibility |
| --- | --- |
| `src/config/animations.rs` | Strict TOML settings, defaults, validation; no renderer types |
| `src/runtime/animation.rs` | Pure clock mapping, easing/critical-spring sampling, rectangle interpolation, transition causes and group IDs |
| `src/platform/smithay/animations.rs` | ID-keyed visual tracks, outgoing scene records, lifecycle integration, completion and resource cleanup |
| `src/platform/smithay/frame_scheduler.rs` | Host/native timing interface, deadlines, pending-frame state, animation sample eligibility |
| `src/platform/smithay/window_image.rs` | Reusable GPU composition of committed content, SSD, borders and original masks; owned snapshots and live sources |
| Existing `scene.rs`, `input.rs`, `overview.rs`, `wallpaper.rs` | Consume the immutable presentation frame; retain their scene/input/effect responsibilities |
| Existing shell modules | Backend-independent hint schema and bounded connection ownership; adapter resolves mapped panel identities |

Keep all Smithay, Wayland, EGL, texture and fence types in the adapter. Core owns
only final desktop state. Runtime selection remains ID-only. No independent
animation workspace/window model should compete with `Desktop`.

Suggested plain data types are `TransitionCause`, `TransitionId`,
`AnimationTime`, `VisualRect` with floating-point coordinates, and sampled
`VisualTransform`/opacity. Adapter-only types add live surface sources, owned
textures, output masks and stack positions. A `PresentationFrame` contains the
time sample, visual windows/workspace scenes, overview geometry and input maps.

An operation proceeds as follows:

1. At an authorized action or client lifecycle boundary, retain the previous
   presentation and record its cause. Coalesce related policy changes into one
   transition group, including neighbor reflow.
2. Apply the ordinary desktop command once. Reconcile final placements and send
   final client size/state requests using existing asynchronous semantics.
3. Compare old and new placements/state by ID. Create or retarget tracks using
   their currently displayed geometry rather than stale original endpoints.
4. At a scheduled presentation, sample every participating track at one time,
   prepare required committed GPU images, and build the immutable scene/input
   description. Do not run a script or policy command from this step.
5. Submit, record which presentation description corresponds to that frame, and
   retire completed tracks/resources when safe. Schedule again only for damage,
   callbacks or unfinished motion.

Placement diffing alone cannot distinguish minimize, destruction, a hidden
workspace, or overview activation. Carry explicit causes through keyboard,
titlebar, XDG, IPC and declarative Rhai paths. Do not put animation side effects
inside each built-in layout; script layouts get the same placement diff handling.

### Time, interruption and completion

- Use monotonic time and closed-form sampling. Timed easing is evaluated from
  elapsed time; critically damped springs use an analytic solution, not one
  simulation step per rendered frame. A dropped frame therefore changes the
  next sample, not the motion's duration or stability.
  For a unit-mass critical spring, use `omega = sqrt(stiffness)` and
  `x(t) = target + (c1 + c2*t)*exp(-omega*t)`, where
  `c1 = start - target` and `c2 = initial_velocity + omega*c1`. Sample its
  analytic derivative for velocity-preserving retargeting.
- Define animation time as `a(t) = a0 + (t - t0) * speed`. Rebase `(a0, t0)` at
  reload before changing speed. Sampling should be a pure function of the epoch
  and target timestamp, avoiding a mutable shared clock being advanced/backed up
  by different outputs' predicted times in a future native backend.
- Repeated maximize/restore, reorder, workspace switch and overview toggles
  retarget from the last displayed pose. Preserve velocity for spring tracks.
  Easing tracks must preserve position/opacity continuity; they need not preserve
  velocity on reversal in the initial version.
- Related geometry uses one transition group and progress law. For example,
  maximizing a tile and moving its neighbors must not produce unrelated timings.
  Delayed client buffers may join the ongoing visual transition without delaying
  all other windows.
- Clamp opacity to `0..=1`, require positive dimensions/scales, and settle exactly
  to the final rectangle. Zero-velocity critical springs avoid overshoot; define
  completion in physical pixels plus velocity, with a bounded maximum settling
  time. Proposed thresholds are at most 0.25 physical pixels of corner error, at most one
  physical pixel/second of corner velocity, and at most 0.001 opacity error.
  Retargeting can inherit a velocity away from the new endpoint; clamp any unsafe
  opacity/size result and do not promise strictly monotonic reversal in that case.
- Manual pointer move/resize and overview drag ghosts follow the pointer directly.
  Before starting a grab, settle that window's geometry track and re-evaluate its
  hit under the pointer. Animate any resulting neighbor reflow, not the dragged
  window's pointer tracking.
- With motion disabled or reduced motion enabled, finish through the ordinary
  completion/cleanup path. Avoid separate lifecycle logic for instant effects.
  Host suspension/occlusion should advance wall time without rendering; on resume,
  expired effects settle rather than replaying missed frames.

## Frame pacing, buffering and synchronization

### Refresh selection

`frame_rate = "refresh-rate"` is the default. Explicit numeric values are exactly
30, 60 or 120. The effective target is the configured cap bounded by the actual
presentation rate; asking for 120 on a 60 Hz presentation path yields up to 60
distinct presented animation samples. Speed is independent of sampling rate.

For nested winit:

1. Enable synchronized EGL presentation with explicit `GlAttributes` and verify
   the backend's host redraw/frame-callback behavior on Wayland and X11.
2. Obtain the host window's current monitor refresh metadata where the pinned
   winit API exposes it. Update on host monitor/mode changes, not just window
   resizing. Prefer actual host presentation timing when available.
3. Use one scheduler for the host framebuffer. All virtual outputs share its
   clock and effective rate. Reflect a reliable host rate in their advertised
   modes; do not set `wl_output` refresh to an animation cap.
4. If metadata/presentation feedback is unavailable, use a clearly reported
   nominal 60 Hz estimate while retaining host synchronization. Report the
   timing source and uncertainty. Do not infer a 30 Hz display merely because a
   slow renderer delivered 30 frames in a measurement window.

Stage 1 below is a feasibility gate: verify which host readiness and presentation
signals the pinned APIs actually provide. If adequate timing is inaccessible,
add a narrow backend timing hook or propose a separately reviewed dependency
change; do not silently claim refresh accuracy from a free-running timer.

For a future native backend, each output supplies its actual mode and presentation
feedback to the scheduler. Shared transition start time is global, but outputs
sample it at their individual presentation timestamps; a 60 Hz output must not
throttle a 144 Hz neighbor.

### Scheduling rules

- Maintain `Idle`, `Scheduled`, and `FramePending` states with a coalesced redraw
  flag. Allow at most one outstanding submission in Clear's presentation policy;
  pending input/client damage replaces future work rather than queuing frames.
- Predict the next presentation opportunity from host readiness/feedback and
  refresh information. Render with enough measured CPU/GPU preparation lead time,
  bounded to avoid excessive latency. Preparation estimates adjust wakeup timing,
  not the requested refresh rate.
- Sample at the predicted presentation timestamp. Rebase prediction after missed
  or changed presentation events. On a miss, advance to the next available
  opportunity and sample the correct elapsed time. Never replay all skipped
  intermediate samples.
- Fixed caps use absolute time deadlines/phase accumulation. Select opportunities
  nearest those deadlines, with a retained phase error, rather than
  `every ceil(display_hz / cap)` refreshes. For example, 60 on 144 Hz needs a mix
  of two/three refresh intervals; 120 on 144 needs a mix of one/two. Avoid drift
  and systematic rounding down to 48 or 72 fps.
- Do not implement automatic `60 -> 30`, `120 -> 60`, or another integer-divisor
  degradation policy. Do not request swap interval 2+ in response to lateness.
  A measured 59 fps on 60 Hz is acceptable and does not alter the target. Strict
  synchronized presentation can still miss individual refreshes; actual delivered
  fps depends on the host/driver. The requirement is to avoid a policy clamp,
  not to promise that every rendering workload can physically present at 59 fps.
- Animation caps affect compositor motion sampling, not client frame callbacks
  or unrelated client/cursor repaint rate. If a client needs a frame between
  capped samples, redraw its latest content at the previous animation pose.
  At the next eligible animation sample, all linked effects advance together.
- Keep normal client callback pacing tied to presentation opportunities, including
  an opportunity for a callback-only/no-damage frame; an idle scene must not
  deadlock a callback-driven client or spin on empty-damage commits. Deduplicate
  callbacks per surface/opportunity. Retain the overview's separate 33 ms
  preview-client throttle while moving its cards at the animation rate.
- Distinguish reconciliation, client damage, visual motion, exposure and callback
  work. When all are idle, stop animation wakeups. Replace the fixed 16 ms poll
  as the animation driver, while preserving bounded shell servicing, child
  reaping, popup cleanup, `--exit-after` and scheduled captures through explicit
  event sources/timers. A capture deadline must still wake a static scene.

### Front/back and GPU resource discipline

Compose a complete frame into the back buffer and swap only through the
synchronized backend; never update the presented front buffer. Avoid software
frame queues and CPU readback in the animation path. Backend EGL/host allocation
can use internal additional buffers: winit cannot guarantee an exact physical
buffer count, so the nested guarantee is front/back semantics and bounded Clear
submissions. An exact two-buffer native swapchain is a later backend decision.

Use GPU synchronization for imported client buffers, offscreen composition,
sampling and buffer reuse. Same-context ordering may provide sufficient ordering
for some texture operations; audit each producer/consumer path instead of adding
`glFinish` or a CPU fence wait to every frame. Never overwrite a retained close
snapshot or a texture still sampled by an outstanding frame. Restore the window
framebuffer after snapshot/blur/capture work before presentation, preserving the
existing EGL restoration behavior.

Initially full damage is acceptable during effects and the existing blur path.
Track union of old/new transformed bounds, including outlines, later using real
buffer age where accessible. Always damage the final settled frame and old ghost
footprint. Idle behavior and correctness take priority over a premature partial
damage optimization.

## Rendering, asynchronous clients and input

### Reusable visual source

Extract the overview's committed-window composition into `window_image.rs`.
Compose toplevel content and subsurfaces, committed XDG geometry offset, SSD,
border and original rounded mask as one premultiplied image. Translation-only
movement should use live render elements without an offscreen copy where possible;
scaling/resize/close/overview can use the shared GPU image path.

Transform the complete original silhouette. Scale corner radii, borders and SSD
controls consistently; an output crop must not create new corners. Multiply
premultiplied RGB and alpha by animation opacity together. Transparent holes stay
transparent. Maintain output-local filtering and blur/glass stacking: filter the
current backdrop with the transformed silhouette, then composite animated content.
Do not bake other windows or wallpaper into a translucent window snapshot, and
do not rerun optical effects recursively inside overview cards.

Popups are excluded from open/close/minimize images and overview cards. For live
movement/resize, attached popups follow the parent's current visual origin and
output constraints, retaining their own shape and input regions. A popup grab
prevents starting an incompatible geometry effect; close/minimize must clean up
its seat ownership and dismiss the popup through the existing protocol path.
Layers remain outside ordinary window animation policy.

### Requested, committed and displayed geometry

Track three different rectangles: desired policy frame, current committed source
frame/content, and current displayed frame. Send final target size/state once;
render the committed source into the displayed rectangle while a client responds.
This changes the present old-content clipping behavior during animated resizes,
so the implementation must update the platform specification explicitly.

For maximize/fullscreen/size-changing reorder:

1. Retain the old committed source and request the final configure.
2. Begin geometric movement immediately with the old image. Do not wait on a
   global transaction or configure-ACK barrier.
3. When a matching committed buffer arrives, replace the live source at the
   current visual pose. If the change is visually substantial, use a short
   crossfade (proposed 80 ms at speed 1) of old/new premultiplied images within
   the same displayed frame. An ACK without a corresponding commit is insufficient.
4. If the client is late, finish geometry with the old source fitted to the target;
   keep the source's original aspect when client constraints prevent the requested
   size. A subsequent usable commit updates content without restarting the entire
   transition. Bound old/new image lifetime and never freeze the event loop.

Correlate configures/commits using actual adapter state and serials rather than
assuming any commit or a coincidentally equal buffer size acknowledges the latest
request. Superseded configures cannot retarget back to stale geometry. SSD mode
continues to change only at its negotiated commit boundary; fullscreen may require
an explicit separate decoration-visibility transition.

### Input and focus

Rendering and pointer hits consume the same visual frame, output clips and stack
order. Maintain the last presented frame for input between redraws; if the nested
backend lacks confirmed presentation timestamps, document its last-submitted
approximation rather than hit-testing a speculative future pose.

For a live scaled surface, inverse-transform the pointer into source coordinates
before applying input regions and subsurface hits. Extend `SurfaceHit` and event
delivery beyond origin subtraction; local motion/button coordinates, SSD control
hits and popup offsets must agree. Check rounded cut-outs in source coordinates.
Re-evaluate pointer focus when an animation moves a window under a stationary
pointer. Existing keyboard/pointer grab and physical release suppression rules
still apply.

Closing and minimizing ghosts never accept input or own protocol focus. Workspace
slides temporarily consume new application pointer presses until the scene settles;
allow compositor shortcuts and policy keyboard focus on the destination, with
paired release suppression. Defer a slide while an active pointer grab/held
application button would make this unsafe. Panels retain their normal input.
Do not redirect a click on the outgoing workspace to a hidden destination surface.

Overview retains compositor input ownership through opening and closing, and its
animated layout supplies card hit boxes. Selection activation commits core policy
once, but ordinary pointer/keyboard delivery resumes only after the closing visual
handoff and intercepted event pairs have completed. Immediate authoritative focus
changes must not make covered clients receive overview clicks or held keys.

## Effects

The parameters below are proposed starting defaults, to be tuned with captures
and real input. They become contracts only with their implementation.

| Effect | Proposed motion | Default timing at speed 1 |
| --- | --- | --- |
| Workspace switch | Full-screen horizontal translation, shared across changed group outputs | Critical spring, stiffness 1000 |
| Window open | Opacity `0 -> 1`, center scale `1.04 -> 1.00` | 150 ms, ease-out-expo |
| Window close | Opacity `1 -> 0`, center scale `1.00 -> 1.04` | 150 ms, ease-out-quad |
| Tiled movement/reflow | Current displayed rectangle to new tile rectangle | Critical spring, stiffness 800 |
| Minimize/restore | Translate toward/from panel target and shrink/grow with late fade | 250 ms, ease-out-cubic |
| Maximize/restore | Current frame to usable output frame, neighbors reflow together | Critical spring, stiffness 800 |
| Overview open/close | Window frames to/from cards; full-output wallpaper to/from canvas | Critical spring, stiffness 800 |
| Fullscreen/restore | Current frame to/from full output, decoration visibility transitions | Critical spring, stiffness 800 |

Critical springs use damping ratio 1. Their duration depends on travel and
settling thresholds; stiffness is not a duration in milliseconds. Group effects
normally use one normalized progress track to keep all rectangles synchronized.
When retargeting requires distinct inherited rectangle velocities, sample
component springs with the same group epoch/settings and settle the group after
all its components finish, rather than sacrificing continuity to force a scalar
progress value.

### 1. Workspace switching

Order workspaces by the current stable UI/core order, not their display labels.
Moving to a later workspace brings its scene in from the right while the outgoing
scene exits left; moving to an earlier workspace reverses this. An explicit jump
across multiple IDs still moves one viewport width, avoiding a long traversal of
intermediate workspaces. There is no wraparound path in the initial design.

Commit `SwitchWorkspace` normally, including swapping whole output groups when
the target is already visible. Retain a presentation-only outgoing scene, using
already captured committed images and old clips; compose incoming windows from
the newly reconciled placements. The outgoing scene is a bounded visual exception
to the current hidden-workspace rendering rule, not a second policy presentation
or a live hidden workspace requiring callbacks.

For stretched groups, apply a common normalized progress and direction, clip each
member to its full output, and scale displacement by each member's viewport width.
No element paints into monitor gaps or an independently presented neighbor. A
swap animates both changed groups from one transition ID; never render the same
live window twice in ordinary policy placement or duplicate its frame callbacks.

Keep panels and the output's wallpaper stationary: current wallpapers are
output-specific, not workspace-specific. Translate the workspace application scene,
including retained popup visuals when safe. Topology/stretch/split changes settle
or rebuild affected tracks instead of trying to slide through invalid geometry.

Rapid `A -> B -> C` and `A -> B -> A` navigation retargets from the current visual
composite. Flatten an interrupted outgoing composite if needed, and release its
old sources; do not grow one scene per key press. Capture a newly arrived client
only into its current workspace scene, without replaying old workspace switches.

Represent direction as a 2D vector selected by a `WorkspaceTransitionDirection`
resolver. Implement only `(±1, 0)` initially. A future workspace grid can supply
`(0, ±1)` and choose vertical viewport extent without changing the compositor
tracks, renderer or timing engine. Do not expose a nonfunctional grid config now.

### 2. Window creation

Start on the first usable buffer map, after classification and initial target
placement exist, not on bufferless toplevel creation. Hidden first maps should
not trigger an offscreen animation that later replays on workspace reveal.

Apply the same progress to opacity and scale around the committed frame center:
`alpha = p`, `scale = 1.04 - 0.04*p`. Transform the complete content/SSD silhouette.
Clip overscale only at actual output/workspace boundaries, not automatically at
the final tile rectangle, so the requested larger starting appearance is visible.
Neighbor reflow uses the opening transition's timing. Treat compositor-classified
XDG launchers consistently, but leave layer-shell clients and popups out of scope.

Pre-map maximize or future fullscreen opens directly into its correct final state;
there must be no first frame at an unrelated ordinary geometry. Focus follows
normal mapping policy immediately. A fast unmap interrupts the open from its
current scale/opacity and creates at most one closing ghost.

### 3. Window closing/destruction

Closing is the reverse of the requested opening appearance: the window becomes
slightly **larger** while fading out. It is not the minimize shrink effect.
Interruption starts from the current visual scale/opacity.

Sending `Effect::Close` only asks the client to close. Start the effect when the
buffer actually unmaps or the toplevel disappears; a client refusing a close
request must remain fully usable. Remove core state, layout participation and
protocol focus immediately on actual removal, while an input-free owned image
finishes above the appropriate reflowing neighbors.

Do not attempt to import a destroyed surface after cleanup. Before
`on_commit_buffer_handler`/unmap processing can drop the old root/tree buffer,
retain a renderable source snapshot, with correct buffer-release ownership, or
use a preexisting compositor-owned last-presented image. Finalize an owned GPU
image at a safe renderer point before releasing those temporary buffer references.
Audit both null-buffer commit and direct client disconnect/destruction; handling
only graceful close is insufficient. Keep a bounded recent-source cache if the
destruction callback arrives after tree state becomes unavailable.

Coalesce null-buffer unmap followed by destruction into one ghost. A remap of the
same toplevel uses a new mapping generation, cancels its old ghost and gets an
ordinary opening effect. Never associate retained textures with a new lifecycle
solely because its `WindowId` is unchanged. If no safe image survives or memory
is exhausted, remove instantly and still animate eligible neighbor reflow.

### 4. Moving between tiled positions

Compare old/new placements for the same mapped normal windows after reorder,
mode changes, transfers, open/close/minimize reflow, reservation changes and
built-in/Rhai layout changes. This animates geometry already produced by policy;
it does not add a new tile-swap command to Clear.

Translate equal-size tiles directly. Size-changing moves use the committed-image
resize path and request final client sizes once. Lift the directly moved/focused
window above affected tiles during overlap, using a stable temporary stack order;
untouched windows retain their policy order. At completion restore exact normal
stacking. Include transformed clips in both rendering and hit-testing rather than
clipping every intermediate window to its final rectangle.

Prevent per-commit restart loops: unchanged targets or changes only to committed
content are not new placement transitions. Coalesce reservation/layout changes
within a policy batch. Cross-output movement is clipped to the authorized source
and destination group outputs; there is no painting through unrelated workspaces
or monitor gaps. Moving to a hidden workspace may animate an outgoing fade, but
must not cause that workspace to appear or focus it. Transfers via overview use
its card/drag presentation rather than playing a second desktop move effect.

### 5. Minimize and restore

On minimize, immediately apply existing core minimize/focus repair. Freeze the
current displayed image as an input-free ghost; shrink it uniformly while moving
its center toward a resolved target. Preserve aspect ratio. A proposed endpoint
fits the image inside the icon rectangle (or a 24×24 logical-pixel fallback),
with opacity fading during the last third. No client receives tiny-size configures.
Core minimized state can set protocol Suspended as it does today because the
effect uses the owned image, not further client drawing.

Resolve a target once at effect start, in this order:

1. Valid visible exact-window icon hint on the window's source output.
2. Valid visible matching-app group icon hint on that output.
3. Center of a mapped panel/dock body on that output, chosen by a stable panel
   priority then mapping identity; support top, bottom and side panels.
4. Bottom-center of the full output when no panel is mapped or IPC is disabled.

Panel body geometry comes from mapped layer arrangement, never the usable area
alone or a namespace guessed to be a taskbar. Recognize configured panel rules,
registered hint providers, and edge-anchored exclusive top/bottom-layer panels;
avoid treating a full-output overlay as a dock. Bound the target to the output.
If the chosen panel disappears, keep the already resolved point for this short
effect; output loss cancels it. Layout movement of an icon during minimization
does not chase or jitter the target.

Restore on a visible workspace reverses the path toward the final ordinary or
maximized placement and unsuspends the client through normal reconciliation.
Reuse the ghost when reversing in flight; after completion use the latest retained
committed source when available. Restoring on a hidden workspace updates policy
without exposing it or replaying a restore on a later workspace switch. Explicit
focus/reveal can combine incoming workspace and restore as one transition.

#### Optional panel/icon hint protocol

Add an additive, negotiated IPC capability rather than trusting arbitrary screen
coordinates attached to `set_minimized`. Keep legacy requests unchanged. The
proposed capability exposes opaque mapped panel IDs/generations and accepts
`set_animation_targets` for a panel, containing replaceable window-ID or exact
app-ID targets with **panel-local logical** rectangles. The adapter translates
them using the panel's current committed origin and clips them to its visible body.
Capability discovery also lets the example shell keep working with older Clear.

- Tie registration to an IPC connection and a mapped Wayland panel owned by the
  same authenticated peer process, using transport peer credentials and Wayland
  client credentials. Namespace/app-ID strings are not ownership proof. The
  current transport does not provide this association; it must be implemented
  and tested. A shell using a separate helper process gets fallback behavior until
  an explicit delegation mechanism exists.
- Accept only finite, positive, bounded rectangles and existing normal window
  targets; reject unknown fields, stale panel generations/output IDs, conflicting
  target selectors and targets outside the visible panel. Keep validation in the
  backend-independent schema plus adapter ownership/geometry validation.
- Propose at most 128 targets per registered panel, 512 total, at most eight
  providers and one owner per panel. Preserve current transport message/work
  budgets. Replace a panel's set atomically, coalesce updates, and expire unused
  hints after a bounded lease (proposed five seconds, renewed only while mapped).
- Clear registrations on disconnect, panel unmap/destruction, output loss or
  mapping-generation change; invalidate coordinates on panel geometry changes
  until refreshed. Never publish texture data or per-frame animation state.
- The Quickshell `AppDock` reports icon bounds after layout/panel commits, pin/task
  changes, dock scrolling and output changes; exclude clipped or invisible icons.
  One grouped icon can target an app, while an individual task icon can target a
  window. Shell absence never prevents minimize/restore.

### 6. Maximize and restore

Animate the current displayed frame to the home output's **usable** rectangle,
including reservation changes, and reverse to the current layout/saved floating
rectangle. Preserve all existing maximize/minimize semantics and saved geometry.
Animate affected tiles from the same operation group; do not animate a separate
normal-to-maximized intermediate step for an initially maximized first map.

Keep SSD visible according to current negotiation, transform the original outline,
and preserve configured border/corner appearance. Current maximization changes the
SSD maximize control to restore; update that control with the policy transition.
Any later maximize-specific border/corner styling would need its own documented
behavior and coordinated blend. Repeated maximize/restore and reservation changes
retarget continuously; delayed clients use the shared resize source rules.

### 7. Overview

Extend the session/presentation lifecycle to `Opening`, `Open`, `Closing`, plus
the existing pending-entry authorization. Retain the closing visual model after
selection/cancel, even if its final desktop command has already committed. Avoid
clearing the thumbnail cache immediately when the runtime selection ends.

On entry, pair current visible desktop window frames with the corresponding card
destinations. Interpolate their centers/sizes using one progress track; fade
captions, selection accents and workspace-strip UI in with that track. Draw each
window once at its moving pose, not both a stationary desktop window and a card.
Windows excluded by pagination fade out; minimized/hidden/offscreen candidates
without a visible desktop pose fade into their card, optionally originating from
a valid minimize anchor. Do not manufacture hidden-workspace policy placements.

Interpolate the **full-output wallpaper composition** into the existing inset
canvas so it visibly scales down with the windows. Preserve its full-output crop,
letterboxing and `fit`/`center` behavior rather than recomputing texture placement
for the shrinking rectangle. The outer backdrop/dim pass and ordinary layer
coverage must blend in continuously; painting the current opaque final overview
backdrop at progress zero would hide the entire opening motion. Other outputs
dim smoothly and retain the overview's input interception.

On cancel, use freshly reconciled current desktop placements as exit targets,
allowing for windows removed or moved during overview. On selection, commit the
existing `Focus`/workspace command once and animate from cards to its result.
Restore a selected minimized window through that single card-to-desktop motion;
suppress an additional minimize/workspace/open effect for the same window.
Fade unavailable card targets out rather than leaving stale surface references.

During overview, workspace preview, pagination and window drops animate the card
layout itself. New cards fade in; removed cards fade out only from bounded owned
images. Drag ghosts follow the pointer directly. A new toggle reverses the current
progress without a snap; after activation has committed, reopening previews the
new desktop state and does not roll that command back. Changing interactive output
settles/transfers presentation safely and rebuilds output-local geometry.

Reuse live committed image updates and existing cache limits. Closing may retain
resources until its last frame, but it must restore preview-only output membership
and protocol suspension when those previews cease to be used. Card transforms
run at the configured animation rate even though preview-only clients remain
throttled to 33 ms callbacks. Search and gesture progress remain later slices.

### 8. Fullscreen and restore

Deliver fullscreen policy before enabling its animation:

- Add explicit core state/commands, layout exclusion, normal-role validation,
  typed actions, declarative Rhai support and optional allowlisted shell state
  operations. Keep minimize independent. Fullscreen preserves floating geometry,
  order, proportions and the underlying maximized flag so exit can restore to
  maximized or ordinary state.
- Place fullscreen in the home output's **full** bounds, excluding neither panels
  nor gaps and never spanning a stretched group. Core currently stores only
  usable `Output.area`; add backend-independent full bounds supplied by the
  adapter, retaining `area` for normal/maximized placement. Test topology repair.
- Support XDG fullscreen/unfullscreen, including requests before first map and
  optional requested outputs. Resolve valid connected targets without copying
  workspace ownership; use the ordinary output/workspace transfer rules when
  an explicit valid target requires a move. Invalid/disconnected targets use the
  current home/focused output, with no unrelated presentation changes.
- Send final size and Fullscreen state once, advertise the capability only when
  this policy works, and distinguish pending request from committed client state.
  Launcher roles reject it. Define move/resize refusal until fullscreen is exited.
- Hide compositor SSD titlebar/border visually for fullscreen without falsifying
  XDG decoration negotiation. Client-side decoration is client-controlled;
  request fullscreen and use committed content when it arrives. Preserve the
  exact restored decoration/inset behavior.
- Proposed stack: overlay layers above fullscreen; fullscreen above ordinary top
  panels and applications on its output. Exclusive layer keyboard authorization
  retains precedence. Reservations remain active for normal/maximized windows;
  fullscreen ignores them. Overview remains available through its existing
  authorization and shows the committed fullscreen image. Multiple fullscreen
  windows use deterministic focus/stack order with only the eligible top one hit.
- Setting maximize while fullscreen updates the underlying restore state;
  minimization hides fullscreen while preserving it, and restore returns to
  fullscreen. Reclassification to launcher clears unsupported states consistently.

Then reuse the maximize/resize transition to full bounds. Blend SSD visibility,
border/corners and panel occlusion from the current pose; exit reverses toward
the newly reconciled usable/tiled/floating frame. A delayed fullscreen commit
must not leave a titlebar visible over a supposedly decoration-free final source
or block other animations. Test fullscreen/overview/minimize interleavings.
Native direct scanout should be disabled during transforms and re-enabled only
after identity geometry settles when that backend exists; it is not implemented
in the current nested path.

## Configuration example

The schema below is accepted by the implemented foundation. Visual effect
triggers remain pending; use the [animation specification](../specs/animations.md#configuration)
for current defaults and validation. This example opts the pure engine into motion
and does not yet animate the compositor scene:

```toml
[animations]
enabled = true
reduced_motion = false
speed = 1.0                 # 2.0: twice as fast; 0.5: half as fast
frame_rate = "refresh-rate" # alternatively 30, 60, or 120 as integers

[animations.workspace_switch]
kind = "spring"
stiffness = 1000.0          # critically damped in the initial implementation

[animations.window_open]
kind = "easing"
duration_ms = 150
curve = "ease-out-expo"
scale = 1.04

[animations.window_close]
kind = "easing"
duration_ms = 150
curve = "ease-out-quad"
scale = 1.04

[animations.window_movement]
kind = "spring"
stiffness = 800.0

[animations.minimize]
kind = "easing"
duration_ms = 250
curve = "ease-out-cubic"

[animations.maximize]
kind = "spring"
stiffness = 800.0

[animations.overview]
kind = "spring"
stiffness = 800.0

[animations.fullscreen]
kind = "spring"
stiffness = 800.0
```

Restore is planned to share each corresponding effect's settings. Keep custom
shaders and damping/bounce controls out of the first visual release. Engine
validation, settling, speed rebasing and atomic reload are now specified in
[animations](../specs/animations.md). Visual integration must retire retained GPU
resources on instant settlement and redraw the final state; the pure engine
currently owns no GPU images.

Successful theme reload invalidates incompatible images and rebuilds or settles
affected transitions; closing ghosts without a live source can finish with their
captured appearance. Commented foundation settings are included in the standard and liquid-glass
examples. Reduced motion initially means instant transitions;
automatic desktop accessibility preference integration is a separate feature.

## Bounds, failure handling and instrumentation

Use explicit limits from the first GPU slice. Suggested initial animation-only
limits, to measure before becoming a specification:

- At most 256 active window tracks, 64 owned retained images and two outgoing
  workspace composites per affected presentation group (old composite plus its
  replacement only during handoff). Superseded tracks are replaced, not queued.
- Retained animation images share a 128 MiB aggregate RGBA8 budget, including
  outgoing workspace composites and old/new resize images. Cap a single image
  at 64 MiB and 8192 pixels per dimension. Choose resolution from visible output
  pixels, avoiding unconditional reuse of an overview-sized low-resolution card
  for a full-screen close effect.
- Reusable capture scratch targets have their own aggregate 128 MiB bound, at
  most two 64 MiB images. Count retained and temporary allocations separately;
  report outstanding GPU resources/driver overhead as additional, and never
  claim these numbers bound existing wallpaper/blur/titlebar caches.
- Keep existing overview cache/label/source limits until explicitly changed in
  its specification. Shared texture ownership must be immutable while borrowed,
  with byte accounting and release responsibility in one place. Sharing a
  mutable overview card texture with a frozen close ghost is unsafe.
- Evict unused images first. If a required image/track cannot be allocated, finish
  that effect instantly or use bounded aspect-preserving downsampling. Do not
  hide a live window, wait for memory indefinitely, read back to CPU, or reduce
  the animation frame-rate target. Missing wallpaper uses the existing theme
  background, which participates in the same canvas transform.
- Output loss/host resize invalidates geometry-dependent composites and timing;
  settle/rebuild affected effects against repaired core state. Renderer reset,
  shutdown and loss of all outputs drop adapter resources. Window removal clears
  its live track but may retain only the explicitly owned closing ghost.

Add opt-in bounded tracing/counters for timing source, nominal/effective target,
requested versus observed cadence, preparation/render duration, submitted and
presented frames (separately), misses, retained bytes, active tracks and fallback
counts. Measure p50/p95/p99 frame intervals and response latency. Avoid per-frame
shell JSON publications or unbounded log histories. Submitted fps is not evidence
of physical presentation fps when the backend has only estimated feedback.

## Delivery sequence and acceptance gates

Each slice updates its owning specification with implemented behavior and tests.
The [animation specification](../specs/animations.md) now owns engine/configuration
contracts; existing component specs own desktop policy, protocol, input, overview
and IPC behavior. Slice 2's foundation and slice 8's instant policy are implemented.
Slice 1 has synchronized swaps and sampling, but idle invalidation/deadline timers
and physical timing verification remain pending. Other effects and acceptance
gates remain work below.

1. **Timing and synchronization groundwork.** Implement the scheduler/timing-source
   seam, explicit synchronized initialization, separated render/reconcile dirtiness,
   callback opportunities and deadline timers. Verify Wayland/X11 behavior supported
   by the actual pinned backend, monitor-rate changes and capture of an idle scene.
   Gate: no animation-driven busy loop when idle; no 16 ms timer ceiling; one
   coalesced pending submission; real or honestly labeled estimated pacing.
   Update platform/rendering specs.
2. **Pure animation engine and configuration.** Add epoch-based speed mapping,
   easing, critical springs, f64 rectangles, transition groups, validation and
   atomic reload. Implement instant completion before GPU effects.
   Gate: deterministic time tests give identical poses at 30/60/120/refresh-rate
   sampling and after skipped frames; speed/reload preserves current pose.
   Add animation/configuration specs and relevant module guidance.
3. **Shared rendering and input transforms.** Extract committed GPU image
   composition, retain lifecycle-safe snapshots, implement presentation geometry,
   inverse-coordinate hits, clips, stack/damage and GPU reuse ownership. Preserve
   overview's current committed appearance and bounds during extraction.
   Gate: opaque/translucent SSD/CSD, subsurfaces, geometry offsets, rounded holes,
   popups and output-edge clips agree visually and for input; destruction uses
   an owned image, never a dead surface.
   Update rendering/platform/decorations specs as necessary.
4. **Open, close and tiled reflow.** Add causes through every entry path, mapping
   generations, graceful/unexpected destruction and animated placement diffs.
   Gate: refused close stays usable; null-unmap/destruction yields one ghost;
   rapid map/unmap and Rhai layouts converge to ordinary final policy without
   extra configures/scripts per animation frame.
5. **Workspace slides.** Add outgoing group scenes, horizontal direction resolution,
   stable wallpaper/panel stacking, retargeting and transient input suppression.
   Gate: hidden switch, two-group swap, stretched/odd outputs and rapid reversals
   have no duplicate live policy placement, gap leaks or unbounded scene chain.
   Update desktop/input/platform specs for the presentation-only exception.
6. **Maximize and minimize/restore.** Reuse size-transition sources; implement
   mapped-panel and no-panel fallback first. Add negotiated icon hint registration,
   connection ownership and Quickshell reporting as a separately reviewable part
   of this slice. Do not block fallback animations on a shell running.
   Gate: delayed commits, interrupted restore, all panel edges, grouped icons,
   scrolling/stale hints and shell restart pass; saved geometry/proportions remain
   unchanged. Update window-state, shell and configuration contracts.
7. **Overview motion.** Add phases and card/wallpaper interpolation, transition-time
   hit regions, phase reversal and one selection-to-desktop handoff.
   Gate: progress-zero/final frames match ordinary/overview scenes; wallpaper
   visibly shrinks; no stationary duplicate window or opaque early cover; minimize
   selection, pagination, drops and preview callback bounds still work.
   Update overview/input/rendering contracts and its design status.
8. **Fullscreen policy, then animation.** Implement full output geometry,
   state/commands/XDG requests/capability/insets/stacking/restore first, with its
   own core/protocol tests. Enable the shared geometry/style animation afterward.
   Gate: pre-map fullscreen, output targets, delayed client commits, reservations,
   overlays, minimize and maximized restore all behave correctly at rest and in
   motion. Update desktop/platform/input/scripting/shell/decorations specs.
9. **Performance and release validation.** Profile combined effects with existing
   blur/glass and many windows; tune defaults/budgets from measurements. Enable
   animations by default only after timing and input acceptance gates pass.
   Publish portable commands and observed limits in the testing guide, separating
   CPU, GPU/protocol and physical-input evidence.

The dependency chain is 1–3 before effects; 4 before 5–7; fullscreen policy can be
implemented independently of the effects, but its animation depends on the shared
resize path. Icon hints are optional enhancement work within minimize, not a
dependency for other animations. No committed dates are implied.

## Verification plan

This document change requires source/text/link review only. The tests below are
work to add/run with implementation; none are claimed to have passed here.

### Deterministic CPU tests

- Test clock rebasing, finite validation, zero duration, settling, opacity/size
  clamps, repeated retargeting and per-group synchronization using injected time.
  Compare poses at identical elapsed timestamps across different sampling rates.
- Inject 59-on-60 presentation sequences and bursts of missed opportunities.
  Assert the requested target stays 60, no divisor fallback appears, no backlog
  accumulates, and effects finish by elapsed time. Test 30/60/120 caps on 59.94,
  60, 90, 120, 144 and 165 Hz with bounded phase error and no rounding drift.
- Test separate client repaint/callback opportunities at capped motion rates,
  callback-only wakeups, an idle scene, occlusion/resume, timing reset, host mode
  change and future independently paced outputs through a fake timing source.
- Test scaled input coordinates, stationary-pointer focus repair, cut-outs,
  subsurfaces, SSD controls, popup origins, workspace input gating and physical
  release suppression. Include effect/grab interruption and host focus loss.
- Extend existing [window-state tests](../tests/window_state.rs),
  [overview tests](../tests/overview.rs), [runtime tests](../tests/runtime.rs),
  [core tests](../tests/core.rs) and [shell IPC tests](../tests/shell_ipc.rs).
  Compare animated versus instant command sequences for identical final state,
  focus, group ownership, placements and saved geometry; test fullscreen separately.
- Test snapshot/track byte limits, generation reuse, unmap-plus-destroy coalescing,
  stale configures, lease expiry, authenticated hint ownership, disconnect cleanup,
  unsupported shell capability and memory-allocation fallback.

### Bounded GPU/protocol fixtures

Add a portable `scripts/vm-animation-smoke.py` using the existing fixture style
and a test-only injected animation clock/capture sequence. Keep test controls out
of the public shell command allowlist. Capture start, intermediate, interrupted
and exact final poses, not only one screenshot near `--exit-after`.

Cover all eight effects, including full-output wallpaper shrink; SSD/CSD,
alpha/subsurfaces, offsets, odd/nonzero output origins, stretched groups, popup
behavior, blur/glass foreground sharpness and output boundaries. Include clients
holding configure ACKs/commits, nonmatching sizes, pre-map maximize/fullscreen,
unexpected disconnect and remap. Check protocol sizes/states/callbacks independently
of pixels and compare final captures against motion-disabled rendering.

Extend [overview smoke](../scripts/vm-overview-smoke.py),
[window-state smoke](../scripts/vm-window-state-smoke.py),
[layer smoke](../scripts/vm-layer-smoke.py),
[decoration smoke](../scripts/vm-decoration-smoke.py),
[wallpaper smoke](../scripts/vm-wallpaper-smoke.py), and
[shell smoke](../scripts/vm-shell-smoke.py) where their existing assertions provide
regression evidence. Every compositor run is bounded, e.g. `--exit-after 15`, and
artifacts stay under `target/`. Run the Rhai path as well as built-in layouts.

Deterministic clock captures establish geometry/appearance, not real cadence.
Run additional real-time traces at available host refresh rates with fixed caps,
light load and deliberately late frames. Confirm no target clamp, bounded latency
and idle wakeups; disclose estimated versus actual presentation feedback. Native
physical-refresh/double-buffer behavior cannot be proven by virtual-output GPU
captures alone.

### Real input and required Rust checks

In an isolated compositor session, manually verify rapid workspace reversal,
clicking moving/scaled windows, titlebar controls, maximize/restore, panel icon
targeting, overview selection/drag/cancel, fullscreen exit and host focus loss.
Confirm no leaked presses/releases, grab breaks, hidden-client hits or wrong focus.
Touchpad gestures remain unimplemented and are not an acceptance claim here.

For each Rust slice, run `cargo fmt`, `cargo check --locked`,
`cargo test --locked`, and check editor diagnostics; build before GPU fixtures.
Optional Quickshell changes also use its pure protocol/model tests and bounded
shell smoke. Record exact cases and limits in [VM testing](vm-testing.md), without
turning CPU tests or source inspection into GPU/physical-input success claims.
