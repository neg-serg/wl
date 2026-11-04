# wl

A Vulkan-accelerated wallpaper daemon for Wayland compositors. Spiritual successor to [swww](https://github.com/LGFae/swww) with GPU-powered rendering.

## Features

- **Vulkan rendering** — all compositing, transitions, and animations run on the GPU
- **Animated wallpapers** — GIF support with GPU-resident frame atlases
- **Smooth transitions** — 25 effects (wipe, wave, burn, glitch, fluid*, morph, …) at native refresh rate
- **Multi-monitor** — per-output wallpaper and independent transitions
- **Fractional scaling** — respects `wp_fractional_scale_v1`
- **Drop-in compatible** — same CLI interface as swww (`img`, `clear`, `query`, `restore`, etc.)
- **Image formats** — JPEG, PNG, GIF, WebP, BMP, TIFF, PNM, TGA, Farbfeld, SVG

## Requirements

- Rust stable (edition 2024)
- Vulkan 1.0+ driver with `VK_KHR_wayland_surface`
- Wayland compositor with `wlr-layer-shell` support (Sway, Hyprland, river, etc.)
- `glslc` (from the Vulkan SDK or `shaderc`) for shader compilation

## Building

```sh
cargo build --release
```

Binaries are placed in `target/release/`:
- `wl` — CLI client
- `wl-daemon` — background daemon

## Usage

```sh
# Start the daemon
wl init

# Set a wallpaper
wl img /path/to/wallpaper.jpg

# Set with a transition
wl img /path/to/wallpaper.png --transition-type wave --transition-duration 1.5

# Set a solid color
wl clear '#1e1e2e'

# Rotate wallpapers from directories, upscaling each image in the background
# (needs realesrgan-ncnn-vulkan, or pass --upscale-cmd)
wl rotate start ~/pics ~/more --interval 30m --upscale always
wl rotate status --json
wl rotate stop

# Query current state
wl query

# Machine-readable state for scripts/status bars
wl query --json

# Stop the daemon
wl kill
```

## Development

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The daemon crate needs `pkg-config`, `libwayland-client` and `glslc` (shaders are
compiled by `daemon/build.rs`). On NixOS the environment is not inherited, so
point the build and the test binaries at the store paths explicitly:

```sh
export PKG_CONFIG_PATH="<wayland-dev>/lib/pkgconfig:<vulkan-loader-dev>/lib/pkgconfig:<libffi-dev>/lib/pkgconfig"
export PATH="<pkgconf>/bin:<shaderc-bin>/bin:$PATH"        # provides pkg-config / glslc
export LD_LIBRARY_PATH="<wayland>/lib:<vulkan-loader>/lib:<glibc>/lib"  # for cargo test
```

`wl-daemon` also ships a GPU integration test that uploads a two-frame GIF atlas
and reads the pixels back, verifying each atlas tile holds its own frame. It is
ignored by default because it needs a live Wayland session and a Vulkan device:

```sh
cargo test -p wl-daemon gif_atlas_tiles -- --ignored --nocapture
```

## Architecture

```
┌─────────────┐    IPC (Unix socket)    ┌──────────────────┐
│  wl │ ◄────────────────────► │ wl-daemon│
│   (client)   │                        │    (Vulkan +      │
└─────────────┘                        │     Wayland)      │
                                        └──────────────────┘
```

- **client** — CLI that sends commands over Unix domain sockets
- **daemon** — long-running process owning Vulkan resources and Wayland surfaces
- **common** — shared IPC types, image decoding, and cache management

## License

GPL-3.0 — see [LICENSE](LICENSE).
