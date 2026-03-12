# Tasks: Creative Transition Shaders

**Input**: Design documents from `/specs/3-transition-shaders/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/

**Tests**: Not explicitly requested — test tasks omitted. Visual verification via CLI.

**Organization**: Tasks grouped by user story for independent implementation.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Extend enums and registration across all layers before writing shaders

- [x] T001 Add 5 new variants (Pixelate, Swirl, Blinds, Diamond, Dissolve) to `TransitionType` enum in common/src/ipc_types.rs
- [x] T002 Add 5 new variants to `TransitionTypeArg` enum and `From<TransitionTypeArg>` impl in client/src/cli.rs
- [x] T003 Add 5 new variants to `TransitionKind` enum in daemon/src/vulkan/pipeline.rs
- [x] T004 Extend `resolve_kind()` with 5 new mappings and add new variants to `pick_random()` choices array in daemon/src/transition.rs
- [x] T005 Register 5 new shader modules in `frag_modules` array in daemon/src/main.rs (lines ~161-170)
- [x] T006 Extend `TransitionKind` → `TransitionType` match in `create_transition()` in daemon/src/transition.rs

**Checkpoint**: Project compiles (shaders don't exist yet — build will fail until Phase 2)

---

## Phase 2: Foundational (Shader Template)

**Purpose**: No blocking prerequisites beyond Phase 1. All shaders share the same push constants layout and apply_resize function.

**⚠️ CRITICAL**: Each shader must include the full `TransitionPushConstants` layout and `apply_resize()` function matching existing shaders (see shaders/transition_fade.frag for reference).

**Checkpoint**: Foundation ready — all 5 shaders can be written in parallel

---

## Phase 3: User Story 1 — New Transition Effects (Priority: P1) 🎯 MVP

**Goal**: 5 new visually distinct transition effects selectable via `--transition-type`

**Independent Test**: `swww-vulkan img <path> --transition-type <name>` for each of the 5 new effects

### Implementation for User Story 1

- [x] T007 [P] [US1] Create pixelate transition shader in shaders/transition_pixelate.frag — progressive UV quantization to grid, coarsening then refining to reveal new image; sample old texture when progress < 0.5, new when >= 0.5
- [x] T008 [P] [US1] Create swirl transition shader in shaders/transition_swirl.frag — polar coordinate rotation increasing toward center with strength from wave_x, center from pos_x/pos_y; swirl peaks at progress=0.5 then unswirls to new image
- [x] T009 [P] [US1] Create blinds transition shader in shaders/transition_blinds.frag — divide screen into strips using fract(uv * num_strips) where num_strips derived from wave_x param; angle controls horizontal vs vertical; each strip reveals new image based on progress
- [x] T010 [P] [US1] Create diamond transition shader in shaders/transition_diamond.frag — Manhattan distance (L1 norm) reveal from center (pos_x, pos_y), expanding diamond shape; same structure as transition_grow.frag but with abs(dx)+abs(dy) instead of Euclidean distance
- [x] T011 [P] [US1] Create dissolve transition shader in shaders/transition_dissolve.frag — hash-based pseudo-random noise per pixel using integer hash of UV coordinates; pixel transitions when hash(uv) < progress

**Checkpoint**: All 5 effects render correctly. Test each: `swww-vulkan img <path> --transition-type pixelate|swirl|blinds|diamond|dissolve`

---

## Phase 4: User Story 2 — Random Selection Pool (Priority: P1)

**Goal**: New effects appear when using `--transition-type random`

**Independent Test**: Switch wallpapers 20+ times with `--transition-type random` and observe new effects in the mix

### Implementation for User Story 2

- [x] T012 [US2] Verify `pick_random()` in daemon/src/transition.rs includes all 10 TransitionKind variants in choices array (done in T004, verify equal probability)

**Checkpoint**: `for i in $(seq 1 20); do swww-vulkan img <path>; done` shows variety including new effects

---

## Phase 5: User Story 3 — Resize Consistency (Priority: P1)

**Goal**: No scaling jump when transition completes for any new effect

**Independent Test**: Switch between images of different aspect ratios (e.g., 21:9 → 9:16) using each new effect with crop mode

### Implementation for User Story 3

- [x] T013 [US3] Verify all 5 new shaders include correct `apply_resize()` function and apply it to both old_uv and new_uv before texture sampling (done during T007-T011, explicit verification pass)

**Checkpoint**: No visible scaling shift at transition end for any new effect with mismatched aspect ratios

---

## Phase 6: User Story 4 — Parameter Compatibility (Priority: P2)

**Goal**: Duration, FPS, and bezier easing work correctly with all new effects

**Independent Test**: `swww-vulkan img <path> --transition-type pixelate --transition-duration 2.0` completes in 2 seconds

### Implementation for User Story 4

- [x] T014 [US4] Verify all new shaders use `pc.progress` uniformly (not custom timing) ensuring duration/bezier compatibility — no code changes expected, verification only

**Checkpoint**: `--transition-duration 2.0` and `--transition-bezier .42,0,.58,1` work with all 5 new effects

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Build verification and stability testing

- [x] T015 Build project with `cargo build --release` and verify all shaders compile to SPIR-V
- [x] T016 Install and restart daemon, test all 5 effects individually
- [x] T017 Stress test: 50 rapid wallpaper switches using new effects to verify no crashes or descriptor pool exhaustion

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — enum and registration changes
- **Foundational (Phase 2)**: N/A — no separate foundational work needed
- **US1 (Phase 3)**: Depends on Phase 1 — write all 5 shaders
- **US2 (Phase 4)**: Depends on Phase 1 (T004) — verify random pool
- **US3 (Phase 5)**: Depends on Phase 3 — verify resize in shaders
- **US4 (Phase 6)**: Depends on Phase 3 — verify parameter usage
- **Polish (Phase 7)**: Depends on all phases complete

### User Story Dependencies

- **US1 (P1)**: Depends on Setup only — can start immediately after T001-T006
- **US2 (P1)**: Effectively complete after T004 — verify only
- **US3 (P1)**: Verification after US1 shaders written
- **US4 (P2)**: Verification after US1 shaders written

### Parallel Opportunities

- T001, T002, T003 can run in parallel (different files)
- T007, T008, T009, T010, T011 can ALL run in parallel (5 independent shader files)
- T012, T013, T014 are verification-only and can run in parallel after shaders exist

---

## Parallel Example: User Story 1

```bash
# Launch all 5 shader tasks together (different files, no dependencies):
Task: "Create pixelate shader in shaders/transition_pixelate.frag"
Task: "Create swirl shader in shaders/transition_swirl.frag"
Task: "Create blinds shader in shaders/transition_blinds.frag"
Task: "Create diamond shader in shaders/transition_diamond.frag"
Task: "Create dissolve shader in shaders/transition_dissolve.frag"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (T001-T006)
2. Complete Phase 3: Write all 5 shaders (T007-T011) — parallelizable
3. Build and test each effect individually
4. **STOP and VALIDATE**: All 5 effects render correctly

### Incremental Delivery

1. Setup → all enum extensions done
2. Write shaders (parallel) → 5 new effects available → MVP!
3. Verify random pool → new effects in random rotation
4. Verify resize consistency → no scaling jumps
5. Verify parameter compatibility → duration/bezier work
6. Stress test → stability confirmed

---

## Notes

- All 5 shader tasks (T007-T011) are fully parallel — different files, identical structure
- Verification tasks (T012-T014) require no code changes if shaders are written correctly
- Total implementation is ~6 files modified + 5 new files created
- Each shader follows the exact same template as existing transition_fade.frag
