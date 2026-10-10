# Desktop overview

## Ownership and entry

Clear MUST own overview selection, geometry, rendering and input. It is not a
managed XDG/layer-shell surface and MUST work without shell IPC or a companion
shell. The typed `toggle_overview` action requests entry/exit; its default is
`leader+w`. An optional [shell request](shell.md) uses the same authorization path.
Rhai may return this declarative action.

Entry cancels Alt-Tab without committing it. Entry MUST wait while a client
keyboard/pointer grab, compositor drag/titlebar interaction, held nonmodifier
client key, intercepted pointer button, or mapped exclusive top/overlay layer
owns input, or the nested host is unfocused. A second toggle cancels a pending
entry. Escape cancels a pending entry as well as an open overview. Acknowledging
an IPC toggle accepts the request; it does not guarantee immediate entry.
On-demand layer focus does not block entry. Opening/closing uses the configured
[overview animation](animations.md) when global/effect motion is enabled.
Disabled or reduced-motion preferences retain the instant path.

## Presentation transitions

Entry interpolates live visible window frames from their last submitted desktop
poses into the fitted overview card positions. Without an active desktop pose,
the adapter uses the reconciled frame origin and committed body dimensions.
Hidden/minimized candidates fade into their card positions without synthesizing
hidden-workspace layouts. The wallpaper canvas smoothly scales from full output
bounds into its overview inset while retaining the full-output crop/letterboxing;
labels, miniature previews, selection accents and outer dimming fade with the
same sampled progress. No per-frame scripts, policy placements or client
configures drive these transitions.

Exit retains an ID-only session and bounded layout after runtime selection has
closed or activated. Cards returning to visible desktop placements move/scale
toward their reconciled origins with committed body dimensions; remaining hidden
or minimized cards fade away. Wallpaper returns to full output bounds. Cancelling
during entry reverses from the last submitted progress, retaining the shared
effect's analytic spring velocity. Changing animation speed rebases the same
clock rather than restarting the scene. Ordinary pagination/navigation changes
are not separate animated layout transitions.

The adapter samples the shared capped animation timestamp before rendering;
prediction does not advance pointer hits. Rendering consumes the sampled layout,
and hit discovery consumes the last successfully submitted layout. Duplicate
ordinary window visuals are suppressed while overview presents; the GPU preview
remains the live representation. Overview continues owning input, callbacks,
preview output membership and temporary unsuspension through its closing
terminal frame, even though the optional shell snapshot already reports runtime
`overview_open = false`. Closing consumes further input until handoff; the
overview toggle can reopen from the submitted progress. Only a
successfully submitted current terminal frame retires this presentation session;
failed rendering/submission cannot restore client input over a still-visible
overview. Host focus loss clears the transition immediately. Output topology
changes refresh the bounded output-local geometry and existing cache cleanup;
loss or resizing of a closing scene's output clears that transition immediately.

One shared engine track captures the effect law, using the ordinary animation
speed/reduced-motion policy. Transition bookkeeping retains at most twelve
desktop frame endpoints and opacity values, and four bounded layouts (target,
transition source, sampled and submitted), each with at most 24 items, plus the
existing ID-only runtime session. Texture/label bounds below continue applying throughout entry
and closing; no separate overview screenshot or retained GPU texture pool is
added.

## Preview and activation

One ID-only runtime session records its output, original focus, selected
workspace/target, workspace order and normal window order. Opening previews the
workspace presented on the focused output and selects its focused normal window
when eligible, otherwise its workspace tile. Normal mapped windows, including
minimized windows, are candidates; launcher roles are excluded.

Preview MUST NOT mutate desktop focus, group presentation, saved geometry,
minimize/maximize flags, modes or layout proportions. It MUST NOT compute hidden
workspace placements or send thumbnail-sized configures. Ordinary lifecycle and
independent desktop commands still apply while preview is open.

Activation of a window MUST use explicit core `Focus(window)`, including hidden
workspace reveal and minimized-window restoration. Workspace activation focuses
the session output and uses `SwitchWorkspace`, including ordinary group swaps.
Escape or another toggle closes without committing preview. Existing desktop
focus repair handles disappearance of the original window; cancellation MUST NOT
resurrect it. New/removed/reclassified/transferred windows refresh candidate order;
a removed selected card falls to the next candidate at its former index, then the
last candidate, or the workspace tile when empty. Workspace previews keep their
identity even when external commands change presentation. Output loss transfers
the session to the first surviving output and keeps the preview workspace; losing
all outputs closes it without deleting desktop windows.

## Layout and input

The interactive output has a compact, centered desktop-preview strip above an
inset wallpaper canvas. Normal windows are spread across that canvas in a grid
of at most four columns and three rows, with incomplete rows centered. Visible
frames use one common downscale factor to preserve their relative committed
sizes; compact title/icon captions sit beneath them. The selected frame has an
accent outline, without a large opaque card background. Grid/strip pagination
keeps the keyboard selection visible;
all candidates remain navigable. Workspace identities and window cards occur
once, including across stretched groups. Other outputs are dimmed and consume
input. A click on another output moves the interactive UI there and previews
that output's presented workspace without changing desktop focus.

| Input | Operation |
| --- | --- |
| Tab / Shift+Tab | Cycle workspace tile and every card, forward/backward |
| Left / Right | Previous/next card, or workspace while the tile is selected |
| Down | Enter the card grid or move one row down, stopping at the last card |
| Up | Move one row up, or return to the workspace tile from the first row |
| Ctrl+Left / Ctrl+Right | Previous/next workspace from either selection kind |
| Wheel | Previous/next workspace |
| Pointer hover | Select the hit card; workspace preview is opt-in |
| Left press/release on the same tile/card | Activate |
| Enter | Activate current selection |
| Escape / overview toggle binding | Cancel |

Workspace hover MUST leave both the large preview and selection unchanged by
default. The strict boolean `[overview] preview_workspace_on_hover` (default
`false`) enables workspace preview on hover; it does not switch desktop policy
until activation. Successful reload applies the preference to an open overview;
invalid reload retains the last-good preference and session. Window-card hover
still selects a card. Clicking a workspace tile activates it and closes overview
regardless of the hover preference.

Left press on a window card arms a transfer drag. Movement of at least 8 logical
pixels starts it; smaller movement retains ordinary click activation. During the
gesture, workspace/card hover and wheel navigation MUST NOT change the source
preview. Active drags also consume keyboard navigation/activation; Escape or the
overview toggle cancels the gesture and closes overview. During pickup, the
original card body/caption MUST be replaced by one live drag image starting at
the card's last submitted rectangle when the threshold is crossed, preserving
the grabbed fraction even if overview entry moved the card after the press;
it MUST NOT leave a stationary duplicate in the large canvas. The configured
overview effect interpolates pickup and subsequent
pointer/size retargets from the last successfully submitted ghost.
Continuous pointer updates MUST allow the transition to advance between submitted
frames rather than restarting at zero progress each refresh. Global speed,
sampling caps and reduced-motion/effect-disable settings apply to this track.
Reduced motion, disabled motion or exhausted track capacity uses the target
immediately. The grabbed fractional point MUST determine the destination anchor,
subject to keeping the full image inside the interactive output.

Away from the desktop strip, the target fits within 240×160 pixels without
upscaling the source card. Pickup may temporarily retain the larger card size.
Within an approach band around the nearest visible workspace tile, the target
smoothly shrinks toward the fitted dimensions of that window's remembered frame
in the miniature. Distance uses the tile's complete hit box (including its label);
the band is twice its preview height, clamped to 64–160 logical pixels, with
smoothstep interpolation. Inside the tile the target uses miniature dimensions,
without enlarging the ordinary drag size; moving away reverses the scaling.
Mapping uses the source window's home output and the same frame hints/cropping as
ordinary desktop miniatures, never a hidden layout computation. An unavailable
or entirely off-output hint falls back to fitting the image within the tile.
The eligible workspace tile remains outlined independently of ghost motion.
A drop on another workspace tile
moves that specific window through `MoveWindowToWorkspace`, without focusing,
revealing, restoring or detaching it first. The transfer preserves saved floating
geometry and floating/maximized/minimized flags. Overview stays open on the source
workspace; candidate/focus repair follows the ordinary lifecycle rules. The
destination may already be visible on another output; the drop MUST NOT swap
workspace presentations or follow the moved window.
Closing/cancelling overview after a completed drop MUST NOT undo that transfer.
Drag destinations are the currently visible strip tiles; drag-driven edge paging
is not implemented.

Release outside a workspace tile or on the source tile cancels the transfer and
MUST NOT activate anything. Source disappearance, launcher reclassification,
external workspace transfer, changed interactive output identity/geometry, host
focus loss and overview exit cancel the gesture. Matching releases remain
suppressed after cancellation. Every drop/cancellation MUST release the one
optional drag animation track; failed submission MUST NOT update its shown pose.
Pressed workspace tiles also freeze pointer-hover
preview until release, preserving same-target click activation.

Rendering and hits MUST consume the same deterministic layout, with half-open
boxes clipped to physical/virtual output bounds, independent of panel usable
areas. Empty-workspace tiles remain selectable. Outside-card events MUST NOT
reach covered clients or layers. While open, protocol keyboard and pointer focus
leave clients; closing reconciles them through ordinary layer/application rules.
Intercepted presses suppress matching releases by physical key/button code,
including after overview closes. Previously forwarded leader modifiers release
through the seat with no focused client while overview is open, keeping its held
key bookkeeping accurate. Modifier changes on intercepted events MUST still be
advertised to the restored protocol keyboard owner without forwarding an
unmatched raw key release. Other shortcuts and compositor pointer gestures do
not execute while overview owns input. Host focus loss cancels overview and any
armed card click.

## Rendering and resource bounds

Overview is the final compositor pass above wallpapers, applications, popups and
all ordinary layers, after any desktop blur/glass composition. Cards themselves
MUST NOT invoke optical filtering or recursively capture the overview. No public
screencopy protocol, CPU window readback, image encoding or shell thumbnail
transport is used.

Visible cards, desktop miniatures and drag ghosts MUST remain live: clients
continue receiving presentation opportunities while their previews are visible.
They composite latest committed toplevel and subsurface trees on the
GPU through the [shared committed image composer](rendering.md#committed-window-images),
cropping content to its committed XDG geometry and adding committed SSD
appearance with the current theme. Transparent content is composited over the
wallpaper canvas (or an opaque theme background when no wallpaper is available).
The compositor MUST first combine client content, SSD, desktop borders and the
original fitted corner mask into one source image, then scale that complete image
for main cards, desktop miniatures and drag ghosts. Corner radii and border insets
scale proportionally with the window; preserve asymmetric and zero-radius corners. The compositor
MUST NOT apply a new fixed-pixel radius to the resized client image. The separate
selection/drag accent follows the scaled source silhouette, with an inset of at
least two pixels when the preview can contain it; it MUST NOT mask the client
image a second time. Client popups, launcher/shell UI and other windows are
excluded. Metadata labels use the prepared titlebar font resources,
a bounded title/app-ID fallback, prepared icons when available, and a minimized
indicator before the title (so truncation retains it). Icon availability follows [decoration preparation](decorations.md);
overview does not initiate filesystem lookup from rendering. Missing buffers or
thumbnail rendering failure retain a selectable metadata card with a rectangular
selection outline; a missing drag image uses the same visible outline fallback.
Every destination MUST sample the entire bounded thumbnail texture; scaling
MUST NOT silently crop to the destination's pixel dimensions.

Desktop miniatures reuse the interactive output's wallpaper and show at most
four nonminimized normal windows in stable workspace order. They use remembered
frame hints, falling back to saved floating geometry, mapped from each window's
home output and bounded to the miniature. They MUST NOT compute hidden-workspace
placements. They are representative previews, not complete desktop captures;
panels, popups and launchers remain excluded. Only windows actually represented
in visible miniature tiles are driven by overview callbacks. A card and its miniature share one bounded committed-tree
texture rather than allocating a texture for each destination size.

The canvas, miniature desktops and dimmed outer backdrop reuse renderer-owned
wallpaper textures. Preview placement MUST scale the full-output wallpaper crop
and letterboxing, including `center` and `fit`, rather than recomputing a crop for
the small destination. The outer backdrop covers normal windows and layers with
wallpaper/theme background before dimming it. Other outputs show that dimmed
backdrop; only the interactive output shows the strip and window canvas.

The renderer-local thumbnail cache MUST retain at most 128 RGBA8 textures and
64 MiB of texture storage, each at most 2048×2048 pixels. Before compositing,
choose each window's texture dimensions from its largest visible card, miniature
or ghost, preserving source aspect ratio and never exceeding the committed source
resolution. Cards within the texture limit MUST NOT enlarge an intermediate
low-resolution capture. Compose and mask the source at committed desktop
resolution (aspect-fitted to the same 2048×2048 bound for oversized sources), then
reduce the complete source image to the largest preview's pixel dimensions.
Smaller destinations share that texture. Visible selected-workspace cards update
on tree commits at most once per presented frame. Hidden pages may
retain still textures, evicting least recently used entries under pressure.
Overview-only visible clients receive throttled frame callbacks (33 ms), including
windows with offscreen ordinary placements; ordinary scene clients retain their
existing callback policy without duplicate preview callbacks. Visible preview trees
receive output enter/leave notifications for the interactive output, restricted
to committed XDG geometry and excluding popups. Minimized windows represented by
visible cards MUST be temporarily unsuspended at the protocol level, while their
core minimized flag, placement and input ownership remain unchanged. Removing a
preview or closing overview restores ordinary output membership and suspension.
Other hidden workspaces/pages MUST NOT be driven by overview callbacks unless
represented in the bounded visible miniature list. Preview output tracking retains
at most 61 window handles (12 cards, 48 miniature candidates and one ghost,
deduplicated by window ID), released as those previews disappear. Labels retain at most 128 textures and
4 MiB, each at most 1024×32 pixels. Label text is bounded to 256 characters and
uses the titlebar's single-line sanitization. Source preparation uses the shared
scratch allocations bounded in [rendering](rendering.md#committed-window-images),
separate from the 64 MiB preview cache; overview does not retain another scratch
pool. Source composition, initialized-region clearing and full-texture sampling
follow that shared image contract.
Existing titlebar/client resources, prepared icons, driver overhead and outstanding
render elements are additional to these retained texture bounds.

Committed fullscreen mask changes MUST invalidate a thumbnail even when source
dimensions and client buffer damage are unchanged. Fullscreen preview sources
suppress compositor titlebars, borders and rounding as in [decorations](decorations.md).
Unmap/destruction, changed thumbnail size, output geometry/identity changes,
theme changes, renderer destruction, overview exit and shutdown release affected
thumbnail resources. Successful reload rebuilds restyled resources without
resetting desktop geometry; failed reload retains last-good configuration.
The current backend uses a single logical-scale-1 nested framebuffer; native
output scaling, session-lock priority, accessibility export, search, window reordering,
hot corners and touchpad progress remain future work.

## Implementation and evidence

- [Runtime session](../src/runtime/overview.rs) and [policy tests](../tests/overview.rs):
  entry/cancellation, navigation, preview purity, hidden/minimized activation,
  group swaps, launcher exclusion, candidate and output removal, strict hover
  configuration and targeted drops preserving focus and saved window flags.
- [Adapter layout/render cache and unit tests](../src/platform/smithay/overview.rs):
  shared hit boxes, odd origins, pagination, tiny outputs, relative frame sizes,
  bounded miniature geometry, destination-driven texture dimensions, and source
  resolution/texture-limit clamps. [Wallpaper placement tests](../src/platform/smithay/wallpaper.rs)
  cover preservation of full-output crops and letterboxing in previews.
  Shared source composition belongs to
  [window_image.rs](../src/platform/smithay/window_image.rs); CPU geometry/cache
  tests do not establish GPU alpha, mask, snapshot lifetime or crop correctness.
- [Presentation transition and pure tests](../src/platform/smithay/overview_animation.rs):
  full-output/card endpoints and midpoint geometry, reversal from a submitted
  phase across speed reload, interrupted activation rebased to new desktop
  endpoints without a first-frame jump, outer-border endpoint geometry, and
  terminal-frame/current-revision retirement.
  These CPU tests do not establish frame cadence, compositor input handoff or
  GPU intermediate pixels.
- [Drag presentation and unit tests](../src/platform/smithay/overview_drag.rs):
  pickup starts at the card, retargets start at submitted geometry, proximity
  uses the same fitted miniature dimensions, moving away reverses sizing, output
  bounds/grab anchors hold, continuous pointer updates advance motion, entry-time
  pickup uses the latest shown card, and speed/reduced-motion changes preserve cleanup.
  These CPU checks do not establish GPU drag pixels or physical pointer behavior.
- [Input routing and seat-path test](../src/platform/smithay/input.rs):
  `overview_keyboard_pairs_and_deferred_entry_without_shell_ipc` exercises
  synthetic keyboard/button pairs and implicit-grab deferral without a shell.
  These are compositor seat tests, not physical device tests.
  `overview_hover_click_and_drag_without_shell_ipc` checks default/opt-in hover,
  workspace clicks, threshold/drop/cancel behavior, frozen source preview,
  independent-transfer invalidation and matching releases.
- [Input routing](../src/platform/smithay/input.rs),
  [focus reconciliation/callbacks](../src/platform/smithay/scene.rs), and
  [final rendering pass](../src/platform/smithay/backend.rs).
- [Protocol/GPU fixture](../scripts/vm-overview-smoke.py): real clients, size-hint
  stability, focus ownership, layer coverage, minimized buffers, cancellation,
  deferred exclusive layers, hidden live updates, alpha/subsurfaces, SSD and odd
  output boundaries, plus a rounded-thumbnail pass above a blurred desktop and
  wallpaper-backed transparency, desktop miniatures, layer coverage and
  bottom-corner content that detects accidental texture cropping. `sharp-preview`
  preserves one-pixel client content and SSD control stripes at native preview
  size, detecting low-resolution capture/upscale blur. `rounded-previews` checks
  proportional source corners in both main and miniature SSD destinations.
  `rounded-asymmetric` checks rounded/square corner order and source-buffer
  orientation. Callback-driven
  animation cases verify continually changing GPU pixels for normal, minimized,
  offscreen, miniature-only windows and subsurfaces; clients honor protocol
  suspension and output membership. Exit/reentry checks preview-only pause/resume.
  These checks do not synthesize physical clicks or keys. Validation records and
  bounded commands belong to the [testing guide](../docs/vm-testing.md).
