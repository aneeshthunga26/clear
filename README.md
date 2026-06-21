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

## Logging

Enable Rust tracing logs with `RUST_LOG`:

```sh
RUST_LOG=debug cargo run -- --command foot
```
