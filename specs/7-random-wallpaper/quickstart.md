# Quickstart: random-wallpaper

**Branch**: `7-random-wallpaper` | **Date**: 2026-03-13

## Prerequisites

- Rust stable toolchain (edition 2024)
- Working wl build environment
- Wayland compositor running (for testing)

## Files to Create/Modify

| File | Action | Purpose |
|------|--------|---------|
| `client/Cargo.toml` | Modify | Add `walkdir` dependency |
| `client/src/cli.rs` | Modify | Add `Commands::Random` variant |
| `client/src/main.rs` | Modify | Add handler dispatch for Random |
| `client/src/random.rs` | Create | Directory scanning, selection, hooks |

## Build & Test

```bash
# Build
cargo build

# Test
cargo test

# Manual test (requires running Wayland session)
cargo run --bin wl -- random ~/pic/wl ~/pic/black

# Test with hooks disabled
cargo run --bin wl -- random --no-greeter-sync --no-notify ~/Pictures
```

## Implementation Order

1. Add `walkdir` to `client/Cargo.toml`
2. Define `Commands::Random` in `cli.rs` with all options
3. Create `random.rs` with:
   - `scan_directories()` — recursive walk + extension filter
   - `pick_random()` — getrandom-based selection
   - `run_hooks()` — greeter sync + notification
4. Wire up in `main.rs`: scan → pick → ensure daemon → send IpcCommand::Img → hooks
5. Add unit tests for scanning and filtering logic
