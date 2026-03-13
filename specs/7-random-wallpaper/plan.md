# Implementation Plan: random-wallpaper

**Branch**: `7-random-wallpaper` | **Date**: 2026-03-13 | **Spec**: spec.md
**Input**: Feature specification from `/specs/7-random-wallpaper/spec.md`

## Summary

Add a `random` subcommand to the `wl` CLI client that recursively scans user-specified directories for image files, picks one at random, and applies it as the wallpaper by sending the standard `IpcCommand::Img` to the daemon. Optional post-apply hooks (greeter cache sync, shell notification file) are configurable and enabled by default. No daemon changes required — the feature is entirely client-side, reusing the existing `img` command's IPC path.

## Technical Context

**Language/Version**: Rust edition 2024, stable toolchain
**Primary Dependencies**: clap (CLI), walkdir (recursive dir scan), getrandom (random selection), bincode (IPC serialization)
**Storage**: Filesystem — reads image directories, writes greeter cache file and notification path file
**Testing**: cargo test (unit tests for directory scanning, extension filtering, option parsing)
**Target Platform**: Linux (Wayland)
**Project Type**: CLI tool (client-side subcommand addition)
**Performance Goals**: Directory scan + wallpaper apply in < 2 seconds for typical collections (< 10k files)
**Constraints**: No daemon changes; reuse existing IPC protocol and transition infrastructure
**Scale/Scope**: Single new subcommand with ~300-500 lines of new client code

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Vulkan-First Rendering | PASS | No rendering changes — uses existing daemon pipeline |
| II. Zero-Copy Performance | PASS | No data path changes — client picks a file, daemon handles upload |
| III. Client-Daemon Architecture | PASS | Feature is client-side only, communicates via existing IPC |
| IV. Wayland Protocol Compliance | PASS | No Wayland protocol changes |
| V. Minimal Complexity | PASS | Single subcommand, reuses existing img flow, no new abstractions |
| VI. Drop-In Compatibility | PASS | New subcommand — does not modify existing command semantics |
| VII. Safety & Correctness | PASS | No unsafe code needed; standard file I/O operations |

All gates pass. No complexity violations.

## Project Structure

### Documentation (this feature)

```text
specs/7-random-wallpaper/
├── plan.md              # This file
├── research.md          # Phase 0: technology decisions
├── data-model.md        # Phase 1: entity definitions
├── contracts/           # Phase 1: CLI contract
│   └── cli-random.md
└── tasks.md             # Phase 2 output (created by /speckit.tasks)
```

### Source Code (repository root)

```text
client/
├── Cargo.toml           # Add walkdir dependency
└── src/
    ├── cli.rs           # Add Commands::Random variant with options
    ├── main.rs          # Add Random command handler dispatch
    └── random.rs        # NEW: directory scanning, image selection, post-hooks

common/
└── src/
    └── (no changes)     # Reuse existing IpcCommand::Img

daemon/
└── src/
    └── (no changes)     # Daemon receives IpcCommand::Img as usual
```

**Structure Decision**: Client-only change. One new file `client/src/random.rs` for the scanning/selection/hooks logic. CLI definition added to existing `cli.rs`. Handler wired in existing `main.rs`.
