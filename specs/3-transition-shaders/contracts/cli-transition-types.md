# Contract: CLI Transition Types

## Extended `--transition-type` values

The `img` subcommand's `--transition-type` argument accepts these new values in addition to existing ones:

```
swww-vulkan img [OPTIONS] <PATH>

--transition-type <TYPE>
  Existing: fade, wipe, grow, wave, outer, random, none
  New:      pixelate, swirl, blinds, diamond, dissolve
```

## Behavior Contract

| Type      | Visual Effect                                         | Uses angle | Uses pos | Uses wave_x |
|-----------|-------------------------------------------------------|------------|----------|-------------|
| pixelate  | Progressive pixelation → reveal                       | no         | no       | no          |
| swirl     | Spiral vortex distortion from center                  | yes        | yes      | yes (strength) |
| blinds    | Venetian blind strips reveal                          | yes (orientation) | no | yes (strip count) |
| diamond   | Diamond-shaped (Manhattan distance) reveal from center | no        | yes      | no          |
| dissolve  | Random per-pixel noise dissolve                       | no         | no       | no          |

## Invariants

- All new types produce identical output to existing types at progress boundaries:
  - `progress = 0.0`: shows old image only
  - `progress = 1.0`: shows new image only
- All new types are included in `random` selection pool
- All new types respect `--transition-duration`, `--transition-fps`, `--transition-bezier`
- All new types apply resize-mode UV transforms (crop/fit/no-resize)
