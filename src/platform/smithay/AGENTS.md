# Smithay adapter

Read [platform guidance](../AGENTS.md) and the root guidance first. This is the
only directory allowed to depend on Smithay and Wayland types.

## File ownership

- `mod.rs`: adapter wiring and protocol delegation.
- `state.rs`: compositor resources, managed client handles, output regions,
  effect execution, and runtime bridge.
- `backend.rs`: nested winit lifecycle, virtual outputs, rendering loop, bounded
  shutdown, and framebuffer capture.
- `protocols.rs`: XDG surface lifecycle, metadata, popups, seat/selection handling,
  and client move/resize requests.
- `layers.rs`: layer-shell lifecycle, namespace policy, and keyboard ownership.
- `input.rs`: physical event translation, shortcuts, pointer routing, and drags.
- `scene.rs`: reconciliation, configure requests, ordered rendering/hit-testing,
  shared visibility clips, and frame callbacks.

## Protocol and geometry invariants

- A surface object is not necessarily mapped. Only buffered windows enter core
  policy. Preserve handles needed for subsequent mapping.
- Requested placement, client-committed geometry, and saved floating geometry are
  distinct. Account for nonzero XDG geometry offsets in rendering and hit tests.
- Launcher-role XDG clients choose their size: do not force placement-sized
  configure hints. Record committed size and let core recenter them. Normal drag
  logic must not convert or resize these launchers.
- Layers may need temporary arrangement for an initial bufferless configure, but
  must not reserve desktop space or take focus until mapped. Null-buffer unmap
  releases both; do not consume the next initial configure on the unmap commit.
- Namespace rules affect the effective current layer only. Never overwrite
  pending client state during reconciliation; `set_layer` is double-buffered.
- Derive usable areas from each layer map's non-exclusive zone plus output origin.
  Pass them to core without modifying saved floating rectangles or restarting bars.

## Rendering and input

- Front-to-back order is overlay, top, application placements, bottom, background.
  Include separate layer popup trees as well as parent surfaces.
- Use the same output-group clips for rendering and hit-testing. Floats may span
  their group's outputs, not independent workspaces or gaps between monitors.
  Tile bodies also respect requested bounds while clients resize asynchronously.
- Use unified hit ownership: panel/popup clicks must not focus or start dragging
  a window behind them. Continue past surfaces whose input regions miss.
- Mapped exclusive overlay/top layers preempt application keyboard focus;
  on-demand layers gain focus on click. Restore focus on unmap/destruction and
  follow mapping/stacking order rather than only creation order.
- Convert keysyms with `xkb::keysym_get_name`, not debug `Keysym::name()` labels.
  Track intercepted releases by physical keycode; preserve grab/serial checks.
- Framebuffer texture mapping can change the EGL target. Preserve the backend's
  restoration of the window target before swapping after capture.

## Verification

Run the root Rust workflow, including adapter unit tests via
`cargo test --locked --lib platform::smithay`. Rebuild the binary before VM tests.
Use both built-in/Rhai foot smokes and `scripts/vm-layer-smoke.py` when relevant.
The layer fixture checks real protocol lifecycle, reservations, pending-state
isolation, keyboard ownership, popup pixels, and client-sized launcher centering.
It does not synthesize physical clicks, keys, or drags. Report that distinction.
See `docs/vm-testing.md` for portable bounded runs, fixture builds, and artifacts.
Machine-specific access and recovery notes belong only in the ignored `.agents/`
directory described by the root guidance.
