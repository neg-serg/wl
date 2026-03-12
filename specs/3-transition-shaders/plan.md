# Implementation Plan: Creative Transition Shaders

**Branch**: `3-transition-shaders` | **Date**: 2026-03-12 | **Spec**: spec.md

## Summary

Add 5 new GPU fragment shaders (pixelate, swirl, blinds, diamond, dissolve) to the transition pipeline, extending the existing `TransitionKind` enum, CLI argument parser, random selection pool, and shader build system. Each shader reuses the existing `TransitionPushConstants` layout with resize-mode UV transforms.

## Technical Context

**Language/Version**: Rust 2024 edition, GLSL 4.50
**Primary Dependencies**: ash (Vulkan bindings), glslc (SPIR-V compiler)
**Storage**: N/A
**Testing**: cargo test, visual verification via wf-recorder
**Target Platform**: Linux (Wayland compositors)
**Project Type**: CLI + daemon (Vulkan wallpaper renderer)
**Performance Goals**: Transitions at native refresh rate (60-144 Hz), no frame drops
**Constraints**: Shaders pre-compiled to SPIR-V at build time, no runtime compilation

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Vulkan-First Rendering | PASS | All transitions execute as GPU fragment shaders via Vulkan graphics pipeline |
| II. Zero-Copy Performance | PASS | Transitions execute entirely on GPU; CPU touches no per-pixel data |
| III. Client-Daemon Architecture | PASS | Client sends transition type via IPC; daemon owns shader pipelines |
| IV. Wayland Protocol Compliance | PASS | No Wayland protocol changes; uses existing swapchain presentation |
| V. Minimal Complexity | PASS | Each shader is a self-contained .frag file; no new abstractions needed. Only extends existing enums. |
| VI. Drop-In Compatibility | PASS | Extends `--transition-type` with new values; existing values unchanged |
| VII. Safety & Correctness | PASS | No new unsafe code; shaders share existing push constants structure |

No violations. Complexity Tracking table not needed.

## Project Structure

### Documentation (this feature)

```text
specs/3-transition-shaders/
├── plan.md
├── research.md
├── data-model.md
├── contracts/
│   └── cli-transition-types.md
└── tasks.md
```

### Source Code (repository root)

```text
shaders/
├── transition_pixelate.frag    # NEW
├── transition_swirl.frag       # NEW
├── transition_blinds.frag      # NEW
├── transition_diamond.frag     # NEW
├── transition_dissolve.frag    # NEW
├── transition_fade.frag        # existing
├── transition_grow.frag        # existing
├── transition_outer.frag       # existing
├── transition_wave.frag        # existing
├── transition_wipe.frag        # existing
├── wallpaper.frag
└── wallpaper.vert

client/src/cli.rs               # MODIFY: add 5 variants to TransitionTypeArg
common/src/ipc_types.rs          # MODIFY: add 5 variants to TransitionType
daemon/src/vulkan/pipeline.rs    # MODIFY: add 5 variants to TransitionKind
daemon/src/transition.rs         # MODIFY: extend resolve_kind and pick_random
daemon/src/main.rs               # MODIFY: register new shader modules
```

**Structure Decision**: Shaders go in existing `shaders/` directory following `transition_<name>.frag` convention. Rust changes are enum extensions in existing files.
