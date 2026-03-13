# wl

A Vulkan-accelerated wallpaper daemon for Wayland compositors. Spiritual successor to [swww](https://github.com/LGFae/swww) with GPU-powered rendering.

## Features

- **Vulkan rendering** — all compositing, transitions, and animations run on the GPU
- **Animated wallpapers** — GIF support with GPU-resident frame atlases
- **Smooth transitions** — fade, wipe, grow, wave, and outer effects at native refresh rate
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
wl img /path/to/wallpaper.png --transition-type fade --transition-duration 1.5

# Set a solid color
wl clear '#1e1e2e'

# Query current state
wl query

# Stop the daemon
wl kill
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
