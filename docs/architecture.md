# Architecture

## Policy boundary

Keep Smithay imports under `src/platform/smithay/`. The rest of Clear works with
`WindowId`, `WorkspaceId`, `OutputId`, and integer logical rectangles. A desktop
command mutates policy state and returns explicit effects (`Spawn`, `Close`,
`Quit`); the platform executes those effects.

The flow is:

1. A protocol event or physical input arrives at the adapter.
2. The adapter translates it to a desktop command or typed action.
3. `Runtime` resolves actions, reloads config, or invokes Rhai.
4. `Desktop` computes ordered `Placement`s per visible region.
5. The adapter sends changed XDG configures, maps visible windows, and synchronizes
   focus. Geometry requests and client-committed buffers are not assumed equal.
6. The renderer uses those same placements and clip regions as hit testing.

Window IDs are assigned on toplevel creation. Windows enter the policy model only
when their first buffer maps; null buffers unmap them. Metadata updates do not
reset window ownership. The protocol adapter retains unmapped toplevel handles to
support clients mapping again.

## Workspace presentation

A window belongs to one workspace and has one home output. An output group
presents one workspace. Groups are disjoint, and a workspace is visible in at most
one group. Core tests assert these invariants through deterministic random command
sequences, including output removal and reconnection.

The effective mode is `workspace.output_override` or `workspace.default`.
Scroll positions and remembered focus belong to workspace/output pairs. Floating
rectangles are retained independently of tile rectangles. Floating positions are
unbounded, including offscreen and cross-output rectangles; usable-area changes
do not clamp or replace saved floating geometry. The adapter crops floats and
popups to the union of outputs presenting their workspace, excluding gaps and
independent groups. Tiled bodies additionally respect their requested rectangle;
tiled/scrolling viewports use the panel-reduced usable area. Rendering and pointer
hit-testing share these clips.

The nested backend represents monitors as virtual `wl_output`s inside one winit
window. Configured widths are relative weights during host-window resize; heights
scale relative to the maximum configured height. These are not physical outputs.
The core already accepts arbitrary logical output rectangles, but the current
configuration/backend exposes a horizontal arrangement at scale 1.

## Window state

`Window::maximized` and `Window::minimized` are independent core flags, not layout
modes or client mapping state. Maximize excludes a window from normal layout
inputs and places it in its home output's usable rectangle with an explicit clip.
Normal windows may be focused above a maximized neighbor; launchers remain above
all normal windows. Minimization excludes placements, layout inputs, and ordinary
focus cycling, but preserves workspace order, saved floating geometry, per-mode
sizing, and maximize state. Explicit `Focus` restores a minimized window; merely
switching workspaces does not. Alt-Tab deliberately includes minimized candidates.

The adapter translates XDG maximize/unmaximize/minimize requests into targeted
core commands. A pre-map maximize request is retained until the first buffered
commit, with maximized size/state in the initial configure. Reconciliation owns
XDG Maximized and Suspended state; minimizing removes the window from the rendered
space without marking the client unmapped. Maximize/minimize cancels active drags.
Launchers reject these states; late launcher classification clears them.
The XDG capability list advertises maximize/minimize, not unimplemented fullscreen
or window-menu policy. Optional shell IPC exposes the flags and validated setters;
no toolkit owns the policy. Restore returns to the current layout or unchanged
saved float rectangle, rather than treating a client-committed size as saved state.

## Interactive resizing

`Desktop::begin_resize` captures a backend-independent `ResizeSession` for the
selected edges. `update_resize` takes total displacement from that baseline,
not incremental motion. Floating resizes keep opposite edges fixed and retain
offscreen geometry. Tile resizes update `LayoutSizing`, scoped by workspace,
output, and mode; `management::arrange_with_sizing` consumes those proportions
without changing `LayoutContext` or saved floating rectangles. Mode switches
retain sizing for later reuse; removed-window entries are cleaned up.

Columns and rows resize adjacent pairs. Master/stack exposes its horizontal
split and stack heights. Grid uses row-local column widths and shared row heights.
Scrolling stores per-window width ratios and recomputes content prefixes and
scroll limits. Spiral, monocle, and script tiles reject resizing; floats in those
regions still resize. Launchers remain client-sized and refuse ordinary drags.

Session revisions invalidate gestures after mode, membership, floating/role,
gaps, usable-area, visibility, or output-group changes, including changes that
are later reversed. The adapter retains native client grab/serial authorization,
selects corners for Super+right drag, and ends invalid sessions on motion. Only
move gestures detach tiles; resizing never does. XDG Resizing state brackets an
accepted resize, and asynchronous client content remains clipped as usual.

## Wallpapers

`config::wallpaper` defines optional global and per-output PNG/JPEG selection,
with inherited path/mode fields and paths resolved against the config directory.
This is independent of output topology and of the optional shell. Runtime prepares
a complete bounded, deduplicated set of immutable premultiplied RGBA resources at
startup/reload, never during rendering. A failed reload retains both the previous
configuration and images. Startup resource errors use the solid theme background.

The nested backend owns `WallpaperCache` alongside its renderer. It imports each
shared image once, retains per-output render element identities, and evicts stale
resources after reload. Failed imports are remembered until resources change.
Placement uses full output rectangles rather than panel-reduced usable areas;
fill/fit/stretch/center never span independent outputs. Images render below all
layer-shell backgrounds and have no input region. The theme color remains behind
transparent pixels and fit/center margins.

## Server-side decorations

`platform/smithay/decorations.rs` owns XDG and legacy KDE decoration negotiation.
New/unset XDG preferences and the KDE default select server-side decorations;
explicit client-side requests are honored. XDG takes precedence if both protocols
are used. Without negotiation, applications remain client-decorated. The current
Smithay XDG handler lacks a destruction callback, so `mod.rs` delegates dispatch
normally while forwarding decoration-resource destruction to the adapter.

Negotiated pending mode determines configure sizes, while the mode of the committed
root buffer determines rendering and input coordinates. ACK alone does not change
that mode. Destroying an XDG decoration retains the current rendering decision until
the next root commit. Object replacement and unmap/remap cannot reuse stale mode
configures. Initial configures belong to the bufferless-commit path, never hidden
window reconciliation. KDE mode changes are applied on root commit as well.

`decoration/titlebar.rs` defines backend-neutral `[theme.titlebar]`: four straight
RGBA colors (`active_background`, `inactive_background`, `active_foreground`,
`inactive_foreground`), each exactly four finite channels in `0..=1`; integer
`height` in `16..=128` (default `32`); `controls_side = "left" | "right"` (default
`"right"`); `show_icon` (default `false`); and `show_title` (default `true`). Defaults
use opaque dark backgrounds (RGB 35/40/52 active, 27/30/38 inactive) and light
foregrounds (239/243/250 active, 174/183/199 inactive), expressed as channels divided
by 255. `[theme.titlebar.controls]` accepts optional `minimize`, `maximize`, `restore`,
and `close` SVG paths. Config resolves relative paths against its own directory,
without shell expansion. Omitted controls use built-in glyphs; omitted `restore`
first falls back to a supplied `maximize` SVG. SVGs keep their own colors/alpha.

Core rectangles remain backend-independent frame rectangles: the adapter insets
SSD content by the configured height, retaining at least one content row in tiny
frames. Normal configure sizes subtract that inset; launcher committed dimensions
add it before core centering, keeping client-owned content sizes unchanged. Client
surface, input, and popup origins all use the same inset and XDG geometry offset.
Changing decoration mode or reloading height never rewrites saved floating rectangles
or tile sizing. Popups and layer-shell trees bypass titlebars.

`runtime/titlebar.rs` prepares immutable premultiplied CPU images outside rendering.
Explicit control SVGs are prepared as one candidate set at startup/reload; startup
resource failure warns and falls back to built-in controls. Invalid startup config
uses safe defaults. Reload rereads same-path files, prepares bindings/scripts,
wallpapers, and titlebar resources before publishing any of them, and retains the
entire last-good configuration/resources on failure. App icons are prepared on
classification/reload; render accessors never perform filesystem IO or SVG decoding.

SVGs are local regular files, at most 256 KiB, parsed without DTD/entity expansion,
with 4096 XML nodes and 32 nesting levels at most. Source dimensions are limited to
1024 pixels per axis. Minimal `resvg` has default features disabled: no SVGZ, SVG
text/font loading, or embedded raster decoders; both image resolvers are disabled,
so neither embedded images nor external/network resources load. The restrictive
subset supports simple paths and gradients, not masks, clips, or filters. `use`,
`pattern`, `marker`, `mask`, `clipPath`, and `filter` elements are rejected by local
name before usvg conversion, including unused/namespaced definitions, so CSS/href
references cannot bypass the check. Icons are aspect-fitted to transparent square boxes of
`clamp(height - 12, 1, 20)` pixels, independent of window width.

App icons use exact desktop-file IDs under bounded XDG data roots and the entry's
`Icon` key: absolute SVG/PNG/JPEG paths, or named icons in fixed `hicolor` size/app
locations, direct icon directories, and `pixmaps`, including legacy `~/.icons`.
There is no recursive/fuzzy lookup, theme inheritance, or `Exec`/`TryExec` execution;
this is not a full icon-theme resolver. Higher-priority desktop entries shadow lower
ones, even if hidden or lacking a usable icon. Missing/invalid icons use a generic
glyph. Positive and negative results share a 256-entry cache reset on reload, with
at most eight data roots, 256 probes per lookup, 4096 total probes and 16 MiB read
budget per asset set. Desktop entries are limited to 64 KiB; PNG/JPEG icons to 2 MiB
encoded, 1024 pixels per axis, and 8 MiB decoded. Unsupported resources or exhausted
lookup budgets fall back without rejecting otherwise valid configuration.

`platform/smithay/titlebar.rs` owns shared hit boxes, Unicode rasterization, and
renderer-local textures. Right-side controls read minimize, maximize/restore, close;
left-side controls mirror to close, maximize/restore, minimize. Narrow frames
prioritize close, then maximize, then minimize. Rendering and hits derive from the
same original-frame layout, not output crops. System font discovery/reads occur at
cache initialization. Bounded title strings (1024 bytes) are shaped with cosmic-text,
cropped away from icons/controls, and rasterized only for visible strips, at most
4096 pixels wide and 128 high. Cache keys include title, local clip, size, activation,
maximization, full style, app ID, and prepared-image identity (including same-path
replacements), not absolute position. Both a 64 MiB texture-byte budget and a
128-entry limit apply, excluding outstanding render elements and driver overhead.

Theme colors are premultiplied once; glyphs and already-premultiplied icon images
use source-over for all four channels. There is no forced opaque fill, accent stripe,
or separator. Rounded rendering composites titlebar and client body before applying
a single shared outline. Its shader preserves sampled alpha instead of forcing it
to one, adds disjoint premultiplied body/ring coverage, and reports no opaque region
for the combined element. Translucent titlebars reveal the lower scene (filtered
when backdrop blur is enabled) without fading client content or double-premultiplying
SVG pixels. Square SSD borders use only a ring even with blur disabled, preventing
border-colored backing from showing through translucent or fully transparent
titlebars. Legacy square, unblurred CSD backing is unchanged.

SSD hits are compositor-owned, never Wayland pointer focus. Input suppresses matching
button releases and activates a control only when released over the same window/control.
A titlebar focus click arms movement without detaching a tile; only crossing the
movement threshold starts a drag. Super+mouse gestures take precedence over controls.
Window buttons target existing core state commands or send a graceful XDG close;
focus, minimize, maximize, and launcher restrictions remain core policy.

## Rounded window outlines

`decoration::CornerRadii` stores validated outer radii, clockwise from top-left.
Configuration accepts a scalar or one/two/four values; two expands as top/bottom.
The default zero keeps the previous square rendering path. No core placements,
saved geometry, layout sizing, or client configure dimensions change.

The Smithay adapter's `rounded.rs` derives an original, unclipped `WindowOutline`
from each placement. Adjacent radius sums are proportionally fitted to its size;
inner radii subtract border thickness. Rendering and body hit-testing intersect
all applicable corner arcs, including overlapping opposite-corner regions.
Output/workspace crops never introduce new rounded corners. Cut-outs pass input
to underlying surfaces; popups and layer-shell surfaces bypass these masks.

The nested backend lazily owns a compiled `RoundedShaders` texture program.
For each visible clip, the adapter composites the unmasked body surface tree into
an RGBA offscreen texture, then applies coverage once. This prevents overlapping
subsurfaces from accumulating coverage or leaking obscured parent colors at the
edge. Body coverage and border-ring coverage contribute premultiplied colors to
one result, preserving translucent clients and borders without a colored seam.
The combined element reports no opaque regions, so cut-outs reveal underlying
content; fresh element identities damage the entire clip, including theme-only
changes. Offscreen allocation is bounded by the visible clip, but currently occurs
per frame; texture reuse and damage optimization remain future work.

Shader coordinates use the original desktop shape and framebuffer height. This
path is specific to the current scale-1 nested backend's single framebuffer;
a native or scaled backend must adapt those coordinates rather than reuse them
unchanged. Reload changes theme data/uniforms, not shader ownership.

## Backdrop blur

`Theme` describes one global backdrop filter for all windows, popup trees, and
layer-shell categories. `blur_method` selects `gaussian` (default) or `kawase`
(Dual Kawase). `blur_radius` is finite in `0..=32`, default zero, which bypasses the
blur renderer entirely. Gaussian interprets it as logical-pixel kernel support;
Kawase uses it as a sample offset in source pyramid texels. `blur_passes` is an
integer in `1..=6`, default `3`; Gaussian ignores it but validation always applies,
including when blur is disabled. Runtime reload remains atomic; neither method
changes geometry, protocols, or client opacity, or adds a glass treatment.

Scene assembly retains front-to-back elements plus ranges grouping each logical
window/surface tree and its viewport. The backend-owned `BackdropBlur` consumes these
groups bottom-to-top: render the foreground tree unfiltered, blur only the already
composited scene with the selected filter, then resolve the group. Subsurfaces
are composed before backdrop replacement, so overlapping client surfaces do not
repeatedly blur their parent. Popup trees sample their parent and all lower groups,
never surfaces above them. Wallpaper groups are ordinary unfiltered scene content.

With premultiplied foreground `F`, composed alpha `A`, and independent rounded frame
coverage `C`, the result is `F + (C-A) * blurred + (1-C) * original`. Keeping coverage
separate from alpha preserves rounded holes and sharp opaque content. Square borders
are rings when blur is enabled or the window uses SSD, rather than backing behind
transparent content. Square SSD keeps this ring at radius zero to honor titlebar
alpha; only square unblurred CSD retains the legacy backing. Client-shaped
layers/popups lack a separate shape region, so their blur contribution is `A*(1-A)`,
leaving fully transparent holes unchanged.

Gaussian uses two separable passes, sigma `max(radius/3, 0.5)`, and at most 65
samples per pass. Filtering is bounded to the tree plus its required vertical halo;
sampling clamps to the output/workspace viewport's texel centers.

Dual Kawase first copies the whole output/workspace viewport into an isolated
level-zero texture, not just the tree bounds. It ceil-halves each dimension for up
to `blur_passes` downsample levels, stopping only when both dimensions reach `1×1`,
then upsamples through the same levels in reverse to the original viewport size.
Downsampling uses a center tap of weight 4 plus four diagonal taps of weight 1
(normalized by 8); upsampling uses four axial taps at twice the offset of weight 1
plus four diagonal taps of weight 2 (normalized by 12). Offsets are measured in
**source-level texels**, without an implicit half-texel offset. Center-aligned linear
sampling uses actual source/destination size ratios for odd dimensions, clamps to
each level's texel centers, and stores RGBA8 intermediates. This isolation prevents
sampling a neighboring independent output. The VM example uses radius `2`, depth `3`;
these values are not equivalent to Gaussian radius `2`.

Declared-opaque groups can skip filtering. Both methods reuse four framebuffer-sized
RGBA8 scratch textures (about 16 bytes per host framebuffer pixel). Kawase additionally
retains an LRU keyed by viewport size and effective depth, not origin or radius:
at most four pyramids, with a combined budget of three framebuffer areas of texels
**including level zero** (up to another 12 bytes per host pixel). Thus retained blur
texture storage is bounded by seven framebuffer areas, about 28 bytes per host pixel,
excluding driver overhead and other renderer resources; this is not a peak-allocation
guarantee during replacement. Upsampling overwrites downsample levels, and every
filtered tree refreshes level zero: the cache reuses allocations, not stale scene
pixels. A framebuffer-size change replaces scratch storage and clears pyramids on
the next blur render. Changing method or disabling blur does not immediately free
previous allocations; while disabled, they retain the last blur-rendered size.
Shaders are initialized lazily per method.

All read/write attachments are separate, auxiliary sampler state is restored, and
rendering explicitly restores the window framebuffer before presentation/capture.
The current path assumes the nested backend's scale-one, single framebuffer.

## Shell policy and protocol ownership

`src/config/shell.rs` contains backend-independent `ShellConfig`, `PanelRule`, and
`PanelLayer` descriptions. The default `[shell]` uses `launcher_app_ids = ["wofi"]`
and one `[[shell.panels]]` rule with `namespace = "waybar"`, `layer = "top"`.
App IDs and namespaces match exactly and case-sensitively. Supplied arrays replace
their respective defaults, and empty arrays disable those rules. Empty/whitespace-only
or duplicate names are rejected, as are unknown fields and invalid layers. Explicit
panel entries require both namespace and layer; accepted layers are `background`,
`bottom`, `top`, and `overlay`. These declarations neither autostart processes nor
provide launcher/panel toggle actions.

`Runtime::classify_window` translates XDG app-ID rules into core `WindowRole`s.
The adapter updates metadata and classifies on mapping and late app-ID changes;
a successful runtime reload reclassifies all managed windows, including hidden
ones. Rejected reloads retain the previous rules and roles. Classification is
independent of workspace mode and the per-window floating flag, and does not
change window ownership, order, focus, or saved floating geometry.

Launcher-role windows bypass built-in and scripted layouts, so they consume no
tiles or scrolling content. Core placement centers them in their home output's
usable area using the last positive client-committed size, falling back to saved
floating dimensions before a valid size is known. A later commit updates their
size and center without replacing saved floating geometry. Oversized launchers
retain their client size rather than being fitted to the output. Placement order
is normal tiles, normal floats, then launchers across outputs; top/overlay
layer-shell surfaces remain above application windows.

Layer-shell objects and reservations belong exclusively to the Smithay adapter.
Namespace rules override only the client's layer, never anchors, margins, sizing,
or exclusive zones. Unmatched clients keep their requested layer. The adapter
arranges layers per output and passes the resulting usable rectangle to
`Desktop::set_output_area`. Panel reservation changes and removal must recompute
tile geometry and launcher centers without altering any saved float rectangle.
Core regression tests exercise this usable-area boundary; they do not simulate
Wayland layer commits or keyboard delivery. Layer-shell launchers are distinct
from app-ID-classified XDG windows. Layer keyboard focus stays in the adapter,
rather than being modeled as workspace-owned application windows. Exclusive
mapped overlay/top layers preempt application focus; on-demand layers take focus
on click. Render and hit-test order is overlay, top, application placements,
bottom, background, wallpaper, with layer popups included. An input-region miss continues
to the next surface, and a layer hit never starts an application drag.

Initial bufferless layer configuration is temporarily arranged without reserving
desktop space. Null-buffer unmaps release reservations and keyboard ownership;
a later bufferless commit begins a fresh mapping handshake. Layer policy changes
only the effective current layer: pending client state stays untouched until
commit. VM fixtures exercise these lifecycle transitions, protocol keyboard
ownership, layer ordering, popup rendering, and client-sized launcher centering.

## Extending modes

A built-in policy is a pure function over `LayoutContext`. Add a mode to `Mode`
and implement its geometry in `management`; no Wayland handler changes should be
necessary. Placement order is back-to-front. Scroll policies can return offscreen
rectangles with a clip, while floating policies retain their saved geometry.

A Rhai policy receives IDs and copied geometry and returns one bounded placement
per input window. The scripting host validates membership, integer geometry, and
resource limits. Runtime disables failing functions until reload; core has a
second validation boundary and built-in fallback. Rhai does not own compositor
state, render surfaces, or run directly from the paint loop.

## Optional desktop-shell boundary

`src/shell/` provides an owned, serializable view of desktop state and a small
validated command allowlist. A bounded nonblocking Unix transport serves that
model; `platform/smithay/shell.rs` polls it on event-loop turns and reconciles
successful commands before responding. State changes are coalesced snapshots,
not an event history. Window/workspace/output IDs remain strings on the wire.

Shell windows use standard layer-shell. Quickshell is one optional example
client under `examples/quickshell/`, not a dependency, mandatory process, or
owner of desktop policy. Clear works without it, and `--no-shell-ipc` disables
the bridge. Each instance exports its own private `CLEAR_SOCKET` to children;
never leak a host compositor's endpoint into a nested instance with IPC disabled.

A future native shell can use the same model and commands without adopting QML.
Notification/tray services stay outside the compositor. Alt-Tab selection and
accept/cancel policy remain in Clear; the shell view presents that state.
See [shell-integration.md](shell-integration.md) for
current capabilities, limitations, socket permissions, and tests.

## Implementation boundaries still to grow

- Current built-ins keep state in the desktop model; extensible user-defined mode
  state and a rich decoration scene API are not yet exposed.
- Native DRM/KMS needs a separate backend and additional Smithay features. Preserve
  the core/runtime boundary when adding it.
- Current resizing is asynchronous without global all-client transactions. A
  delayed client cannot block the desktop; its old tiled content is clipped.
- Rendering currently repaints continuously with buffer age zero for correctness.
  Event-driven damage scheduling is a later optimization.
- Activation, full cursor-surface handling, and fullscreen semantics need their
  own protocol/policy work rather than shortcuts in the
  renderer. Physical pointer/keyboard gestures still need interactive testing;
  the automated VM fixture checks protocol ownership and framebuffer output.
