<!--
Sync Impact Report
- Version change: N/A → 1.0.0 (initial ratification)
- Added sections: All (initial constitution)
  - Principle I: Vulkan-First Rendering
  - Principle II: Zero-Copy Performance
  - Principle III: Client-Daemon Architecture
  - Principle IV: Wayland Protocol Compliance
  - Principle V: Minimal Complexity
  - Principle VI: Drop-In Compatibility
  - Principle VII: Safety & Correctness
  - Section: Performance Standards
  - Section: Development Workflow
  - Governance
- Removed sections: None
- Templates requiring updates:
  - .specify/templates/plan-template.md ✅ no changes needed (generic)
  - .specify/templates/spec-template.md ✅ no changes needed (generic)
  - .specify/templates/tasks-template.md ✅ no changes needed (generic)
- Follow-up TODOs: None
-->

# swww-vulkan Constitution

## Core Principles

### I. Vulkan-First Rendering

All rendering MUST use the Vulkan API directly. No software
rasterization, no OpenGL fallbacks, no wlr shared memory buffers
for frame presentation. The Vulkan backend is the sole rendering
path.

- Image compositing, transitions, and animation blending MUST
  execute on the GPU via Vulkan compute or graphics pipelines.
- Swapchain management MUST use `VK_KHR_wayland_surface` for
  direct Wayland integration.
- Shader pipelines MUST be pre-compiled to SPIR-V at build time;
  runtime shader compilation is prohibited.

**Rationale**: GPU-accelerated rendering eliminates CPU bottlenecks
present in the original swww's software blitting path, enabling
smooth animations at high refresh rates with minimal CPU usage.

### II. Zero-Copy Performance

Data movement between CPU and GPU MUST be minimized at every layer.

- Decoded image data MUST be uploaded to GPU memory via staging
  buffers with DMA where supported.
- Animation frames MUST remain in GPU memory once uploaded; frame
  re-upload per display cycle is prohibited.
- IPC transfers from client to daemon MUST use shared memory
  (`memfd`/`mmap`) rather than socket byte streams for payloads
  exceeding 64 KiB.
- Transition effects MUST execute entirely on the GPU; the CPU
  MUST NOT touch per-pixel data during transitions.

**Rationale**: Eliminating unnecessary copies is the primary
mechanism for achieving measurably lower latency and CPU usage
compared to the original swww.

### III. Client-Daemon Architecture

The system MUST maintain a strict separation between the CLI client
(`swww-vulkan`) and the long-running daemon (`swww-vulkan-daemon`).

- The daemon MUST be the sole owner of Vulkan resources and Wayland
  surfaces.
- The client MUST communicate with the daemon exclusively via IPC
  (Unix domain sockets with shared memory for bulk data).
- The daemon MUST support hot-reloading wallpapers without restart.
- Multiple outputs (monitors) MUST be managed by a single daemon
  instance, with per-output wallpaper state.

**Rationale**: This architecture mirrors the proven design of the
original swww while enabling the daemon to hold persistent GPU
state across wallpaper changes.

### IV. Wayland Protocol Compliance

The daemon MUST be a well-behaved Wayland client.

- Surface management MUST use the `wlr-layer-shell` protocol
  (unstable v1 or stable when available).
- The daemon MUST respect compositor-driven scaling (fractional
  scaling via `wp_fractional_scale_v1`).
- Frame presentation MUST synchronize with Wayland frame callbacks
  to avoid tearing and unnecessary GPU work.
- The daemon MUST handle output hotplug (add/remove) gracefully,
  including Vulkan swapchain recreation.

**Rationale**: Protocol compliance ensures compatibility across
Wayland compositors (Sway, Hyprland, river, etc.).

### V. Minimal Complexity

Every abstraction MUST justify its existence with a concrete,
current requirement.

- No wrapper types, helper functions, or trait abstractions for
  single-use operations.
- Vulkan resource management MUST use straightforward RAII patterns;
  no custom allocator frameworks unless profiling demonstrates need.
- Dependencies MUST be evaluated for necessity: prefer `ash` (raw
  Vulkan bindings) over higher-level Vulkan frameworks unless a
  framework eliminates significant verified complexity.
- New features MUST be accepted only if they are simple to implement
  and maintain relative to their user value.

**Rationale**: The original swww succeeded by staying focused.
This project MUST maintain that discipline while adding Vulkan
complexity only where it directly serves performance goals.

### VI. Drop-In Compatibility

The CLI interface MUST be compatible with the original swww where
reasonable.

- The `img`, `clear`, `query`, `restore`, `kill`, `pause`,
  and `clear-cache` subcommands MUST be supported with the same
  semantics.
- Transition types (`fade`, `wipe`, `grow`, `wave`, etc.) MUST
  be supported with compatible flag syntax.
- Image format support MUST be a superset of swww (JPEG, PNG, GIF,
  WebP, BMP, TIFF, PNM, TGA, Farbfeld, SVG; AVIF optional).
- Cache format MAY differ but MUST support migration from swww
  cache or graceful fallback.

**Rationale**: Users switching from swww MUST be able to adopt
swww-vulkan with minimal script changes.

### VII. Safety & Correctness

Unsafe code MUST be confined and justified.

- All Vulkan FFI calls MUST be wrapped in safe Rust abstractions
  at module boundaries.
- `unsafe` blocks MUST include a `// SAFETY:` comment explaining
  the invariant being upheld.
- The daemon MUST handle Vulkan device loss (`VK_ERROR_DEVICE_LOST`)
  by attempting recovery or clean shutdown—never by panicking.
- Memory-mapped IPC buffers MUST validate size and alignment before
  access.

**Rationale**: A wallpaper daemon runs for the lifetime of a
desktop session; crashes and undefined behavior are unacceptable.

## Performance Standards

Measurable performance targets that MUST be met before release:

- **Idle CPU usage**: < 0.1% CPU when displaying a static wallpaper
  (no active transitions or animations).
- **Transition frame rate**: Transitions MUST sustain the output's
  native refresh rate (e.g., 60 Hz, 144 Hz) without frame drops.
- **Wallpaper switch latency**: Time from client `img` command to
  first frame displayed MUST be < 100ms for a 4K JPEG image.
- **Memory overhead**: GPU memory usage MUST NOT exceed 2x the
  uncompressed framebuffer size per output for static wallpapers.
- **Animation memory**: GIF animation frames MUST be stored in GPU
  memory in a compressed or atlas format; full uncompressed frame
  storage per frame is prohibited for animations exceeding 64
  frames.

## Development Workflow

- **Language**: Rust (edition 2024, MSRV tracked in
  `rust-toolchain.toml`).
- **Build system**: Cargo workspace.
- **Vulkan bindings**: `ash` crate for raw Vulkan API access.
- **Testing**: `cargo test` for unit/integration tests; visual
  regression tests MAY use headless Vulkan rendering via
  `VK_EXT_headless_surface`.
- **CI**: All PRs MUST pass `cargo clippy -- -D warnings`,
  `cargo fmt --check`, and the full test suite.
- **Commit discipline**: Atomic commits with clear messages;
  force-pushes to shared branches are prohibited.

## Governance

This constitution is the authoritative reference for architectural
and design decisions in swww-vulkan. All code reviews and design
proposals MUST verify compliance with these principles.

- **Amendments**: Any change to this constitution MUST be documented
  with rationale, approved by the project maintainer, and include a
  migration plan for affected code if applicable.
- **Versioning**: The constitution follows semantic versioning:
  - MAJOR: Principle removal or incompatible redefinition.
  - MINOR: New principle or materially expanded guidance.
  - PATCH: Clarifications, wording, or non-semantic refinements.
- **Compliance review**: Each implementation plan MUST include a
  Constitution Check section verifying alignment with all active
  principles before work begins.
- **Complexity justification**: Any deviation from Principle V
  (Minimal Complexity) MUST be recorded in the plan's Complexity
  Tracking table with rejected alternatives.

**Version**: 1.0.0 | **Ratified**: 2026-03-11 | **Last Amended**: 2026-03-11
