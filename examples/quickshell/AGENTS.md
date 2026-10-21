# Quickshell example

Follow the root guidance. This optional client never owns compositor policy.
Its contract is in [shell](../../specs/shell.md#optional-quickshell-appearances).

- `DesktopShell.qml` assembles shared per-output panels and services. Entry points
  `shell.qml` and `liquid-glass.qml` select appearance through `ShellStyle.qml`.
- Keep styling independent of `ClearBridge.qml`, IPC, and the pure `Protocol.js`
  model. Add alpha to background colors, not whole controls or windows.
- `ShellToolTip.qml` owns delayed, noninteractive popup placement; `AppIcon.qml`
  owns theme lookup and the bundled missing-icon fallback. Reuse these across
  controls. Explicitly anchor panel hover surfaces below the panel and prevent
  upward flips; retain the preview's cancellable dwell and pointer-crossing delay.
- `AppContextMenu.qml` owns the output-local modal app/window menu and executes
  published desktop actions through Quickshell. Its transparent dismissal layer
  excludes the separate bounded painted body from input. Only the bounded body
  opts into glass; never apply geometric coverage to the full-output catcher.
  Retain both empty 1×1 roles on close, but keep the content layout at its final
  size while transparent. Derive requested geometry from the screen, not the
  dismissal layer's last configure. Incubate the closed menu asynchronously at
  startup so the first right-click does not compile it. `MenuAction.qml` shares rows.
  `LivePreview.qml` swaps two uncached image slots only after decode succeeds;
  never show an icon between successful frames. Keep preview delegates keyed by
  window ID across metadata snapshots. Hover surfaces use a one-second dwell.
  Window commands use explicit IDs, never incidental focus. `ClearBridge.qml`
  keeps at most one preview request in flight; hover polling must stop on close.
- `ClearBridge.qml` discovers connection-authorized animation panels;
  `AppDock.qml` reports actual icon item geometry after layout and scrolling.
  Keep coordinate conversion in QML and validation/clipping in `Protocol.js`.
  Gate reporting on the advertised capability and a unique committed panel
  match; never guess global coordinates or replay reports across reconnects.
  See [advisory targets](../../specs/shell.md#advisory-panel-icon-targets).
- Fullscreen navigation comes from the optional `fullscreen_desktop` snapshot,
  never a synthetic entry in the wire workspace array. Keep activation typed and
  resolve title/task visibility using the effective presentation.
- Match rounded panel input regions to their painted silhouettes. Preserve lazy
  overlay creation and the transparent 1×1 closed state.
- Keep glass tint/geometry independent of standard appearance changes. Run the Node
  model tests and bounded real Quickshell smokes for runtime/lifecycle changes.
  Read QML lint output, separating existing dynamic-API warnings from new errors.
- `scripts/vm-shell-ui-smoke.py` checks single-output UI captures, delayed hover
  cancellation and real popup coordinates; include compact and glass cases for
  shared layout/interaction changes. It does not inject physical input.
- Use `--glass --verify-backdrops` for actual blur/glass GPU comparisons; the
  ordinary `--glass` flag only chooses the QML appearance in the UI fixture.
- GPU captures verify pixels, not physical pointer or keyboard interaction.
