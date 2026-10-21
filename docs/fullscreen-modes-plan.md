# Fullscreen modes — implementation plan

**Status: desktop fullscreen and the nested low-latency presenter are implemented;
native direct presentation remains a proposal.** The dedicated occupied-only slot, restoration, typed navigation,
overview/shell integration and asynchronous XDG lifecycle are specified in
[desktop state](../specs/desktop.md#fullscreen),
[platform lifecycle](../specs/platform.md#xdg-window-lifecycle-and-configures),
[overview](../specs/overview.md#fullscreen-desktop), [shell](../specs/shell.md) and
[animations](../specs/animations.md). The sketches below retain the broader
future native design. `desktop` and `low-latency` are exposed, with the latter
using [nested complete-frame composition](../specs/rendering.md#low-latency-fullscreen).

## 1. Intended behavior

Support two fullscreen modes sharing **one fullscreen occupant across the entire
compositor**, including multiple outputs. This interprets “one at a time” globally,
rather than allocating one occupant per monitor.

| Property | Desktop fullscreen | Low-latency fullscreen |
| --- | --- | --- |
| Typical use | Browsers and ordinary applications | Games and latency-sensitive applications |
| Desktop destination | One fixed, special fullscreen desktop | The same slot, using a different presentation path |
| Window replacement | The previous occupant exits fullscreen and returns home | The same atomic replacement policy |
| Effects | Existing configurable fullscreen/workspace transitions | Immediate transitions; no blur, glass, rounding, SSD or retained animation images |
| Rendering | Normal demand-driven composition and applicable damage repair | No compositor damage repair; direct scanout when eligible, otherwise complete-frame rendering on new content |
| Output scheduling | Normal synchronized presentation | Latest ready content, bounded queues, supported VRR or explicitly permitted tearing |
| Availability | Implementable on the current nested backend | A nested reduced-work path first; native DRM/KMS required for direct scanout and physical output control |

The special desktop has a stable internal identity and display label `Fullscreen`.
It is absent from navigation while empty, and occupies a fixed position after the
configured desktops while occupied. It is not an ordinary numbered workspace,
cannot be configured as a layout, and cannot be a drag/drop destination. Creating
or closing fullscreen applications never creates a new desktop per application.

Low-latency mode gives the application the shortest supported presentation path.
Clear still owns output configuration, input routing, protocol lifetimes and
return to the desktop. Direct scanout displays a client buffer without Clear
rendering a replacement image; transferring display ownership through DRM leasing
is a separate feature for cooperating clients, not the normal game path.
See the upstream [Smithay scanout description](https://smithay.github.io/smithay/smithay/backend/drm/compositor/index.html)
and [kernel DRM lease interface](https://www.kernel.org/doc/html/latest/gpu/drm-uapi.html#drm-display-resource-leasing).

## 2. Replace independent flags with one policy owner

Introduce a small backend-independent `FullscreenSlot` in a focused
`src/core/fullscreen.rs` module. Keep Wayland surfaces, buffers, fences and DRM
objects in the adapter. A proposed model is:

```rust
enum FullscreenMode { Desktop, LowLatency }
enum PresentationTarget { Workspace(WorkspaceId), Fullscreen }

struct FullscreenSlot {
    window: WindowId,
    mode: FullscreenMode,
    target_output: Option<OutputId>,
    revision: u64,
    return_target: ReturnTarget,
}
```

These are the broader two-mode design sketches; the current two-mode slot API
is defined in `src/core/fullscreen.rs`. `Desktop` stores
`Option<FullscreenSlot>` and derives a window's public fullscreen boolean from
slot membership. Do not retain independently writable fullscreen flags that can
disagree with the slot.

The application keeps its normal workspace ownership, position in workspace
order, saved floating rectangle, layout proportions and underlying maximized
state. The slot is a presentation destination, not a second workspace owner.
Exclude its occupant from ordinary layout/rendering while it belongs to the slot;
the same window must never appear as both a home-workspace card and a fullscreen
card. Layout computation for remaining windows may change, but saved proportions
and floating geometry must survive.

Keep an output group's underlying normal workspace intact. Add an explicit
presentation-target query instead of pretending `Fullscreen` is a `WorkspaceId`.
On a stretched group, override only the selected output; its other outputs keep
presenting the group's normal workspace. The fullscreen destination can differ
from the saved home output without mutating that home just to satisfy an XDG
output preference. Update callers that currently assume `workspace_for_output`
also identifies everything visible there.

`ReturnTarget` records the invoking output/presentation and previous eligible
focus. Store IDs and revisions, not a full cloned desktop or a stale placement
rectangle. On exit, recompute placement from current topology, reservations and
layout. Repair missing return IDs using ordinary core focus/topology rules.
The return target must always be an ordinary presentation, including replacement
invoked while Fullscreen is selected; never save the disappearing special desktop
as its own return destination. Replacing on a different output restores the old
output's underlying desktop and invalidates its presentation history immediately.

### State transitions

| Event | Required result |
| --- | --- |
| Enter A into an empty slot | Validate A, record return context, assign A and its mode; activate the special desktop for a foreground user operation |
| Fullscreen B while A occupies it | Validate B before mutation; clear A's fullscreen state, restore A's ordinary participation, assign B and publish one final policy result |
| Repeat enter for the same A/mode/output | Idempotent; no new restore record, animation, buffer allocation or configure loop |
| Change A's mode | Keep its return context and occupancy; change the presentation path, resetting only affected backend state |
| Exit the occupant | Remove the slot and its navigation entry; return to the recorded normal presentation if the slot was being viewed |
| Exit an older, displaced window | No effect on the current occupant; an old unfullscreen request cannot clear B's slot |
| Switch to an ordinary desktop | Keep the slot occupied but dormant; stop presenting it and send normal visibility/suspension updates |
| Select Fullscreen or explicitly focus its occupant | Activate the slot on its target output without adding a second owner or clearing underlying maximize state |
| Minimize the occupant | Exit fullscreen, preserve its ordinary restore state, then minimize; remove the special desktop |
| Unmap, destroy, or classify occupant as launcher | Release the slot, repair focus and discard presentation resources; remapping begins without fullscreen |
| Move occupant to a normal workspace/output | Exit fullscreen first, then execute the explicit transfer; do not silently move the special desktop |
| Remove target output | Repair to the occupant's valid home/focused/first surviving output; if headless retain policy without placements or GPU presentation resources |

Use one mutation/reconciliation boundary for replacement. Configure A out of
fullscreen and B into fullscreen, then publish the final snapshot. Never publish
an intermediate state with two fullscreen flags. At most one normal fullscreen
application is active; its subsurfaces, popups and authorized transient dialogs
remain usable. They are not additional fullscreen occupants.

Keep explicit state setters nonrevealing. Setting fullscreen on a hidden mapped
window can replace the slot with a dormant occupant, but does not implicitly
switch desktops or grant keyboard focus. Reject entering a minimized target
without an explicit restore/focus operation. Foreground shortcuts and eligible
foreground client requests compose state change with activation; background client
requests must respect activation/focus policy. Validate invalid IDs and launcher
roles before displacing anyone. Document these choices because current setters
allow minimized fullscreen flags and have no global occupancy limit.

## 3. Desktop navigation, shell and ordinary app behavior

Extend runtime overview targets and presentation navigation with a typed
`Fullscreen` target. While occupied, append its tile after ordinary desktop tiles
on the target output and include it in next/previous desktop navigation there.
Numbered shortcuts continue selecting numbered workspaces. On other outputs,
activating the occupant focuses its target output rather than showing it twice.
When empty, no overview selection, shortcut or shell request may activate the
special desktop; repair a stale selection to a valid ordinary target.

Selecting a home workspace while its window is fullscreen shows the remaining
ordinary windows. Selecting the occupant through the dock/window list activates
the special desktop. Alt-Tab on Fullscreen should include the normal windows from
its recorded return workspace plus its occupant; accepting an ordinary window
returns to that window's normal presentation without unfullscreening the occupant.
Cancelling preserves the current presentation and focus.

Expose a separate optional `fullscreen_desktop` snapshot with owner window, mode,
target output and active/dormant status; retain the existing window `fullscreen`
boolean for compatibility. Do not inject a fabricated decimal workspace ID into
the existing `workspaces` array. Keep `output`/home distinct from the effective
presentation output. Add a narrowly validated `activate_fullscreen` shell request;
keep spawn/quit/reload outside IPC. Update Quickshell workspace navigation, dock
activation and pure protocol/model tests alongside the schema.

Desktop mode keeps smooth entry, exit and replacement using the existing animation
clock and submitted-geometry input mapping. Distinguish the fullscreen desktop
transition from ordinary window maximization. A replacement may briefly retain
the outgoing image for animation, but only B is the policy occupant; retained A
is not interactive or a second managed fullscreen window. Callback-only work
cannot acknowledge transition retirement.

Panels and unrelated launchers do not cover the fullscreen application. Reserve
explicit access to overview, the compositor switcher and authorized shell dialogs;
they temporarily take presentation/input ownership. Preserve ordinary workspace
reservations for the return desktop. Browser fullscreen popups and related modal
windows must work without revealing unrelated content underneath transparent
fullscreen pixels. Use a solid backing where the application does not cover the
output, as required by the upstream
[XDG fullscreen request](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/stable/xdg-shell/xdg-shell.xml).

## 4. Preserve asynchronous XDG lifecycle

Both modes use the standard XDG fullscreen configure/state, with final full-output
content dimensions and no SSD inset. XDG has no separate “game fullscreen” request;
Clear selects the mode from user configuration or an explicit compositor action.

Preserve the existing distinction between desired policy, ACKed configures and
buffered committed appearance. An ACK alone does not hide SSD or make a client
buffer eligible for direct scanout. A held or undersized commit must not block
other clients or output processing. Desktop mode may fit the old committed image
during the transition. Low-latency mode uses a simple full-frame composition of
the existing buffer with border fill until an eligible new buffer arrives, without
allocating an animation snapshot.

Pre-map requests record mode/output intent in `ManagedWindow`; an unbuffered
toplevel does not evict the mapped occupant or expose an empty fullscreen desktop.
Assign each accepted enter intent a monotonic revision. A newer accepted request
supersedes an older pending enter; a late first buffer, ACK or configure commit
cannot reacquire a displaced slot. Reconcile revoked pre-map intent with an exit
configure. Keep pending records bounded by existing managed toplevel lifetimes.

The existing `set_client_fullscreen` focus/move/set chain must become one request
into the controller. Do not use `MoveToOutput` to implement presentation-only
output selection. Update commit, unmap, destruction, reclassification, focus and
topology paths together. Reject unavailable output preferences through the ordinary
fallback rule; never manufacture an empty fullscreen rectangle.

## 5. Low-latency presentation

### Two concrete backend deliveries

**Nested:** on the fullscreen output, bypass ordinary desktop scene building,
wallpaper, titlebars, filters, overview preparation and retained-image composition.
Draw only the application tree, required popups/cursor and backing into the host backbuffer. Clear still
imports/renders into a host window and remains constrained by the pinned host
redraw/swap path. This is reduced compositor work, not direct scanout or a promise
of native latency. Preserve the existing synchronized nested swap contract.
Virtual outputs currently share one host framebuffer. While a visible fast-path
slot uses that framebuffer, render a complete host frame without its shared damage
tracker, composing other virtual outputs normally with their own scene clips.
Do not clear or skip the other desktops, apply their effects to the game, or free
resources they still need. This limits nested savings compared with independent
native outputs. A dormant slot does not force its special renderer to remain active.

**Native:** implement session/device ownership, DRM/KMS outputs, GBM allocation,
input, hotplug and atomic presentation first. Then present an eligible client
DMA-BUF on the output's primary plane, with a hardware cursor when available.
The client renders; Clear validates and schedules the buffer, with no wallpaper
draw, effect passes, intermediate image copy or compositor-owned primary image
in steady direct scanout. Other outputs continue their normal rendering paths.

### No damage repair on the fast path

Use a separate presenter, not `OutputDamageTracker::render_output` with age zero:
that still walks the damage tracker. In low-latency mode:

- Bypass Clear's expensive committed-tree visual fingerprint/damage comparison
  and output repair history on the affected output. Applied buffer/content/tree
  changes schedule a presentation directly; unchanged callback/input-region
  commits still use their separate callback/input paths.
- Consume and validate Wayland commits, buffer assignments, synchronized child
  transactions, viewport/transform state and synchronization normally. Damage
  messages remain legal protocol input, even though this mode does not use them
  to decide repair regions. Do not remove `on_commit_buffer_handler` or expose
  buffers before their transactions are ready.
- Maintain a cheap committed-content revision for cache invalidation and lifetime
  decisions. Before overview/normal composition resumes, rebuild from the latest
  applied tree; skipping the visual fingerprint must not reuse a stale thumbnail
  or image. This bookkeeping is separate from output damage repair.
- For composed fallback, clear and draw the entire output for every eligible new
  content update, without querying buffer age or traversing repair history. For
  direct scanout, queue the new client framebuffer; omit optional damage clips.
- Keep demand-driven sleep. No new buffer, required cursor/popup update or exposure
  means no draw or flip. Callback-only clients must continue to progress.
- On returning to the normal renderer, invalidate its history and force a complete
  recovery frame before using buffer age again.

The pinned Smithay `DrmCompositor` contains an `OutputDamageTracker` internally;
its scanout flags do not establish that this requirement is met. Use its plane
eligibility and lifetime logic as the reference. Audit the actual hot path, then
build the narrow one-application presenter on the pin's DRM surface/exporter APIs
or make a reviewed upstream change if the needed damage bypass is unavailable.
Do not introduce an unreviewed Smithay fork or claim “no damage tracking” from
`ALLOW_SCANOUT` alone. Preserve atomic validation and buffer-lifetime checks.

### Eligibility and safe fallback

Direct scanout requires the correct visible occupant/output, supported exportable
buffer format/modifier, valid source/destination geometry, compatible transform
and color path, sufficient opaque coverage, and ready synchronization. Validate
the exact plane configuration with an atomic test when its eligibility changes.
Cache only the validated configuration key; do not assume every buffer from the
same application is compatible. Smithay documents that hardware feasibility is
established by an atomic test, with composition as fallback.
[Scanout eligibility](https://smithay.github.io/smithay/smithay/backend/drm/compositor/index.html).

Unsupported SHM/imports, transparency, scaling, a software cursor, popups or modal
dialogs can require a minimal complete-frame composition. Overview, the switcher,
session lock, captures or explicit visible diagnostics interrupt direct scanout
when they need compositor pixels. Fall back on that output promptly and retain
the requested low-latency mode; reenter scanout when it becomes eligible. Never
omit required pixels merely to keep a scanout counter high.

Report requested mode separately from effective `direct-scanout`,
`full-frame-composed` or `normal-overlay` path and a bounded reason code. Normal
HUD/capture diagnostics can affect the path being measured; benchmarks must record
them. Do not equate the existing submitted-frame FPS with physical scanout rate.

### Scheduling and app control

Keep at most one submitted/in-flight flip and one replaceable, ready candidate per
output in Clear's queue. Prefer the newest ready candidate without blocking the
event loop for an unfinished buffer. Account for the displayed buffer separately;
the application/driver may retain more buffers. Superseded commits still require
correct callback, discarded presentation feedback and buffer-release handling.
Respect any advertised client ordering/timing protocol; do not bypass FIFO or
commit-timing promises to implement a mailbox queue.

Use page-flip completion and GPU fence readiness as event sources. Advertise actual
native presentation feedback. Preserve acquire/release synchronization, including
buffer references until scanout has stopped using them. Explicit sync is a native
foundation: the pinned Smithay implementation requires acquire points to be ready
before applying surface transactions, with capability-gated syncobj event sources.
[Smithay explicit sync](https://smithay.github.io/smithay/smithay/wayland/drm_syncobj/index.html).

Allow VRR when the output, mode and user preference support it. Tearing is a
separate user preference: asynchronous flips require the client's committed async
hint, explicit user permission and tested backend/driver support. Otherwise use
supported synchronized presentation without lowering cadence to a refresh divisor.
Do not silently set nested EGL swap interval zero. The tearing hint is permission
the compositor may honor, not client ownership of the display.
[Upstream tearing-control XML](https://gitlab.freedesktop.org/wayland/wayland-protocols/-/blob/main/staging/tearing-control/tearing-control-v1.xml),
[kernel KMS](https://www.kernel.org/doc/html/latest/gpu/drm-kms.html).

Games also need relative pointer motion, pointer lock/confinement, cursor-surface
handling, appropriate keyboard-shortcut inhibition and visible idle inhibition.
Add these protocols with focus/visibility authorization and release on exit,
replacement, unmap, output loss or session deactivation. Keep a compositor escape
binding and session-lock handling available. Content-type `game` is an advisory
mode-selection hint, never authorization for raw input/device/display ownership.
Native Wayland games are the initial compatibility target; XWayland is a separate
dependency for many existing games and is not currently implemented.

### GPU and RAM use

After presentation safely switches, release unneeded output-local animation images,
overview thumbnails, filter scratch, wallpaper/titlebar/HUD GPU caches and repair
history. Keep resources still used on other outputs or by in-flight GPU work.
Refactor currently shared framebuffer caches to support safe trimming before
claiming this saving; clearing logical entries alone may not free scratch capacity.
Allocate fallback render targets lazily and release them after scanout settles.
Do not retain a full-resolution “just in case” screenshot of the game.

Retain small bounded policy/eligibility records and buffers genuinely needed for
current or pending presentation. An unsignalled client fence must not cause Clear
to retain every subsequent candidate. Account for renderer/import caches, DRM
framebuffers and deferred release explicitly. App allocations and host/driver
swapchains are outside Clear's control. Publish measured residency rather than
promising zero memory or the absolute minimum on every GPU. GPU work and RAM can
rise during fallback even while the user-selected mode remains low latency.

## 6. Configuration and commands

The implemented strict mode/rule schema is specified in
[configuration](../specs/configuration.md#fullscreen-modes). The following broader
sketch includes native preferences that remain proposed and are currently rejected:

```toml
[fullscreen]
default_mode = "desktop"

[fullscreen.low_latency]
direct_scanout = true
vrr = "auto"              # auto | off
tearing = "off"           # off | app-requested

[[fullscreen.rules]]
app_id = "example-game"   # exact match; user-controlled policy
mode = "low-latency"
```

Ordinary client fullscreen and the existing `toggle_fullscreen` action resolve
explicit per-request mode, exact app rule, then default. Add an explicit
`toggle_fullscreen_low_latency` action for user override. Do not guess that every
fullscreen browser/video player is a game. Add content-type auto-selection only
as a separately enabled policy after manual/rule selection works.
The explicit low-latency action changes an existing desktop-mode occupant in place;
invoking it again on a low-latency occupant exits fullscreen. The ordinary toggle
continues to exit any current fullscreen mode.

Extend `set_fullscreen` IPC with an optional validated mode for enter operations;
reject a mode on exit and unknown values before any replacement. Keep omitted
mode compatible with ordinary desktop fullscreen by default. Atomic config reload
prepares and validates rules before applying; default/rule changes affect future
entries, while an explicit mode action changes the existing occupant. Rendering
preferences update its effective path without losing return state. Capability
failure downgrades the path, not the requested fullscreen state.

## 7. Delivery stages and separate commits

1. **Core slot and return policy.** Add the module, global exclusivity, replacement,
   dormant/active presentation, topology repair and minimize/transfer rules. Update
   desktop/window-state specifications and replace tests asserting simultaneous
   fullscreen stacking or minimized fullscreen retention. Preserve all mode/geometry
   restoration tests. No backend optimization in this commit.
2. **XDG and desktop integration.** Replace focus/move/set routing, add pre-map
   revision handling and asynchronous configure transitions, update scene/input
   ownership and animated handoffs. Update platform/decorations/animations specs
   and GPU fixtures, including displaced clients holding commits.
3. **Navigation, config and IPC.** Typed special-desktop targets, occupied-only
   overview/cycling, actions/rules, separate snapshot and Quickshell controls.
   Update configuration/input/scripting/overview/shell specs and examples together.
4. **Nested low-latency presenter (implemented).** Separate full-frame path with no damage repair
   or effects, direct commit wakeups, cache trimming and path diagnostics. Test
   idle/callback/recovery behavior and record that host presentation remains.
5. **Native backend foundations.** Feature-gated DRM/GBM/session/libinput outputs,
   lifecycle/hotplug, DMA-BUF import/feedback, explicit sync, presentation feedback
   and game input protocols. Keep nested winit usable. Audit the pin's APIs and
   make any dependency revision change a separate reviewed commit; tearing-control
   integration is not present in the pin's exposed Wayland module inventory.
6. **Native direct presentation.** Minimal validated scanout path, bounded candidate
   queue, hardware cursor, full-frame fallback, resource retirement and recovery.
   Add capability-tested VRR/tearing in separate follow-up commits, with hardware
   evidence for each supported path.

Use the existing module boundaries; avoid a general rendering framework. New
modules need their own `AGENTS.md` if placed in new directories. During each
implementation stage, update the relevant specs alongside code rather than marking
this proposal implemented prematurely.

## 8. Verification and acceptance

For implementation, run Cargo fmt/check/test and editor diagnostics as required
by the root guidance. This documentation change itself requires source/link review,
not builds or compositor runs.

| Layer | Required coverage |
| --- | --- |
| Pure policy | Empty slot unreachable; globally at most one owner; atomic A→B→C replacement; stale exits; per-mode restoration; maximized/floating/tiled preservation; minimized/hidden/pre-map cases; stretched/multiple/headless output repair; config reload; invalid commands leave occupant intact |
| Runtime/shell | Occupied-only special tile and navigation; no duplicate home card; Alt-Tab cancel/accept; no fabricated workspace ID; strict mode requests; displaced/removed target repair; snapshots reflect one final replacement state |
| XDG/GPU fixtures | Browser-like enter/exit; pre-map and stale map intent; held ACK/buffer commits; displaced old fullscreen content restoring; popups/transient dialogs; transparent/undersized content with backing; output choice; panel/input priority; smooth desktop mode and immediate game mode |
| Fast-path instrumentation | Zero ordinary scene/effect/repair calls for the fast output in steady presentation; mixed nested outputs remain correct without shared repair tracking; bounded retained candidates; no idle spin; callbacks progress; tracker recovery after exit; cache capacity released when unused |
| Native synchronization | Unsignalled acquire fences; delayed flips; discarded candidates; correct release after scanout retirement; import/atomic rejection; cursor/popup/overlay fallback; device reset/hotplug/session loss; VRR/async support and fallback |
| Measured performance | Release-mode commit-to-submit and submit-to-present latency distributions, compositor CPU time, GPU passes/bandwidth, and compositor-owned RAM/GPU residency; record refresh, GPU/driver, app queueing, cursor/overlay/capture/HUD state |

Extend [fullscreen core tests](../tests/core.rs),
[action/IPC tests](../tests/fullscreen.rs), [window-state tests](../tests/window_state.rs),
[fullscreen protocol/GPU fixture](../scripts/vm-fullscreen-smoke.py),
[animation fixture](../scripts/vm-animation-smoke.py) and
[idle fixture](../scripts/vm-idle-smoke.py). Keep protocol/GPU cases bounded and
prefer an isolated VM; real physical output scanout/VRR/tearing needs representative
hardware, not only a virtual KWin host. Native hardware validation must also test
physical pointer lock, escape and return to the desktop.

Compare ordinary composition, nested low-latency full redraw and native direct
scanout using the same reproducible workload. Readbacks/FPS HUD are correctness
tools and can inhibit scanout; use presentation feedback and external measurement
for latency acceptance. Do not claim native scanout, minimum RAM or a latency gain
from CPU tests or a nested FPS reading.

## 9. Code and source review anchors

- [Current core flags/commands](../src/core/types.rs) and
  [policy mutation/placement](../src/core/desktop.rs) and [slot ownership](../src/core/fullscreen.rs).
- [XDG requests/commits](../src/platform/smithay/protocols.rs),
  [managed pre-map state](../src/platform/smithay/state.rs),
  [scene/configure/hits](../src/platform/smithay/scene.rs).
- [Backend and damage path](../src/platform/smithay/backend.rs),
  [visual demand](../src/platform/smithay/render_demand.rs),
  [overview targets](../src/runtime/overview.rs),
  [shell snapshots/validation](../src/shell/mod.rs).
- [Pinned Smithay DRM compositor source](https://github.com/Smithay/smithay/blob/e1fb2496c9d7ec4d994cc50fe61b7439686ba8b9/src/backend/drm/compositor/mod.rs):
  scanout eligibility/flags and internal damage tracker.
- [Pinned Smithay syncobj source](https://github.com/Smithay/smithay/blob/e1fb2496c9d7ec4d994cc50fe61b7439686ba8b9/src/wayland/drm_syncobj/mod.rs):
  acquire blockers, capability checks and buffer release.

Upstream documentation explains protocol/hardware constraints; the pinned source
and [Cargo feature selection](../Cargo.toml) determine available integration APIs.
Native features and game protocols listed above are planned dependencies, not
claims about the current Clear build.
