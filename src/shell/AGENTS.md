# Backend-independent shell protocol

Parent guidance: [src/AGENTS.md](../AGENTS.md). Also follow core and runtime
invariants. The model owns no backend or renderer resources; the transport is
plain Unix IO and must not import Smithay, Wayland, Qt, or Quickshell.

## Responsibilities

- `mod.rs` defines version 1 requests, owned serializable snapshots, allowlisted
  command execution, and newline-terminated JSON response encoding.
- `server.rs` owns nonblocking newline framing, connection/message/work budgets,
  subscriptions, socket ownership/permissions, backpressure, and identity-checked
  cleanup. Never unlink arbitrary stale paths or grow buffers without limits.
- `animation_targets.rs` owns bounded connection-owned advisory icon records,
  strict panel-local geometry/selector validation, expiry and pure resolution.
  Live mapped panel descriptors and Wayland ownership stay in the adapter; never
  trust client-provided output coordinates or mutate core state from hints.
- The Smithay adapter owns backend reconciliation. It reconciles successful
  mutations before acknowledging/publishing; model snapshot reads remain pure.
- Preserve optional operation: no shell process is required, and no arbitrary
  toolkit-specific actions belong in the wire contract. Future native shells can
  consume the same model. See [the shell specification](../../specs/shell.md)
  for the v1 contract and update it with every implemented protocol change.
- `parse_request` rejects malformed/unknown fields and unsupported versions.
  `execute` independently checks the version and validates every target and mode
  before mutation, including when callers construct requests directly.
- Request IDs are `u32`; all desktop IDs are canonical decimal strings. Resolve
  targets by ID, not display names. Never expose spawn, quit, reload, arbitrary
  actions, or script invocation through this capability allowlist.

## State and behavior

- Snapshots are pure reads of core policy and runtime switcher state, including
  hidden windows, launcher roles and read-only overview open state. The validated
  `toggle_overview` request queues the same adapter authorization as the shortcut. Never compute placements or invoke scripts
  to build a snapshot.
- Output `area` is usable logical geometry after reservations, not physical bounds.
  Groups describe presentation; workspaces retain ownership and stable window order.
  Window `output` is its saved home, not proof of visibility, and `floating` is its
  saved exception flag rather than a derived layout/launcher status.
- Modes serialize canonically, including the `script:` prefix. Input uses core
  mode parsing, limited to 256 UTF-8 bytes; empty and unknown modes fail. Script
  layout names retain core's fallback behavior, not arbitrary script-action access.
- Set/clear mode updates the current workspace/output pair through the core API
  without focusing that output. Switch/stretch/split first focus their validated
  output, then use the existing core command. Focus-window may reveal a hidden
  workspace and restores minimization. Additive v1 `maximized`/`minimized`/`fullscreen` fields
  keep hidden windows in snapshots. Validated `set_maximized`/`set_minimized`/`set_fullscreen`
  commands target normal windows without implicit workspace switches; launchers
  reject them atomically. Minimize repairs focus through core; restore preserves
  maximize state. Do not add separate shell policy or undo core focus decisions.
- Snapshot/subscribe execution is a no-op; the transport supplies their responses.
  Snapshot responses carry an ID and `state`; unsolicited `state` responses omit
  the ID. Error responses carry `message` and an ID or JSON null.
- `animation_panels` and `set_animation_targets` require live adapter ownership
  validation; pure `execute` rejects them. The transport supplies kernel peer
  credentials without exposing them on the wire. Panel registrations must expire
  and be removed on disconnect/unmap/geometry changes. Accepted reports do not
  alter focus, configures or desktop policy; the animation adapter consumes the
  resolved target when enabled minimize motion begins.
- Use Serde for all JSON encoding, including untrusted client metadata. Every
  encoded response is exactly one newline-terminated JSON message.

## Verification

Start with `cargo test --locked --test shell_ipc`, then the root Rust workflow.
Tests cover strict parsing, response framing/escaping, large IDs, hidden windows,
launcher roles, usable areas, groups, workspace-specific overrides, mode roundtrips,
read-only requests, error atomicity, and equivalence to core command behavior.
Model tests do not verify sockets or rendering. Run
`cargo test --locked --lib shell::server` for real Unix stream tests covering
permissions, kernel credentials, framing, fairness, partial writes, limits, and cleanup.
Run `cargo test --locked --lib shell::animation_targets` for advisory store
ownership, bounds, atomicity, lifecycle and expiry. Use
`scripts/vm-shell-smoke.py` for compositor integration and optionally add
`--quickshell quickshell` for real panels. Physical clicks/keys are not automated.
`scripts/vm-shell-hints-smoke.py --quickshell quickshell` checks real nonempty
example reports, foreign-process refusal and mapping identities after restart;
it does not verify visible minimize motion or physical scrolling.
When working in parallel, format only owned files.
