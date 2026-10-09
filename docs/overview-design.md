# Compositor-rendered overview — design proposal

**Status:** selection, configurable workspace hover and window-to-workspace drops
are implemented; animation, search and
gesture progress remain proposed. The [overview specification](../specs/overview.md)
owns implemented behavior. The remainder records the design direction, including
future slices, rather than adding contracts. The user-facing
reference is [Plasma's overview](https://github.com/KDE/kwin/tree/master/src/plugins/overview),
with [Niri's overview](https://niri-wm.github.io/niri/Overview.html) as a reference
for workspace navigation. Clear should implement its own UI rather than copy
either compositor's workspace model.

## Decision and scope

Clear owns the entire overview: activation, transient selection, layout,
rendering, animation, hit-testing, and input. Quickshell or another shell may
offer a button that requests it, but does not draw it, hold its state, or supply
window images. The shortcut works with `--no-shell-ipc` and when no companion
shell is running. No public screencopy protocol is needed for this feature.

The first complete interaction shows workspace choices and live window cards on
the current output, supports keyboard and pointer selection, and closes by
choosing a window/workspace or pressing Escape. Dragging a window to a workspace
thumbnail is now implemented. Later slices add search and touchpad-driven progress. The rendering
and state model should accommodate those additions without changing desktop
ownership or using a shell-specific protocol. Future hot corners and gestures
enter through the same compositor action as the keyboard shortcut.

## Ownership

| Owner | Proposed responsibility |
| --- | --- |
| `src/core/` | Continue to own authoritative window/workspace IDs, groups, focus, and commands. Supply read-only candidate/order queries if existing iterators are insufficient. Preview must never change saved geometry or group presentation. |
| `src/runtime/` | Own a short-lived `OverviewSession`, analogous to the existing Alt+Tab switcher: initiating output, original focus, selected workspace/window, and open/closing phase. Apply a validated selection through existing core commands only when committed. |
| `src/input/` and `src/config/` | Define a typed toggle action and validated binding/configuration. The candidate default binding is `leader+w`; the exact default belongs in the implementation specification. |
| `src/platform/smithay/overview.rs` | Build compositor UI geometry and hit regions from one immutable view model per frame; own GPU thumbnail/font resources and render elements. No Qt/Quickshell types enter this module. |
| `src/platform/smithay/input.rs` and `backend.rs` | Translate physical input, manage seat focus and suppression, draw the overview as the final compositor-owned pass, and schedule frames while it changes. |
| Optional shell IPC | May gain only a validated `toggle_overview` request and read-only open state for a panel button. The overview does not depend on the socket being enabled. |

This follows the present split: [core commands](../src/core/types.rs),
[runtime switcher state](../src/runtime/mod.rs), [scene composition](../src/platform/smithay/scene.rs),
and [physical input routing](../src/platform/smithay/input.rs). It does not turn
the overview into a managed XDG window or a layer-shell client.

## View model and layout

On entry, take an ID-based view of workspaces, outputs/groups, mapped normal
windows, their latest committed content, and the current focus. Keep IDs rather
than long-lived references to client surfaces. Rebuild affected cards when a
window maps, unmaps, changes title/state, or an output changes; never retain a
stale surface handle after destruction.

The visual layout follows Plasma's broad shape: a centered miniature desktop
strip across the top and an inset wallpaper canvas containing the selected
workspace's windows below. The presentation revision uses relative window sizes
and compact captions, avoiding full rectangular card backgrounds. Implemented
layout, preview bounds and limitations live in the [overview specification](../specs/overview.md).
Each card contains the window's scaled content, title, app icon when available,
and a minimized indicator. The selection highlight is compositor-owned. Cards
are arranged for legibility rather than by scaling normal tiled placements; no
client receives a resize/configure merely because the overview opened. Transient
popups and the optional shell UI are not included in a window thumbnail.

Opening initially selects the workspace presented on the initiating output.
Changing the highlighted workspace in the overview is a preview only. Enter or
click on a window closes the overview and invokes explicit `Focus(window)`, which
already reveals hidden workspaces and restores minimized windows. Activating a
workspace tile closes the overview and uses the ordinary workspace-switch
command. Escape closes without changing desktop focus, layout, or presentation.
An unmapped or closed selected window moves selection to the next valid card;
an empty workspace remains selectable. A minimized window can show its last
committed frame, with an icon/title fallback when no usable buffer exists.

The UI is drawn within each physical/virtual output rectangle, using that
output's logical scale and clipping. One global session has one keyboard
selection; pointer interaction can select a different output. A workspace shown
by a stretched output group keeps one identity, and a window appears once in the
overview rather than once per group member. For the first slice, the initiating
output hosts the interactive workspace strip and cards; other outputs are
dimmed and consume input while the overview is open. A later multi-output UI can
place cards on each output without changing core workspace ownership. Output
loss moves the session to a surviving output or closes it when none remain.

The current `Desktop::placements()` covers only presented workspaces and must
not be mutated to manufacture hidden-workspace previews. Overview cards use
read-only window/workspace records plus the latest mapped client buffers; they
do not call normal placement reconciliation for hidden workspaces.

## Rendering and resources

Build thumbnail textures on the GPU from the same committed toplevel surface
trees used by normal rendering. Include subsurfaces and the currently committed
SSD/CSD appearance, clip to the window's original frame, and scale only the
rendered result. Preserve transparent content over the overview wallpaper canvas.
Do not read pixels back to the CPU, encode image bytes into shell IPC, map hidden
windows into the ordinary scene, or issue thumbnail-sized XDG configures.

Prepare thumbnails lazily for cards that can be seen. The selected workspace's
cards may update on client damage, at most once per presented frame; other
workspace previews can retain a still frame until selected. Frame callbacks for
clients visible only as overview thumbnails should be deliberate and bounded,
without accidentally driving every hidden workspace at full rate. Drop GPU
resources on unmap, resize, output loss, renderer reset, and overview exit.
Start with a bounded cache (proposed 64 MiB and at most 512×320 pixels per card),
evict least-recently-used hidden cards, and fall back to icon/title cards under
pressure. Measure these limits before making them contractual.

The overview is a final compositor-owned pass over the usual wallpaper,
windows, and layer scene. It must not recursively include itself in thumbnails.
Session-lock surfaces, when implemented, take precedence. While open, ordinary
panel/notification layers are visually covered and do not receive clicks; an
existing exclusive keyboard layer blocks opening until it releases focus. The
overview should reuse Clear's text/icon preparation and output-local clipping,
but must not force blur or liquid-glass work for each card. Optical effects can
be added only after their cost is measured in the new render path.

The nested backend currently repaints continuously and assumes one scale-1
framebuffer. The first implementation can work there, but frame timing,
thumbnail scale, and per-output resources must be expressed so a later native
backend and damage-driven rendering do not require a second overview design.
Opening/closing animations should use monotonic presentation time, stop cleanly
on output loss, and have a reduced-motion/instant path.

## Input, focus, and lifecycle

When opening, cancel an Alt+Tab selection without committing it. Defer opening
while a client pointer/keyboard grab, compositor drag, or exclusive layer focus
is active; do not silently break that operation. Once open, Clear intercepts
overview pointer, wheel, and keyboard events before forwarding to the Wayland
seat. Match visual card geometry and hit regions from the same layout snapshot.
Suppress matching physical releases for intercepted presses, as the existing
shortcut path does. Keyboard support in the first slice includes arrows/Tab to
navigate, Enter to activate, and Escape to cancel; pointer clicks activate
cards or workspace tiles. Pointer events outside cards stay in the overview
rather than hitting covered clients.

Desktop policy focus is preserved during preview. Protocol keyboard and pointer
focus leave client surfaces while the overview owns input, then reconcile to the
selected or original surface on close. Never leave a client with a held key or
button because the overview intercepted only half of an event pair. If the
original window disappears, cancellation uses ordinary core focus repair.
Config reload may restyle/rebuild the overview but must not reset desktop
geometry; compositor shutdown drops the session and GPU resources.

The design must leave room for accessible names, selected state, and keyboard
navigation to be exposed through the planned screen-reader interface. A
compositor-rendered overview cannot rely on Qt accessibility to supply them.

## Delivery and verification

1. Add a pure `OverviewSession` and deterministic card/selection model. Test
   entry, preview without desktop mutation, activation, cancellation, window
   removal, workspace swaps, stretched groups, and output removal.
2. Add the typed action and compositor input mode. Test release suppression,
   grab/layer precedence, focus restoration, and pointer hit regions. Add an
   optional shell IPC trigger only after the keyboard path works without it.
3. Render card shells and then live GPU thumbnails. Use bounded captures to
   check stacking, title/content clipping, alpha, odd output sizes, multiple
   outputs, minimized/hidden windows, and resource cleanup. Check CPU/GPU time
   and memory with many windows before enabling live updates by default.
4. Window-to-workspace dragging has input/policy tests and its own contract.
   Add animation and later search/gesture progress with separate behavior
   specifications and input tests. Verify real keyboard, mouse, and touchpad
   behavior in an isolated session; GPU images alone cannot establish input
   behavior.

When implemented, update [desktop](../specs/desktop.md), [input](../specs/input.md),
[platform](../specs/platform.md), [rendering](../specs/rendering.md), and optional
[shell IPC](../specs/shell.md) contracts in the same code changes. This design
does not amend those contracts on its own.
