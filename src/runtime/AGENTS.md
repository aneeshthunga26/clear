# Runtime orchestration

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities and boundaries

`mod.rs` connects configuration, compiled bindings, desktop commands, and Rhai.
It also defines backend-neutral launch `Options`. Keep Smithay and protocol
objects out of this module; return effects for the platform to execute.

- `overview.rs` owns ID-only preview selection, navigation and validated drops;
  move explicit window IDs without incidental focus/reveal. Commit through core
  commands, never compute hidden placements. The adapter authorizes queued toggles.
  See [overview](../../specs/overview.md) for lifecycle and input requirements.
- `animation.rs` owns the pure monotonic clock, closed-form pose tracks, group IDs,
  and bounded track storage. Keep policy, surfaces, and GPU resources outside it.
  Convert adapter timestamps through the runtime's absolute monotonic origin.
  Reload preparation must finish before clock rebasing/settlement; see
  [animations](../../specs/animations.md). Visible effects are not yet connected.
- `presentation.rs` owns pure validated snapshot diffs, retained-image request IDs,
  workspace/window transition tracks, and immutable sampled frames. Policy and
  GPU ownership stay with core and the adapter. Only acknowledge actual submitted
  draws, preserving captured phase and opaque track revision metadata; retirement
  requires an acknowledged terminal frame, even after config settlement.
  `is_animating` includes a pending final redraw. Keep overview composition and
  adapter input transforms outside the planner. See
  [animations](../../specs/animations.md#pure-presentation-planning); these pure
  frames are not yet consumed by the renderer.
- Startup may fall back to safe defaults. Reload must prepare and validate the
  candidate config, bindings, and script host before replacing live state.
  Rejected reloads retain the last good configuration and behavior.
- `wallpaper.rs` prepares immutable premultiplied RGBA resources on startup/reload,
  never per frame. PNG/JPEG only; encoded files are capped at 16 MiB, dimensions
  at 8192, per-image decode/RGBA at 64 MiB, and aggregate RGBA at 128 MiB.
  Identical paths share resources. Explicit reload rereads even unchanged paths.
  Failed resource preparation retains the entire last-good reload state; startup
  warns and uses the configured solid theme background instead. The adapter owns
  textures and placement, with no dependency on a shell client.
- `titlebar.rs` prepares backend-neutral `TitlebarAssets`; `TitlebarImage` contains
  public width/height and premultiplied RGBA8 pixels. Explicit control SVGs are
  prepared on startup/reload, deduplicated by path, and reread even when paths are
  unchanged. Startup resource errors log and use built-in controls while keeping
  theme/config policy; reload errors retain all prior config and asset sets.
- Titlebar SVGs use resvg with default features disabled (no text/system fonts,
  SVGZ, or embedded raster decoders), plus disabled data/file image resolvers.
  XML DTDs are rejected; input is capped at 256 KiB, 4096 XML nodes, 32 element
  levels, and intrinsic dimensions 1024. Reject all `use`, `pattern`, `marker`,
  `mask`, `clipPath`, and `filter` elements by local name before usvg conversion,
  even unused or namespaced definitions. Shallow sibling reference graphs can
  expand exponentially despite input/depth/raster limits; CSS or href references
  must not bypass this restriction. Paths and gradients remain supported.
  Render original colors into a transparent aspect-preserving square box of
  `TitlebarTheme::icon_size()` (at most 20x20), never per frame.
- App icons are optional local best-effort resources, prepared only by
  `classify_window` (including successful reload reclassification of hidden
  windows). Match exact app-ID `.desktop` filenames in XDG data roots, read only
  the main group's Icon key, respect Hidden=true, and never execute entry fields.
  Search common hicolor scalable/fixed-size apps directories, direct icons, and
  pixmaps, plus legacy `~/.icons`; absolute Icon paths are supported. No recursive
  scans, theme-inheritance traversal, or network access. Unknown/invalid icons
  return None for the renderer's generic fallback. `show_icon=false` skips lookup.
- App lookup retains at most 256 positive/negative IDs (255-byte safe names), uses
  at most 8 XDG roots plus legacy icons, 256 probes per app and 4096 per generation,
  and 16 MiB aggregate read allowance. Desktop files cap at 64 KiB; raster app
  icons at 2 MiB encoded, 1024 per dimension and 8 MiB decoder allocation. SVGs
  share the control limits. PNG/JPEG pixels are premultiplied before resizing.
  Accessors never touch files; exhaustion stops new lookups until reload.
- Output topology changes currently require restart. Do not partially apply an
  unsupported topology reload.
- Apply workspace declarations to persistent desktop IDs without discarding
  existing windows or unrelated saved state.
- Classify XDG app IDs into core window roles on mapping/metadata changes and
  reclassify all managed windows after successful reload, including hidden ones.
  Classification must preserve ownership, focus, and saved floating geometry.
- Script actions return validated declarative actions; never let scripts mutate
  the desktop or own compositor objects directly.
- Keep per-function failure handling: disable failing script functions until
  reload and use the built-in layout fallback rather than failing the compositor.
- Spawning, closing clients, and shutdown are explicit effects, not OS/protocol
  operations performed by policy code.
- Pending Alt-Tab selection contains normal windows on the focused workspace.
  Advancing it must not change focus; finish commits the selected window and
  cancel preserves focus. Minimized windows remain candidates and restore only
  when explicit focus is committed; cancelling never restores them. Maximize and
  minimize actions route to core without rewriting modes or floating geometry.
  The adapter owns physical release/cancel detection.

## Verification

Start with `cargo test --locked --test runtime --test titlebar_theme`; animation
changes also require `cargo test --locked --test animations --test presentation`,
then the
root workflow. Titlebar app-icon tests isolate XDG/HOME using a subprocess, not
unsafe process-global environment mutation; they verify CPU resources, not GPU
pixels or physical input. Cover
command routing, reload atomicity, script failure/fallback, hidden-window
reclassification, and preserved state. Use the VM Rhai smoke for adapter-level
integration; a pure runtime test does not verify physical shortcuts.
