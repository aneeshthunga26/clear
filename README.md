# Clear

Clear is an early tiling window manager experiment built on
[Smithay](https://github.com/Smithay/smithay). For now it runs Smithay's
Smallvil example code, renamed so the compositor state is `Clear`.

## Requirements

- Rust with Cargo
- A Wayland or X11 desktop session for the nested winit window
- Smithay's native dependencies, including Wayland, EGL/OpenGL, and xkbcommon
- A Wayland client to launch inside Clear

By default Clear tries to start `weston-terminal`. If you do not have it
installed, pass another client with `--command`.

## Run

Start Clear with the default client:

```sh
cargo run
```

Start Clear with a specific client:

```sh
cargo run -- --command foot
```

Other examples:

```sh
cargo run -- --command alacritty
cargo run -- --command weston-terminal
```

Clear opens a nested compositor window using Smithay's winit backend. The
launched client receives `WAYLAND_DISPLAY` pointing at Clear's socket, so it
runs inside the nested compositor rather than directly on your host compositor.

## App Launcher

Clear reads its config from `$XDG_CONFIG_HOME/clear/config.toml`, or
`~/.config/clear/config.toml` if `XDG_CONFIG_HOME` is not set.

The default launcher shortcut is `Super+Space`, where `Super` is the Windows
key on most keyboards. The default launcher command is `wofi --show drun`.

Example config:

```toml
[keys]
leader = "Super"

[shortcuts]
launcher = "leader+Space"

[apps]
launcher = "wofi --show drun"
launcher_app_id = "wofi"
```

Press `Super+Space` while Clear is running to open the launcher. Clear launches
the configured app inside the nested compositor and centers the launcher window
on the output.

## Logging

Enable Rust tracing logs with `RUST_LOG`:

```sh
RUST_LOG=debug cargo run -- --command foot
```
