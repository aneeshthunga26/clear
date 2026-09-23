# Platform adapter boundary

Parent guidance: [src/AGENTS.md](../AGENTS.md).

`mod.rs` exposes the platform runner while keeping the adapter implementation
private. It should stay a small boundary, not become another desktop-policy
or orchestration layer.

## Nested module

Read [smithay/AGENTS.md](smithay/AGENTS.md) before changing protocols, rendering,
physical input, or the nested backend.

## Rules

- Keep all Smithay and Wayland imports in `smithay/`, not in this entry module or
  backend-independent modules.
- Translate native events into typed core/runtime operations and interpret their
  returned effects in the adapter.
- Keep public launch options backend-neutral through `runtime::Options`.
- The current backend is nested winit with virtual outputs. Native DRM/KMS and
  physical-monitor hotplug are not implemented; do not imply otherwise.
- New backends must preserve the desktop/runtime boundary instead of teaching
  core policy about surfaces, seats, serials, or renderer resources.
- Preserve the pinned Smithay revision and existing feature expectations unless
  a dependency change is explicitly part of the task.

## Verification

Follow the root Rust workflow. Adapter behavior requires bounded VM integration
in addition to policy tests; see the nested guidance and `docs/vm-testing.md`.
