# Smithay adapter

Read [platform guidance](../AGENTS.md) and the root guidance first. This is the
only directory allowed to depend on Smithay and Wayland types.

## File ownership

- `mod.rs`: adapter wiring and protocol delegation, including the XDG decoration
  destruction notification absent from the pinned Smithay handler.
- `state.rs`: compositor resources, managed client handles, output regions,
  effect execution, and runtime bridge.
- `backend.rs`: nested winit lifecycle, virtual outputs, rendering loop, bounded
  shutdown, and framebuffer capture.
- `frame_scheduler.rs`: coalesced host redraw requests, monotonic frame capture,
  absolute capped animation sampling, and explicitly identified host metadata or
  nominal timing. Keep animation sampling separate from client callback pacing.
- `protocols.rs`: XDG surface lifecycle, metadata, popups, seat/selection handling,
  and client move/resize requests.
- `decorations.rs`: XDG/KDE negotiation and committed-versus-pending SSD mode.
- `titlebar.rs`: configurable shared titlebar geometry/hits, Unicode text and
  prepared-icon composition, and renderer-owned bounded per-visible-strip textures.
  Schema lives in `decoration/titlebar.rs`; bounded local SVG/app-icon CPU preparation
  lives in `runtime/titlebar.rs`, never filesystem IO/SVG decoding in rendering.
  No UI toolkit or shell dependency.
- `layers.rs`: layer-shell lifecycle, namespace policy, and keyboard ownership.
- `overview.rs`: shared canvas/desktop-strip layout and hits, GPU thumbnails and
  compact metadata-label caches, plus the transient pointer drag/ghost description.
  Input arms/drops it; reconciliation cancels invalid sources. Reuse one bounded texture across card/miniature
  destinations, plan its resolution from the largest visible destination before
  rendering any tile, and explicitly sample its full source when scaling. Apply
  desktop corner masks to the composited content/SSD source before resizing the
  complete image. Radii/insets scale with that source. A separate selection ring
  follows its silhouette without masking the client image again.
  Render as a final pass, with no hidden placement reconciliation or thumbnail-sized
  configures. Share visible preview candidates with callback/suspension and output
  membership handling; synchronize preview output notifications after Space refresh
  and restore ordinary ownership on cleanup. Follow [overview](../../../specs/overview.md) bounds and cleanup.
- `input.rs`: physical event translation, shortcuts, pointer routing, and drags.
- `shell.rs`: optional socket initialization and dispatch; reconcile commands
  before acknowledging/publishing. Wire types and transport stay in `src/shell/`.
- `scene.rs`: reconciliation, configure requests, ordered rendering/hit-testing,
  shared visibility clips, and frame callbacks.
- `wallpaper.rs`: renderer-owned immutable texture cache, per-output image
  placement and scaled overview placement preserving the full-output crop.
  Resource decoding and filesystem access stay in runtime.
- `blur.rs`: renderer-owned backdrop scratch textures, bounded separable Gaussian
  and Dual Kawase filters, viewport-pyramid LRU, per-tree composition, and independent
  coverage/alpha masks. `magnify.frag` and `liquid_glass.frag` translate the two
  reference SVG displacement stages and specular compositing, using immutable
  embedded maps in `glass-maps/`. Keep magnification as a separate RGBA8 pass;
  collapsing sample coordinates changes interpolation. Initialize the whole
  viewport for glass, preserve original-frame map coordinates across crops, and
  restore every auxiliary sampler binding. Glass works with blur radius zero;
  enable the scene's ring-only square borders for either backdrop effect.
  Client-alpha layers default to square optical bounds and alpha-derived backdrop
  coverage; `clear-glass-pill-*` / `clear-glass-rounded-*` opt into fitted radii
  and independent geometric backdrop coverage across the panel interior.
- `rounded.rs`: fitted outer/inner window outlines, shared body hit masks, and
  renderer-owned rounded shaders with offscreen surface-tree composition.

## Protocol and geometry invariants

- A surface object is not necessarily mapped. Only buffered windows enter core
  policy. Preserve handles needed for subsequent mapping.
- Advertise implemented maximize/minimize/fullscreen capabilities. Retain pre-map
  maximize/fullscreen intent and optional fullscreen output until first mapping;
  configure usable area for maximize and full bounds for fullscreen. Reconciliation
  synchronizes Maximized/Fullscreen/Suspended states.
  Minimize removes placements/space visibility, never the managed mapping or core
  window. Clear initial intent and transient protocol states on client unmap.
  State changes cancel active drags; maximized/fullscreen windows reject dragging.
  Suppress SSD, borders and rounding only at a buffered fullscreen root commit;
  an ACK alone cannot switch displayed decoration. Preserve negotiated mode.
  Keep fullscreen-output Top layer ordering identical for rendering and hits.
- Requested placement, client-committed geometry, and saved floating geometry are
  distinct. Account for nonzero XDG geometry offsets in rendering and hit tests.
- Default negotiated XDG/KDE decorations to SSD, honor explicit CSD (and KDE none),
  and prefer XDG when both exist. No protocol means CSD. Pending mode sizes client
  configures, while committed root-buffer mode drives rendering and input. ACKs
  and subsurface commits do not change the visible XDG mode. Destroy retains the
  current mode until the next root commit; replacement/remap must not reuse stale
  configures. Hidden/unplaced reconciliation must not initiate initial configures.
- Placements are whole frames. Subtract configured SSD height (integer `16..=128`,
  default 32) for normal configure sizes and add it to launcher committed dimensions
  before core centering. Tiny frames retain at least one client row. Account for
  the same dynamic inset in space mapping, render trees, pointer and popup origins.
  Reloading height never mutates saved geometry or tile proportions. Keep requested
  frame hints distinct from committed content and saved geometry.
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

- Front-to-back order is overlay, top, application placements, bottom, background,
  wallpaper. Wallpapers use full output rectangles, never panel-reduced usable
  areas; they have no input ownership and do not span stretched output groups.
  Include separate layer popup trees as well as parent surfaces.
- Use the same output-group clips for rendering and hit-testing. Floats may span
  their group's outputs, not independent workspaces or gaps between monitors.
  Tile bodies also respect requested bounds while clients resize asynchronously.
- Rounded outlines derive from the original placement, never from each output
  crop. Body hit-testing and shader coverage intersect all applicable corner arcs;
  popups and layer-shell trees bypass rounding. Zero radius keeps the square path.
  Composite each unmasked body tree before applying coverage once, otherwise
  overlapping subsurfaces leak through the antialiased edge. Add premultiplied body
  and ring coverage before blending; do not hide border color behind translucent
  content. Never claim the combined texture's transparent corners are opaque.
  Lazy shader ownership stays with the backend; per-clip offscreen allocations
  are bounded by visible geometry, with full damage on theme/shape changes.
  Current shader coordinates assume scale 1 and one nested framebuffer.
- Backdrop blur groups each window/layer/popup tree, never individual subsurfaces.
  Consume groups bottom-to-top and filter only the already composited lower scene;
  keep foreground glyphs and opaque pixels sharp. Preserve independent rounded
  coverage `C` versus premultiplied alpha `A`, with premultiplied foreground `F`:
  `F + (C-A) * blurred + (1-C) * original`. Layers and popups without explicit
  coverage use `A * (1-A)` blur weight to preserve holes. Neither method changes
  client opacity. Optional liquid-glass optics replace only the filtered backdrop;
  retain original uncropped tree bounds, viewport clamps, premultiplied alpha, and
  fully initialized viewport sources. See [rendering](../../../specs/rendering.md#liquid-glass).
- Global `blur_method` selects `gaussian` (default) or `kawase` (Dual Kawase).
  Finite `blur_radius` in `0..=32` defaults to zero, bypassing blur exactly;
  enabled glass remains independent.
  Gaussian uses logical-pixel support and two separable passes. Kawase uses offsets
  in source pyramid texels, without an implicit half-texel offset; these are not
  Gaussian-equivalent radii. Integer `blur_passes` in `1..=6` defaults to 3 and is
  always validated, even for Gaussian (which ignores it) or radius zero.
- Kawase isolates the whole output/workspace viewport at level zero, ceil-halves
  each dimension for up to `blur_passes` downsample levels, stops early only at
  `1×1`, then upsamples through the same levels in reverse. Use actual size ratios
  for center-aligned linear sampling of odd dimensions and clamp to each source
  level's texel centers. Preserve storage-coordinate crop origins and RGBA8
  intermediates. The VM example uses Kawase radius 2/passes 3.
- Both filters clamp sampling to the output/workspace viewport. Keep read/write
  targets distinct and restore auxiliary sampler state plus the window target.
  Four shared framebuffer-sized RGBA8 scratch textures are retained (16 bytes per
  host pixel). Kawase adds an LRU keyed by viewport size/effective depth, not origin
  or radius: at most four pyramids and three framebuffer areas of texels total,
  including level zero (up to 12 more bytes per host pixel). The retained blur
  texture bound is therefore seven framebuffer areas, excluding driver overhead,
  other renderer resources, and transient replacement allocations. Refill level
  zero for every filtered tree and overwrite downsample levels on ascent; cache
  allocations, not prior scene pixels. Framebuffer resize replaces scratch and
  clears pyramids on the next blur render. Switching method or disabling blur does
  not immediately release allocations; disabled caches retain their last rendered
  size. Initialize each method's shaders lazily.
- Use unified hit ownership: panel/popup clicks must not focus or start dragging
  a window behind them. Continue past surfaces whose input regions miss.
- `[theme.titlebar]` selects active/inactive background/foreground RGBA (exactly
  four finite channels in `0..=1`), controls_side left/right (default right),
  show_icon (default false), and show_title (default true). Right-side controls read
  minimize/maximize-or-restore/close; left-side controls mirror that order. Close
  retains priority on narrow frames, then maximize, then minimize. Drawing and hits
  share original-frame boxes, even when cropped across outputs.
- Optional `[theme.titlebar.controls]` minimize/maximize/restore/close SVGs resolve
  relative to config; absent restore falls back to maximize SVG, then built-ins.
  Keep literal SVG colors/alpha, not foreground recoloring. Runtime uses minimal
  resvg with bounded local files, no external/network/embedded images, SVG text/fonts,
  SVGZ, or DTD/entities. Reject use/pattern/marker/mask/clipPath/filter elements by
  local name before usvg conversion, even unused/namespaced definitions; the subset
  supports simple paths/gradients, not masks/clips/filters. App icons use exact desktop
  IDs and bounded hicolor/direct-icon/pixmaps fallback, not full icon-theme resolution.
  Missing app icons use a generic glyph; startup control failures use built-ins.
  Failed reloads retain the whole config/resource set; successful reloads reread
  same-path assets. Access prepared images only from rendering.
- Titlebars and clients share one rounded outline; composite them before masking.
  Preserve alpha in premultiplied source-over and the rounded shader, including
  translucent foregrounds; never force sampled alpha to one or double-premultiply
  prepared icons. No forced opaque titlebar fill, accent stripe, or separator.
  Square SSD borders use a ring even without blur, never a backing fill behind
  transparent titlebars. Preserve legacy square unblurred CSD backing.
  Titlebar textures cover only visible strips, at most 4096×128 pixels each. Cache
  identity includes full style, app ID and prepared-image identity as well as title,
  local crop, size, focus and maximization. Both 64 MiB texture bytes and 128 entries
  bound retained cache storage, excluding outstanding elements and driver overhead.
  SSD hits never own Wayland pointer focus. Suppress paired releases; controls
  activate only on release over the original window/control. Titlebar clicks focus;
  detach a tile only after the movement threshold. Super+mouse gestures take priority
  over controls, including close. Host focus loss cancels armed titlebar interactions.
- Mapped exclusive overlay/top layers preempt application keyboard focus;
  on-demand layers gain focus on click. Restore focus on unmap/destruction and
  follow mapping/stacking order rather than only creation order.
- Convert keysyms with `xkb::keysym_get_name`, not debug `Keysym::name()` labels.
  Track intercepted releases by physical keycode; preserve grab/serial checks.
- Resize drags hold core `ResizeSession`s and pass total displacement from the
  initial pointer position. End invalid sessions and clear XDG Resizing state.
  Never detach tiles or overwrite saved float geometry during resize; only move
  drags detach. Super+right chooses a corner by pointer quadrant; native requests
  retain their requested edges and existing seat/grab/serial authorization.
  Spiral/monocle/script tiles refuse resizing, but floating exceptions do not.
- Alt-Tab is selected in runtime and committed on physical Alt release; Escape
  cancels. Keep keyboard ownership and final focus in the compositor.
- Shell state is published after reconciliation, using a dirty flag and snapshot
  comparison rather than running scripts or serializing from the renderer. Poll
  the nonblocking transport each event-loop turn to flush replies. Export this
  instance's `CLEAR_SOCKET` to children, removing inherited values if disabled.
  Optional IPC bind failures must not prevent compositor startup.
- Framebuffer texture mapping can change the EGL target. Preserve the backend's
  restoration of the window target before swapping after capture.
- Request synchronized EGL interval 1 after rendering to the host framebuffer;
  `GlesRenderer::bind` only wraps the target and does not make EGL current. The
  pinned `GlAttributes::vsync` only selects a compatible config. Do not change
  interval or configured sampling target in response to lateness. Host redraw
  metadata is not presentation feedback; follow the timing limitations in
  [platform](../../../specs/platform.md#nested-frame-timing). Continuous redraw
  remains necessary until all client/layer/exposure/callback invalidations and
  capture deadlines have explicit wakeups.

## Verification

Run the root Rust workflow, including adapter unit tests via
`cargo test --locked --lib platform::smithay`. Rebuild the binary before VM tests.
Use both built-in/Rhai foot smokes and `scripts/vm-layer-smoke.py` when relevant.
The layer fixture checks real protocol lifecycle, reservations, pending-state
isolation, keyboard ownership, popup pixels, and client-sized launcher centering.
Use `scripts/vm-wallpaper-smoke.py` for full-output wallpaper placement, scaling,
per-output resources, and layer priority with real GPU captures.
`scripts/vm-window-state-smoke.py` checks native XDG maximize/minimize (including
pre-map maximize), restore, configure state/size, focus, IPC, and capture visibility.
`scripts/vm-fullscreen-smoke.py` checks real fullscreen requests, full bounds,
overlay/top priority, output targets, pre-map state, held SSD commits, restoration
and launcher refusal. Synthetic gesture refusal is covered by adapter unit tests;
neither establishes physical input.
`scripts/vm-rounded-smoke.py` checks fitted/asymmetric radii, body/border alpha,
subsurface composition, geometry offsets, cut-outs, and unchanged layers/popups.
`scripts/vm-decoration-smoke.py` checks real XDG/KDE negotiation, held ACKs/commits,
object lifecycle, title/control pixels, title updates, frame/content insets, launcher
centering, reservations, maximize, and popup origins. The 11 original configurable
`titlebar-*` GPU cases passed on private LOCAL virtual KWin, not a VM. Geometry,
launcher/popup, mirrored controls, hidden text and generic icon results are in
`target/titlebar-geometry/`; translucency, inactive colors and the three SVG cases
are in `target/titlebar-colors/`. Both square GPU regressions passed in
`target/titlebar-square/`; eight selected negotiation cases passed in
`target/titlebar-negotiation/`, not the entire legacy suite. Kawase radius 2/passes 3
`rounded-ssd` passed in `target/titlebar-blur-regression/`. No physical input was
tested. All 14 CPU harness/oracle tests (including two square regressions), 10
schema/resource tests and Cargo fmt/check/test/build passed after fixes. Do not
infer GPU passes from CPU tests. Reload is covered by Rust tests, not this GPU
fixture, whose IPC allowlist excludes reload. See `docs/vm-testing.md` for exact
cases, bounded commands and coverage limits.
`scripts/vm-blur-smoke.py` compares radius zero with `--radius` using independent
Gaussian/Dual Kawase oracles across windows, every layer kind, popups, stacking,
and output boundaries; checks crisp foreground, composed subsurfaces, holes, and
rounded outlines. `--method` defaults to `gaussian`; radius defaults to 12 for
Gaussian or 2 for Kawase; `--passes` defaults to 3 (range 1..6). Include
`--case output-boundary-odd` for a 321×241 viewport at (319, 0) beside magenta,
and focused fractional-radius runs at depths 1/3/6. Preserve separate artifact
paths. `scripts/test_vm_blur_smoke.py` runs compositor-free oracle/harness
regressions, including reference map integrity, SVG stage order, independent
controls, and source/foreground handling; these are not GPU validation.
Recorded GPU passes on private local virtual KWin (not a VM) cover the original
ten Kawase cases at radius 2/passes 3, plus `stacking` and
`output-boundary-odd` at radius 1.5/passes 1 and 6. Gaussian radius 12 passed only
`xdg`, `stacking`, and `output-boundary-odd` in this round. Full Cargo
fmt/check/test/build and all 11 CPU oracle/harness tests also passed; no physical
input tests were performed. See `docs/vm-testing.md` for artifact paths.
These fixtures do not synthesize physical clicks, keys, or drags. Report that
distinction.
See `docs/vm-testing.md` for portable bounded runs, fixture builds, and artifacts.
Machine-specific access and recovery notes belong only in the ignored `.agents/`
directory described by the root guidance.
