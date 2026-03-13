# CLI Contract: `wl random`

**Branch**: `7-random-wallpaper` | **Date**: 2026-03-13

## Command Synopsis

```
wl random [OPTIONS] <DIRECTORY>...
```

## Positional Arguments

| Argument | Required | Description |
|----------|----------|-------------|
| `DIRECTORY` | Yes (1+) | Directories to scan recursively for image files |

## Options — Random-Specific

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--no-greeter-sync` | bool flag | false | Disable copying wallpaper to greeter cache |
| `--greeter-path <PATH>` | path | `~/.cache/greeter-wallpaper` | Custom greeter cache file path |
| `--no-notify` | bool flag | false | Disable writing wallpaper path to notification file |
| `--notify-path <PATH>` | path | `~/.cache/quickshell-wallpaper-path` | Custom notification file path |

## Options — Inherited from `img`

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--outputs <NAMES>` | comma-separated | all outputs | Target output names |
| `--resize <MODE>` | crop/fit/no | crop | Image scaling strategy |
| `--transition-type <TYPE>` | enum | fade | Transition effect |
| `--transition-duration <SECS>` | f32 | 0.5 | Transition duration |
| `--transition-step <N>` | u8 (1-255) | 90 | Frame step |
| `--transition-fps <N>` | u32 | 240 | Target FPS |
| `--transition-angle <DEG>` | f32 | 45.0 | Wipe angle |
| `--transition-pos <POS>` | x,y or name | center | Grow origin |
| `--transition-bezier <A,B,C,D>` | 4×f32 | .25,.1,.25,1 | Timing curve |
| `--transition-wave <F,A>` | 2×f32 | 20,20 | Wave params |
| `--upscale <MODE>` | once/always/never/off | (from prefs) | Upscaling mode |
| `--upscale-cmd <CMD>` | string | realesrgan-ncnn-vulkan | Custom upscaler |
| `--upscale-scale <N>` | 2/4/8/16 | auto | Force scale factor |

## Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Wallpaper applied successfully |
| 1 | No image files found in specified directories |
| 1 | All specified directories are invalid/missing |
| 1 | Daemon connection/communication failure |
| 1 | Image application failure (daemon error) |

## Behavior

1. Validate that at least one directory argument is provided
2. Recursively scan all directories for files with supported extensions (bmp, gif, hdr, ico, jpg, jpeg, png, tif, tiff, webp)
3. Warn on stderr for directories that don't exist (continue scanning others)
4. If no candidates found, print error to stderr and exit 1
5. Select one candidate at random
6. Ensure daemon is running (auto-start if needed)
7. Send `IpcCommand::Img` with the selected path and all transition/resize/upscale options
8. On success, execute enabled post-hooks:
   - Greeter sync: `cp <selected> <greeter-path>` (create parent dirs if needed)
   - Notification: write absolute path as text to `<notify-path>` (create parent dirs if needed)
9. Print selected wallpaper path to stdout
10. Exit 0

## Output

**stdout**: The absolute path of the selected wallpaper (one line)
**stderr**: Warnings for missing directories, errors on failure

## Examples

```bash
# Basic usage — scan two directories
wl random ~/pic/wl ~/pic/black

# Custom transition
wl random --transition-type wave --transition-duration 1.5 ~/Pictures

# Disable all hooks
wl random --no-greeter-sync --no-notify ~/Wallpapers

# Custom hook paths
wl random --greeter-path /tmp/greeter.png --notify-path /tmp/wp-path.txt ~/pic

# Specific output only
wl random --outputs eDP-1 ~/Pictures/landscape
```
