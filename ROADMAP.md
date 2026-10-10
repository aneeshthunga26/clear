# Planned desktop features

This is a proposal backlog, not a description of implemented behavior. The
[specifications](specs/README.md) remain the source of truth for what Clear does
today. Each feature needs its own design and specification update when work on it
begins. The items below have no committed dates or priorities.

Clear currently runs nested with virtual outputs. It has Super+mouse
move/resize/scroll gestures and an Alt+Tab switcher, and an instant compositor
overview with window-to-workspace drops, but no touchpad gestures,
animated overview transitions, compositor animations, live window capture protocol, native DRM/KMS
output, HDR pipeline, or compositor screen-reader interface. See the current
[input](specs/input.md), [platform](specs/platform.md#current-limitations), and
[rendering](specs/rendering.md) contracts.

## Feature list

| Feature | Initial deliverable | Completion checks and dependencies |
| --- | --- | --- |
| Monitor and window screencasting | Let a user select an output or an individual window through a desktop portal and stream it to PipeWire. | OBS and a browser can start/stop either source; output changes and window closure end or update streams safely; cursor and private surfaces have explicit policies. Build the capture path and a standalone session/portal integration first. Niri's [screencasting guide](https://niri-wm.github.io/niri/Screencasting.html) is the reference for source selection and privacy. |
| Touchpad and mouse gestures | Add three-finger workspace navigation, four-finger overview control, and mouse drag/scroll navigation beyond the existing Super+pointer operations. | Gestures track progress, cancel cleanly, respect output boundaries and client grabs, and remain configurable. Touchpad input needs a native input path; nested winit may not expose the required gesture events. See Niri's [gesture behavior](https://niri-wm.github.io/niri/Gestures.html). |
| Overview, following Plasma | The [initial overview](specs/overview.md) is implemented; extend the [design](docs/overview-design.md) with animation, search and window transfers. | Clear owns activation, transient selection, rendering, and input; Quickshell may only request opening. Selecting a card uses normal focus/restore behavior. Search and moving windows between workspaces follow in later slices. [Plasma's KWin overview](https://github.com/KDE/kwin/tree/master/src/plugins/overview) is the visual reference. |
| Hot corners for overview and show desktop | Configure a corner of each output to open the overview or toggle show desktop. | Edge dwell, retrigger delay, fullscreen behavior, and multi-output boundaries are predictable; show desktop restores only the windows it hid. The action belongs in compositor state rather than the optional Quickshell example. Use KDE's [screen-edge controls](https://docs.kde.org/trunk_kf6/en/kwin/kcontrol/kwinscreenedges/index.html) as the interaction reference. |
| Animations | Follow the [implementation plan](docs/animations-design.md) for workspace slides, window open/close and tiled movement, minimize/restore to panel icons, maximize, overview with wallpaper scaling, and fullscreen. | Configuration/clock, nested frame sampling, a pure presentation planner, shared GPU window images, inverse pointer delivery and optional validated panel hints are implemented. Connect these foundations to scene effects next; default to presentation refresh with 30/60/120 caps, speed controls and no automatic divisor fallback. Preserve final policy, asynchronous client handling and bounded GPU resources. Instant fullscreen policy and XDG handling are implemented. Niri's [animation controls](https://niri-wm.github.io/niri/Configuration%3A-Animations.html) are a reference. |
| HDR | Display HDR clients on capable physical outputs while keeping SDR content and captures correct. | Negotiate color spaces and output capability, render at sufficient precision, apply SDR/HDR mapping, and fall back cleanly on SDR displays. This requires native DRM/KMS plus a color-managed rendering and capture pipeline; the current RGBA8, scale-1 nested path is not an HDR base. Track this independently: HDR is not listed among Niri's mainline features in its [README](https://github.com/niri-wm/niri). |
| Screen reader support | Make compositor-owned controls and state changes usable with Orca, starting with workspaces, Alt+Tab, and the overview. | Keyboard-only navigation and spoken focus/status announcements work in a standalone Clear session; the optional shell is audited separately. Investigate AccessKit and the accessibility keyboard-monitor interface, while avoiding conflicts with a host screen reader in nested mode. Niri's [accessibility guide](https://niri-wm.github.io/niri/Accessibility.html) is the reference. |

## Shared groundwork

- A standalone session with native output and input support is a prerequisite
  for physical touchpad testing and meaningful HDR validation. It also provides
  the session environment needed by screen readers and desktop portals.
- Overview, animations, and screencasting all need a clear ownership model for
  scene snapshots, output-local clipping, frame timing, and cleanup when windows
  or outputs disappear.
- Accessibility and reduced-motion behavior should be designed with the overview
  and animations, then verified with actual input devices and Orca. GPU captures
  and protocol fixtures alone cannot establish those interactions.
