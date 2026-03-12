# Feature Specification: Creative Transition Shaders

**Feature Branch**: `3-transition-shaders`
**Created**: 2026-03-12
**Status**: Draft
**Input**: User description: "Добавить новые необычные шейдеры переходов для смены обоев"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Use New Transition Effects (Priority: P1)

A user switches wallpapers and sees visually interesting transition effects beyond the existing basic set (fade, wipe, grow, wave, outer). New effects include pixelate (image dissolves into blocky pixels), swirl (spiral vortex distortion), blinds (venetian blinds opening), diamond (diamond-shaped reveal pattern), and dissolve (random noise-based pixel fade).

**Why this priority**: Core value of the feature — users want variety and visual appeal when switching wallpapers.

**Independent Test**: Can be fully tested by running `swww-vulkan img <path> --transition-type <new_type>` and observing the visual effect renders correctly and completes.

**Acceptance Scenarios**:

1. **Given** a wallpaper is displayed, **When** user runs `swww-vulkan img <path> --transition-type pixelate`, **Then** the old wallpaper dissolves into increasingly large blocks before revealing the new image
2. **Given** a wallpaper is displayed, **When** user runs `swww-vulkan img <path> --transition-type swirl`, **Then** the transition animates as a spiral vortex distortion from center
3. **Given** a wallpaper is displayed, **When** user runs `swww-vulkan img <path> --transition-type blinds`, **Then** horizontal or vertical slats reveal the new wallpaper like venetian blinds
4. **Given** a wallpaper is displayed, **When** user runs `swww-vulkan img <path> --transition-type diamond`, **Then** a diamond-shaped reveal pattern expands from center to edges
5. **Given** a wallpaper is displayed, **When** user runs `swww-vulkan img <path> --transition-type dissolve`, **Then** individual pixels randomly transition from old to new image using noise

---

### User Story 2 - New Effects Included in Random Selection (Priority: P1)

When using `--transition-type random`, all new transition effects are included in the random pool alongside existing ones, so users automatically experience variety without remembering effect names.

**Why this priority**: Most users use `random` as the default. New effects must be part of that pool to deliver value.

**Independent Test**: Can be tested by switching wallpapers repeatedly with `--transition-type random` and observing that new effects appear in the mix.

**Acceptance Scenarios**:

1. **Given** `--transition-type random` is used, **When** user switches wallpapers many times, **Then** new transition effects appear alongside existing ones
2. **Given** all transition types exist, **When** random selection occurs, **Then** each transition type has equal probability of being selected

---

### User Story 3 - Consistent Resize Behavior During Transitions (Priority: P1)

All new transition shaders must apply the same crop/fit/no-resize UV transformations as existing shaders, so images do not shift or change scaling when a transition completes.

**Why this priority**: This is a correctness requirement — scaling jumps break the user experience and were already fixed for existing shaders.

**Independent Test**: Can be tested by switching between images of different aspect ratios using each new transition type and verifying no visible shift occurs at transition end.

**Acceptance Scenarios**:

1. **Given** a wide image is displayed (21:9) with crop mode, **When** transitioning to a tall image (9:16) using any new effect, **Then** both images maintain correct crop scaling throughout the transition and after completion
2. **Given** fit resize mode is used, **When** transitioning with any new effect, **Then** letterboxing is consistent during and after the transition

---

### User Story 4 - Duration and Parameters Work with New Effects (Priority: P2)

All existing transition parameters (`--transition-duration`, `--transition-fps`, `--transition-bezier`) work correctly with new transition effects, giving users the same level of control.

**Why this priority**: Consistency with existing behavior — no special handling needed but must be verified.

**Independent Test**: Can be tested by running `swww-vulkan img <path> --transition-type pixelate --transition-duration 2.0` and confirming the effect takes 2 seconds.

**Acceptance Scenarios**:

1. **Given** `--transition-duration 2.0`, **When** using any new transition, **Then** the transition completes in exactly 2 seconds
2. **Given** `--transition-bezier .42,0,.58,1`, **When** using any new transition, **Then** the easing curve is applied correctly

---

### Edge Cases

- What happens when transition duration is very short (0.1s) with complex effects like swirl? Effect should still be visible as a brief flash.
- What happens with very large images (8K+)? Shader must not cause GPU stalls.
- What happens when rapidly switching between different new transition types? Descriptor sets and textures must be properly managed.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST provide at least 5 new transition shader effects: pixelate, swirl, blinds, diamond, dissolve
- **FR-002**: Each new transition MUST accept the existing push constant parameters (progress, angle, position, wave, resize mode, aspect ratios)
- **FR-003**: All new transitions MUST apply resize-mode UV transformations identically to the wallpaper shader (crop/fit/no-resize)
- **FR-004**: The `random` transition type MUST include all new effects in its selection pool with equal probability
- **FR-005**: The CLI MUST accept new transition type names as valid values for `--transition-type`
- **FR-006**: All new transitions MUST respect `--transition-duration`, `--transition-fps`, and `--transition-bezier` parameters
- **FR-007**: New transition shaders MUST be compiled to SPIR-V at build time alongside existing shaders
- **FR-008**: Each new transition MUST produce a visually complete result at progress=0.0 (show old image) and progress=1.0 (show new image)

### Key Entities

- **TransitionKind**: Extended enumeration of concrete transition effect types available for rendering
- **TransitionTypeArg**: CLI argument enum mapping user-facing names to internal transition kinds
- **Transition Shader**: Fragment shader implementing a specific visual effect for blending between two textures

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: All 5 new transition effects render correctly without visual artifacts
- **SC-002**: No transition effect causes a visible scaling shift when completing (resize consistency)
- **SC-003**: All transitions complete within their specified duration (±50ms tolerance)
- **SC-004**: Daemon remains stable through 50+ rapid wallpaper switches using new effects
- **SC-005**: Users can select any new effect by name via CLI or experience it through random selection
