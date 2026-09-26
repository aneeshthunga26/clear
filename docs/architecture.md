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
bottom, background, with layer popups included. An input-region miss continues
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
- Activation, full cursor-surface handling, and complete fullscreen/maximize
  semantics need their own protocol/policy work rather than shortcuts in the
  renderer. Physical pointer/keyboard gestures still need interactive testing;
  the automated VM fixture checks protocol ownership and framebuffer output.
