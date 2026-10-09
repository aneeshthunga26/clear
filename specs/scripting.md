# Rhai extensions

## Execution model

Rhai is the extension language for layouts and typed actions. Scripts exchange
copied maps, arrays, IDs, and integer geometry. They MUST NOT receive compositor,
surface, seat, or renderer objects, or directly mutate desktop state. The host
compiles functions once per load; top-level statements are never executed and
calls use a fresh scope. Imports, exports, `eval`, native IO, and module resolution
are unavailable; print/debug output is discarded.

These in-process limits bound ordinary extension work; they are not a hostile-code
security boundary. Runtime invokes scripts during policy reconciliation, not
directly from painting.

## Layout interface

`script:NAME` calls `NAME(ctx)` with:

| Field | Value |
| --- | --- |
| `area` | `{x, y, width, height}` usable region |
| `windows` | Ordered `[{id, rect: {x, y, width, height}}, ...]` |
| `focused` | A participant's integer ID, or Rhai unit |
| `gaps` | Integer 0–4096 |
| `scroll_offset` | Integer with absolute value at most 1000000 |

The supplied window rectangle is saved floating geometry and may be unconfigured
or outside the region. A script must initialize its requested layout as needed.
Float exceptions, launchers, maximized windows, and minimized windows bypass this
interface. Empty window lists return an empty result without executing the script.

The result MUST be an array with exactly one map per input window, using exactly
`window`, `x`, `y`, `width`, and `height`. IDs must be input members and occur once.
Geometry MUST be integer-valued, fit `i32`, have positive dimensions, lie within
the context area, and have all edges within ±1000000. Context window IDs must fit
Rhai's signed integer range and be unique; focused ID must belong to the context.
Unknown fields, floats substituted for integers, missing windows, duplicates,
foreign IDs, and out-of-area placements MUST fail validation. Overlap is not
itself rejected.

The adapter receives tiled placements with the region clip. Core rechecks complete
membership, normalizes rectangles, and derives focus/tiled flags itself; a custom
callback cannot forge those flags.

## Action interface

An action function takes no arguments and returns an array of action maps using
the [binding action schema](input.md#action-schema), without `key` fields.
`alt_tab` is not supported by the Rhai decoder: its gesture belongs to physical
input. `toggle_overview` is supported as a request for adapter authorization
under the [overview contract](overview.md). All returned maps are decoded and validated before any is dispatched.
Malformed results fail as a whole. Successful actions are applied in returned
order; nested script dispatch shares runtime's 128-action expansion budget.

Function names begin with an ASCII letter or underscore and continue with ASCII
letters, digits, or underscores, at most 128 bytes.

## Limits and failure handling

| Resource | Limit |
| --- | --- |
| UTF-8 source | 256 KiB |
| Layout participants | 1024 |
| Returned actions per call | 128 |
| Engine operations | 100000 |
| Call depth | 32 |
| Expression depth | 64 globally, 32 in functions |
| String size | 16384 |
| Array size | 4096 |
| Engine map budget | 16384 entries, including nested DTO maps |
| Variables / functions | 256 / 128 |
| Guarded map entries | 256 per map |
| Value traversal | Depth 32 and budget 16384 |
| Execution deadline | 250 ms, checked by the progress callback every 1024 operations |

The deadline is cooperative, not a hard real-time preemption guarantee. Source
read, compile, execution, and result-validation failures are reported to runtime.
A failed function name is disabled until successful reload. Layout failures or
absent hosts use master/stack fallback; action failures apply no returned actions.
Core has an additional membership-validation fallback even for non-Rhai custom
callbacks. Startup and reload behavior follow [configuration](configuration.md).

## Implementation and evidence

- [Script host and decoder](../src/scripting/mod.rs),
  [runtime routing](../src/runtime/mod.rs), [core fallback](../src/core/desktop.rs).
- [Extension tests](../tests/extensions.rs): malformed placements/actions,
  invalid contexts, limits, unavailable IO/imports, skipped globals, and host
  recovery after failures.
- [Runtime tests](../tests/runtime.rs) and [core tests](../tests/core.rs): disabled
  functions, bounded expansion, fallback, and exclusion of nonparticipants.
- [Example layout](../examples/columns.rhai) and
  [Rhai smoke path](../scripts/vm-smoke.py) provide integration references.
