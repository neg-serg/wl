# Research: Creative Transition Shaders

## R1: Pixelate Effect Algorithm

**Decision**: Progressive pixelation by quantizing UV coordinates to a grid that starts fine and coarsens over time, then reverses to reveal the new image.

**Rationale**: Standard approach used in game engines and video editing. The grid cell size is derived from `progress`: at progress=0 cells are 1px (no effect), at progress=0.5 cells are at maximum size (most pixelated), then they shrink back revealing the new image.

**Algorithm**: `floor(uv * grid_size) / grid_size` where `grid_size = mix(max_grid, min_grid, abs(2.0 * progress - 1.0))`. Sample old texture when progress < 0.5, new texture when >= 0.5.

**Alternatives considered**: Voronoi-based pixelation (too expensive), simple crossfade with pixelation (less visually interesting).

## R2: Swirl Effect Algorithm

**Decision**: Polar coordinate rotation where the angle of rotation increases toward the center and varies with progress. At progress=0.5, maximum swirl distortion; it unswirls to reveal the new image.

**Rationale**: Classic GPU distortion effect. Cheap to compute — just UV coordinate math in polar space using `atan`/`length`.

**Algorithm**: Convert UV to polar around (pos_x, pos_y), add rotation `angle_offset = strength * (1.0 - dist/radius) * sin(progress * PI)`, convert back to cartesian. Sample old when progress < 0.5, new when >= 0.5.

**Alternatives considered**: Turbulence-based swirl (more complex, no visual benefit at wallpaper scale).

## R3: Blinds Effect Algorithm

**Decision**: Divide screen into N horizontal or vertical strips. Each strip reveals the new image by sliding open like venetian blinds. The `angle` parameter controls orientation (0° = horizontal blinds, 90° = vertical blinds).

**Rationale**: Simple and visually distinctive. Uses `fract()` to create repeating strips, then compares strip-local position against progress.

**Algorithm**: `float strip = fract(uv.y * num_strips)` (or uv.x for vertical). `float mask = step(strip, progress)`. Mix old/new based on mask. `num_strips` can be derived from `wave_x` parameter (reuse existing push constant).

**Alternatives considered**: Rotating blinds (3D perspective needed, too complex for 2D shader).

## R4: Diamond Effect Algorithm

**Decision**: Manhattan-distance based reveal (L1 norm) from center, creating a diamond/rhombus shaped expansion.

**Rationale**: Visually distinct from the existing `grow` (which uses Euclidean/L2 distance for circular reveal). Same algorithmic complexity, different distance metric produces diamond shape.

**Algorithm**: `float dist = abs(uv.x - pos_x) + abs(uv.y - pos_y)`. Same structure as grow.frag but with L1 distance.

**Alternatives considered**: Rotated square (same visual result, more math). Hexagonal distance (not as clean).

## R5: Dissolve Effect Algorithm

**Decision**: Hash-based pseudo-random noise per pixel. Each pixel transitions at a different time based on its noise value compared to progress.

**Rationale**: Classic dissolve effect used in games and film. GPU-friendly since noise is computed from UV coordinates — no texture lookups or state needed.

**Algorithm**: `float noise = hash(uv)` using a simple integer hash function on pixel coordinates. `float mask = step(noise, progress)`. Mix old/new based on mask.

**Alternatives considered**: Perlin noise dissolve (smoother but more expensive, and the "salt-and-pepper" style is more visually distinctive). Blue noise (requires a noise texture upload).

## R6: Push Constants Reuse

**Decision**: All 5 new shaders use the existing `TransitionPushConstants` layout unchanged. No new push constants needed.

**Rationale**: The existing layout has all fields needed:
- `progress`: animation progress (all effects)
- `angle`: blinds orientation, swirl direction
- `pos_x`, `pos_y`: center point for swirl, diamond
- `wave_x`, `wave_y`: strip count for blinds, swirl strength
- `old/new_resize_mode`, `old/new_img_aspect`, `screen_aspect`: resize UV transforms

**Alternatives considered**: Adding effect-specific parameters (unnecessary complexity; existing params can be repurposed per-effect).
