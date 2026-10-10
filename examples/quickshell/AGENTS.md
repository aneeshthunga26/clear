# Quickshell example

Follow the root guidance. This optional client never owns compositor policy.
Its contract is in [shell](../../specs/shell.md#optional-quickshell-appearances).

- `DesktopShell.qml` assembles shared per-output panels and services. Entry points
  `shell.qml` and `liquid-glass.qml` select appearance through `ShellStyle.qml`.
- Keep styling independent of `ClearBridge.qml`, IPC, and the pure `Protocol.js`
  model. Add alpha to background colors, not whole controls or windows.
- `ClearBridge.qml` discovers connection-authorized animation panels;
  `AppDock.qml` reports actual icon item geometry after layout and scrolling.
  Keep coordinate conversion in QML and validation/clipping in `Protocol.js`.
  Gate reporting on the advertised capability and a unique committed panel
  match; never guess global coordinates or replay reports across reconnects.
  See [advisory targets](../../specs/shell.md#advisory-panel-icon-targets).
- Match rounded panel input regions to their painted silhouettes. Preserve lazy
  overlay creation and the transparent 1×1 closed state.
- Keep standard appearance unchanged when adding glass styling. Run the Node
  model tests and bounded real Quickshell smokes for runtime/lifecycle changes.
  Read QML lint output, separating existing dynamic-API warnings from new errors.
- GPU captures verify pixels, not physical pointer or keyboard interaction.
