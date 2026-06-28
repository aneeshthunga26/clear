# Agent Guidance

## Project Context

Clear is an early Smithay-based Wayland compositor. The code currently tracks
Smithay's Smallvil example closely, with project-specific changes layered on top
for configuration and launcher behavior.

Prefer small, local changes that keep the Smallvil structure recognizable unless
the task explicitly asks for a larger compositor architecture change.

## Rust Style

- Use idiomatic Rust 2024 and keep formatting under `cargo fmt`.
- Prefer explicit, simple data structures over premature abstraction.
- Preserve Smithay naming and handler patterns where possible.
- Keep behavior in the module that owns the relevant protocol or event flow:
  - input handling in `src/input.rs`
  - compositor state and process spawning in `src/state.rs`
  - XDG shell behavior in `src/handlers/xdg_shell.rs`
  - layer-shell/status bar behavior in `src/handlers/layer_shell.rs`
  - user config parsing in `src/config/mod.rs` with app-specific submodules

## Config Style

Clear reads user config from `$XDG_CONFIG_HOME/clear/config.toml`, falling back
to `~/.config/clear/config.toml`.

Keep the config organized around these top-level TOML sections:

```toml
[keys]
leader = "Super"

[shortcuts]
launcher = "leader+Space"
status_bar = "leader+Grave"

[apps.launcher]
command = "wofi --show drun --normal-window"
app_id = "wofi"

[apps.status_bar]
command = "waybar"
namespace = "waybar"
position = "top"
layer = "top"
exclusive = true
```

Missing or invalid config should fall back to defaults instead of preventing the
compositor from starting.

Layer-shell bars should live above normal windows. Keep the default status bar
layer as `top` unless there is a specific reason to test lower layers.

Keep wofi's default command in normal-window mode. Once Clear advertises
layer-shell for Waybar, plain wofi may choose layer-shell and bypass the XDG
launcher centering/toggle code.

## Comment Style

Use doc comments for public structs, fields, and methods that are part of
Clear's local behavior. Keep them short and focused on purpose:

```rust
/// Runtime config loaded from the user's XDG config file.
pub config: Config,
```

Use inline comments only when the reason is not obvious from the code, especially
for Wayland/Smithay timing, protocol state, or event suppression:

```rust
// A client that never saw the press should not see the release.
state.suppressed_launcher_key = None;
```

Avoid comments that restate assignments or control flow. Prefer explaining why a
guard exists, why a callback is needed, or why behavior is intentionally delayed.

## Verification

After Rust changes, run:

```sh
cargo fmt
cargo check
```

If the Rust toolchain proxy fails in the Cursor environment, use the system
Cargo path that has worked in this repo:

```sh
PATH=/usr/bin:/bin:$PATH /usr/bin/cargo fmt
PATH=/usr/bin:/bin:$PATH /usr/bin/cargo check
```

Also check diagnostics for edited Rust files before finishing.
