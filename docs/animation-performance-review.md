# Animation performance review

Review baseline: `e124e28` (`rewrite/modular-wm`). This is a source review of the
implemented nested renderer, including ordinary window/workspace effects,
overview entry/exit, drag pickup/proximity scaling, desktop-tile zoom, and shell
reveal. Recommendations below describe the baseline, not an assertion that a
subsequent checkout still has every issue.

The review inspected the pinned Smithay renderer/event-source implementation as
well as Clear. It did not run a frame-time benchmark, GPU profiler, or physical
input test. Pass counts and invalidation conditions below are established by
source inspection. Their ranking is an estimate of likely benefit; no measured
FPS increase or percentage speedup is claimed. Validation of applied changes
belongs in [the testing guide](vm-testing.md).

The implemented contracts remain in [animations](../specs/animations.md),
[overview](../specs/overview.md), [rendering](../specs/rendering.md), and
[nested timing](../specs/platform.md#nested-frame-timing).

## Recommended first changes

| Priority | Change | Affected work | Estimated effort / risk |
| --- | --- | --- | --- |
| 1 | Keep sufficient thumbnail resolution stable during transforms | Overview entry/exit, drag scaling, tile zoom | Medium / medium: quality and bounded cache admission |
| 1 | Skip the ordinary scene covered by overview | All overview frames, especially blur/glass themes | Small / low to medium: shell ordering and fallback |
| 2 | Omit the image composer's identity-copy pass | Source preparation for every ordinary effect; full-resolution previews | Small / low: retain mask and synchronization ordering |
| 2 | Prepare each overview source once per frame | Cards shared with desktop miniatures or drag ghosts | Small / low: invalidate on the next committed frame |
| 2 | Sample the ordinary planner once on successful preparation | All ordinary effects and enabled idle frames | Small / low: retain resampling after failures |

These changes reduce work without changing motion laws, pointer following,
presentation acknowledgement, swap synchronization, or configured animation caps.
They are suitable for separate, reviewable commits before broader renderer work.

### 1. Destination size currently invalidates committed content

At the review baseline,
[`PreviewSizes::texture_size` and `OverviewCache::thumbnail`](../src/platform/smithay/overview.rs)
choose the exact largest **sampled** preview dimensions. `ThumbnailAppearance`
includes this exact size. Any change removes the cached thumbnail; reuse is
allowed only when the old texture has the new exact size. The renderer then
composes the entire committed image again.

Consequently, a static client can incur fresh texture allocation and three
offscreen passes on every frame of overview resizing. This is especially visible
in tile zoom: the texture grows from miniature to desktop size even though the
client has not changed. Drag proximity scaling repeatedly shrinks and grows the
same source. Entry/exit similarly changes card sizes.

Separate content identity from requested sampling resolution. Keep a cached
texture when its content identity is unchanged and it already has sufficient
resolution. Do not shrink it for each interpolated rectangle. Plan resolution
from known transition endpoints where available; for arbitrary gesture or
interruption growth, use bounded capacity steps or another policy that avoids a
new allocation for every pixel of growth. Preserve aspect ratio and the actual
committed-source resolution limit. A sufficient larger image should also remain
the target size when fresh content commits during the transition.

The existing per-frame maximum across all destinations is still necessary:
rendering a miniature first must never prepare an undersized image later used
by a larger card. Endpoint/capacity planning supplements that rule. Maintain the
128-entry/64 MiB bounds using the **actual retained allocation**, and handle
pressure before allocating. Reopening, client size/style changes, output changes,
and source destruction must still invalidate the correct resources. Document
any deliberate capacity retention or shrink policy in the overview spec because
the baseline explicitly requires releasing changed-size thumbnails.

Avoid promoting every card to its maximum native resolution: twelve 2048-square
RGBA8 images alone require 192 MiB, so they can evict and recompose each other
under the 64 MiB cache. Even genuine desktop endpoints can be large for many
overlapping floating or monocle windows. Reserve the current required visible
sizes first, then admit endpoint/capacity upgrades from remaining headroom.
Under pressure, discard optional excess capacity before degrading to repeated
eviction of currently visible sources. Never satisfy a prewarming preference by
making a required visible card sample an undersized capture. The animation state
already holds target/previous card layouts, desktop frames and tile-zoom
endpoints, so these can feed planning without new hidden-workspace layouts.

Expected reduction is structural: for unchanged content and a sufficient cached
image, a sequence of destination-only changes requires texture draws rather than
repeated image composition. This does not imply a particular FPS gain.

Validation: unit-test stable reuse while shrinking, growth admission, aspect
ratio, source/max-edge limits, and byte accounting. Exercise both directions and
interruption of entry/exit, drag approach/retreat, and tile zoom. Run overview
detail/rounded/live-buffer GPU cases to detect low-resolution upscaling, stale
commits, or source-mask regressions.

### 2. Overview renders and filters an invisible ordinary scene

[`backend.rs`](../src/platform/smithay/backend.rs) calls `scene_elements` before
constructing overview, then runs the ordinary backdrop compositor when blur or
glass is enabled. [`scene.rs`](../src/platform/smithay/scene.rs) already omits
ordinary application windows under overview, but still collects ordinary layer
trees, their popups, and wallpaper. Those layer trees can trigger multiple blur
passes even though the overview covers them.

The end of [`OverviewCache::elements`](../src/platform/smithay/overview.rs)
provides a full-output `desktop_backing` for **every** output. That helper places
an opaque theme-color base under the wallpaper, including wallpapers with alpha.
Thus the ordinary layer/wallpaper scene contributes no visible pixels while a
valid overview frame is present. Final framebuffer occlusion cannot recover the
offscreen work already performed by the blur compositor.

Choose the scene after determining whether overview elements were produced:
ordinary overview frames need only the overview elements. During desktop-tile
zoom, build the existing revealed top/overlay shell scene over the live overview,
then apply its normal blur/glass grouping. The hidden ordinary scene can be
omitted beneath that complete backing as well. If overview construction cannot
provide its complete backing, retain the ordinary fallback rather than making
an empty output. Skip invoking blur for the plain overview-only scene; merely
passing an empty ordinary scene to blur still clears/composes a full framebuffer.

Preserve renderer cache housekeeping currently performed in `scene_elements`,
particularly wallpaper retention. Keep callbacks, surface output membership,
suspension, policy reconciliation, and input ownership independent of this
rendering shortcut. The shell reveal must still show eligible popups and honor
fullscreen Top/Overlay order and output-local opacity.

Validation: overview entry/exit, asymmetric/tiny/odd outputs, wallpaper alpha or
missing wallpaper, and multiple outputs; desktop-tile shell reveal with blur,
glass, popups, and a fullscreen target. Verify that the outer backdrop stays
opaque and that the shell still appears during zoom rather than afterward.

### 3. Full-resolution image preparation has an unnecessary copy

[`WindowImageComposer::compose`](../src/platform/smithay/window_image.rs)
currently performs three offscreen stages: compose the committed body/SSD into
the body scratch texture; apply the original silhouette into masked scratch;
copy/scale masked scratch into the independently owned result. All three stages
clear their initialized target and finish with renderer synchronization.

For the ordinary animation cache,
[`animations.rs::image_size`](../src/platform/smithay/animations.rs) selects the
same bounded dimensions as the composer's native source dimensions. In that
case the third stage is an identity copy. Render the masking stage directly into
the owned result and omit the copy when the sizes match. Keep the separate
masked intermediate when a thumbnail actually needs downsampling, so its
original corner/border coverage is still baked before resampling.

This removes one of three offscreen stages for those image updates. It is not a
claim of a one-third frame-time improvement: unchanged images are cached, body
composition can draw multiple elements, and blur/final composition may dominate.
The main benefit applies when content/style changes or new images are prepared;
moving an already cached image continues to use the existing texture path.

Preserve separate read/write targets, full initialized-region clears, bounded
scratch dimensions, independent result ownership, full-source sampling, and the
window framebuffer restoration before submit. Do not repurpose a result still
held by an outgoing or last-submitted snapshot as scratch.

Validation: ordinary open/close, minimize/restore, movement/maximize/fullscreen,
held fullscreen commits, and workspace motion; rounded/transparent/subsurface
sources and GPU comparisons between full-size previews and desktop appearance.

### 4. Shared overview images still repeat source preparation

Every call to `OverviewCache::thumbnail` invokes
`PreparedWindowImage::prepare` before looking up the cached appearance. The same
window can appear in a card and miniature, or a ghost and miniature, in one
frame. These calls repeat surface-tree gathering, titlebar element creation,
signature/mapping vector construction, and cache comparisons even when the
second call will share the already prepared texture.

The overview already plans each window's largest destination before drawing
and owns a generation counter. Validate/prepare each distinct source once in
that generation, then reuse it for its additional destinations. Clients cannot
commit halfway through this synchronous rendering operation. Continue to check
the source in the next frame, and allow an unsuccessful or evicted source to
follow the existing fallback. Do not equate a cached image from an earlier
generation with a source validated this frame.

Validation: a real source visible simultaneously as a card and miniature should
update both after a buffer/subsurface/style commit. Also exercise the drag ghost
and tile zoom sharing paths and cache pressure. A temporary preparation counter
can establish one preparation per distinct visible source, without adding a
permanent debug allocation to the hot path.

### 5. The ordinary frame is sampled twice unconditionally

`prepare_window_animations` samples `PresentationPlanner` to build visuals, then
samples it again after processing `unavailable`, even when that list is empty.
Each sample constructs a new frame and samples the same bounded tracks at the
same timestamp. Use the first frame unless failed source preparation actually
settled or changed planner tracks; retain the second sample for that failure
path. Keep the acknowledgement metadata from the final chosen frame intact.

This is a small CPU improvement, particularly at high refresh rates or with many
windows. It does not justify changing analytical motion or acknowledgement rules.
Existing presentation tests cover sampling purity and missing-source retirement;
adapter tests should retain the failure/settlement behavior.

## Larger follow-up areas

### Reuse ordinary image storage without breaking snapshots

`prepare_window_animations` supplies `None` for the composer's reusable texture
on every changed source. This is safe because current, submitted, retained and
presented visuals may share an older `Arc<CachedImage>`, but live content can
allocate a fresh GPU texture on every commit. A bounded reusable pool or an
ownership-aware small image rotation could avoid allocation churn once an image
is no longer owned anywhere.

This requires explicit accounting of the pool within the existing memory budget,
renderer synchronization, mapped-generation checks, and a proof that no
outstanding element/snapshot can sample the reused allocation. Test destruction
immediately after a commit, interrupted minimize/restore, workspace reversal,
unmap/remap, held client geometry, and budget exhaustion. Do not weaken snapshot
ownership merely to reduce allocations.

### Reuse rounded appearance in ordinary settled rendering

When animations are enabled, the bridge maintains committed images for visible
ordinary windows so later closing/workspace transitions can retain the last
submitted appearance. After an effect settles, ordinary rendering returns to
[`RoundedShaders::window`](../src/platform/smithay/rounded.rs), which allocates
and composites another per-clip offscreen body. This can duplicate work already
available in the committed-image cache; unchanged rounded windows can also
repeat ordinary offscreen work every redraw.

A shared appearance path may reuse the bounded committed image for eligible
settled windows or cache the ordinary rounded composition. First reconcile
differences in committed/requested geometry, tile cropping, SSD activation/style,
output clips and fullscreen state; ordinary rendering must not accidentally fit
a held buffer using animation semantics after the effect has ended. This is a
larger change than omitting the identity copy and needs broad rounded, decoration,
blur and input regression coverage.

### Damage tracking and callback-driven redraws

The backend supplies buffer age zero to `OutputDamageTracker` and submits full
framebuffer damage. Several generated texture/solid elements also receive new
IDs every frame. Consequently, unchanged portions of the scene do not currently
benefit from stable incremental damage. Backdrop filtering independently rebuilds
its lower scene and refills a complete Kawase viewport for each filtered tree.

Incremental rendering needs a coordinated design: valid host buffer age; stable
element identity/commit versions; damage for both old and new transformed bounds;
blur support/optical sampling dependencies; cache invalidation on reload, output
changes, captures and source replacement. Do not enable nonzero buffer age as an
isolated tweak, or reuse filtered pixels when lower content moved.

Likewise, eliminating continuous redraw requires explicit wakeups for all client
and layer commits, callbacks, input/exposure, animation terminal frames, shell
commands, and capture/deadline work. It can reduce idle power, but is separate
from removing wasted work within an animation frame. Preserve continuous host
opportunities until those invalidation paths are complete.

The [demand-driven rendering plan](demand-driven-rendering-plan.md) expands this
follow-up into wakeup coverage, callback/maintenance deadlines, staged commits and
acceptance checks. It remains a proposal rather than a current scheduling contract.

### Keep the low-latency drag behavior

[`overview_drag.rs`](../src/platform/smithay/overview_drag.rs) already derives
translation directly from the latest pointer on every host frame; only pickup
offset/scale use the animation track. Position-only motion does not restart an
easing law. Preserve this while optimizing cached image sizing. Reducing source
composition and invisible filtering is a better immediate candidate for drag
responsiveness than reintroducing interpolation of pointer position.

## Timing findings and measurement plan

The `event_loop.run(Some(Duration::from_millis(16)), ...)` argument in
[`smithay/mod.rs`](../src/platform/smithay/mod.rs) is a maximum idle poll timeout,
not a fixed sleep or a 62.5 FPS cap. The pinned Smithay `WinitEventLoop` registers
its host event source and drains queued Winit events in `before_sleep`. Calloop
uses a zero timeout when that hook returns readiness. Incoming registered host
events can wake polling before the maximum timeout. Reducing 16 ms alone is not
an established animation-cadence fix and may merely increase idle housekeeping.
The separately polled shell transport can experience idle-loop latency; that
does not imply that host redraws are limited by the same interval.

Do not describe every `renderer.wait` as a measured CPU/GPU stall. Pinned
Smithay first attempts an EGL fence wait inserted into the graphics context;
CPU waiting is a fallback. Its frame finish also creates/flushes synchronization
or uses a finish fallback when required. Fewer offscreen stages reduce that
overhead, but removing synchronization broadly would need separate validation
of imported buffers, scratch reuse and driver behavior.

For comparison, use a release binary both before and after each optimization,
with the same resolution, host refresh, theme, client count and animation speed.
Comparing an unoptimized debug build with the final release build cannot isolate
the code change. Warm up shader/font/texture caches before recording steady
state, and record cold-first-animation behavior separately.

An FPS overlay should count successful compositor submissions over real elapsed
time. Update its displayed text infrequently and reuse one small cached texture.
Render it after overview and backdrop effects so it adds no blur work. Label the
metric as compositor submission cadence rather than physical presentation;
nested host scheduling can differ. With a manual animation cap, host submission
FPS can remain high while animation poses update less often. A future animation
sample-rate readout should be distinct from this number.

Record, when instrumentation is available, redraw interval distributions and
time spent in reconciliation, source preparation, ordinary/overview scene
construction, backdrop filtering, final rendering, and `backend.submit`.
Measure submit separately because synchronized swaps can wait for the host.
Asynchronous GPU timer queries are needed to attribute GPU cost reliably; CPU
time around command submission alone does not do so. Read query results later,
without forcing the GPU to finish on each measured frame.

Useful comparison scenarios are: static-client entry/exit and tile zoom; live
client content during the same motions; drag approach/retreat with fast pointer
reversals; ordinary movement/maximize/fullscreen and held commits; workspace
switch/reversal; minimize/restore and destruction. Run first with backdrop
effects disabled, then the same blur/glass settings, to separate image/cache work
from filtering cost. Keep animation caps and synchronized swaps unchanged, and
do not lower the configured cadence after a slow frame.

GPU/protocol smoke captures establish visual/state correctness, not delivered
cadence or physical pointer latency. Runtime measurements and the user's manual
comparison remain separate evidence.
