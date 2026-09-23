# Rhai extensions

Parent guidance: [src/AGENTS.md](../AGENTS.md).

## Responsibilities and boundaries

`mod.rs` owns the bounded Rhai engine, compiled script host, conversion between
plain script values and typed layouts/actions, and result validation. Rhai is
the chosen extension language; do not add Lua or JavaScript runtimes.

- Expose copied IDs, metadata, geometry, and declarative actions—not desktop
  references, native handles, renderer objects, or direct state mutation.
- Compile scripts once per load/reload, not per frame. Top-level statements are
  not executed; do not rely on them to initialize user-visible state.
- Preserve operation, recursion, and collection limits and restrictions on
  imports, evaluation, and native I/O. These are in-process safeguards, not a
  hostile-code security boundary.
- Validate layout result membership, uniqueness, integer ranges, geometry, and
  completeness before accepting it. Current custom layouts are output-bounded;
  saved floating windows are a separate core policy.
- Validate action results atomically; malformed lists must not partially execute.
- Return actionable errors to runtime. Runtime owns disabling failed functions
  until reload and choosing fallback layouts.
- Keep `examples/columns.rhai` and documented script contracts synchronized when
  changing exposed values or function signatures.

## Verification

Start with `cargo test --locked --test extensions --test runtime`, then the root
workflow. Include malformed results, resource limits, prohibited operations,
and recovery after script errors. For real-client integration, run the VM smoke
with `--script examples/columns.rhai` as described in `docs/vm-testing.md`.
