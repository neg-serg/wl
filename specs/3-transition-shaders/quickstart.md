# Quickstart: Creative Transition Shaders

## What this feature adds

5 new wallpaper transition effects: pixelate, swirl, blinds, diamond, dissolve.

## Files to modify

1. **5 new shader files** in `shaders/` — one `.frag` per effect
2. **common/src/ipc_types.rs** — add 5 variants to `TransitionType` enum
3. **client/src/cli.rs** — add 5 variants to `TransitionTypeArg` enum + `From` impl
4. **daemon/src/vulkan/pipeline.rs** — add 5 variants to `TransitionKind` enum
5. **daemon/src/transition.rs** — extend `resolve_kind()` mapping and `pick_random()` choices array
6. **daemon/src/main.rs** — register new shader modules in `frag_modules` array

## Build & test

```bash
# Build (shaders auto-compiled by build.rs)
cargo build --release

# Install
install -m 755 target/release/swww-vulkan ~/.local/bin/
install -m 755 target/release/swww-vulkan-daemon ~/.local/bin/

# Restart daemon
swww-vulkan kill; sleep 1; swww-vulkan init &

# Test individual effects
swww-vulkan img /path/to/image.png --transition-type pixelate
swww-vulkan img /path/to/image.png --transition-type swirl
swww-vulkan img /path/to/image.png --transition-type blinds
swww-vulkan img /path/to/image.png --transition-type diamond
swww-vulkan img /path/to/image.png --transition-type dissolve

# Test random includes new effects
for i in $(seq 1 20); do swww-vulkan img /path/to/image.png; done

# Stress test rapid switching
for i in $(seq 1 50); do swww-vulkan img /path/to/image.png --transition-type dissolve; done
```
