# Tasks: Random Wallpaper Command

**Input**: Design documents from `/specs/7-random-wallpaper/`
**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Add dependency and create module skeleton

- [x] T001 Add `walkdir` and `getrandom` dependencies to `client/Cargo.toml`
- [x] T002 Create empty `client/src/random.rs` module file and add `mod random;` to `client/src/main.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Define CLI args and shared helpers that all user stories depend on

**CRITICAL**: No user story work can begin until this phase is complete

- [x] T003 Add `Commands::Random` variant to the `Commands` enum in `client/src/cli.rs` with: positional `directories` arg (Vec<PathBuf>, required, 1+), all inherited `img` transition/resize/upscale options (reuse or flatten existing arg definitions), `--no-greeter-sync` flag, `--greeter-path` option (default `~/.cache/greeter-wallpaper`), `--no-notify` flag, `--notify-path` option (default `~/.cache/quickshell-wallpaper-path`)
- [x] T004 Implement `scan_directories(dirs: &[PathBuf]) -> Vec<PathBuf>` in `client/src/random.rs` — recursively walk each directory using `walkdir`, filter files by supported extensions (bmp, gif, hdr, ico, jpg, jpeg, png, tif, tiff, webp, case-insensitive), warn on stderr for missing/unreadable directories, return collected absolute paths
- [x] T005 Implement `pick_random(candidates: &[PathBuf]) -> &Path` in `client/src/random.rs` — use `getrandom` to fill a `[u8; 8]` buffer, convert to u64, modulo candidate count to select index

**Checkpoint**: CLI parsing works (`--help` shows random subcommand), directory scanning and random pick are ready

---

## Phase 3: User Story 1 — Set a Random Wallpaper (Priority: P1) MVP

**Goal**: User runs `wl random <dirs>` and a randomly selected wallpaper is applied with a transition

**Independent Test**: Run `wl random ~/pic/wl` with a Wayland session and daemon running — a random wallpaper appears

### Implementation for User Story 1

- [x] T006 [US1] Add `Commands::Random` match arm in `client/src/main.rs` — wire up the full flow: call `scan_directories()`, error-exit if empty, call `pick_random()`, ensure daemon is running (reuse existing `daemon::init()` pattern from the `Img` handler), build `IpcCommand::Img` from the selected path and all transition/resize/upscale options, send via `IpcClient`, handle response, print selected path to stdout
- [x] T007 [US1] Handle edge cases in `client/src/random.rs` — when all directories are missing/invalid exit with descriptive error, when no image files found exit with code 1 and message "no image files found in specified directories"
- [x] T008 [US1] Integrate upscale resolution logic in `client/src/main.rs` — reuse `resolve_upscale()` from the existing `Img` handler so `--upscale` flags work with the random command identically to `img`

**Checkpoint**: `wl random ~/pic/wl ~/pic/black` applies a random wallpaper with transitions. All `img` options (transition, resize, upscale) work. MVP complete.

---

## Phase 4: User Story 2 — Greeter Wallpaper Sync (Priority: P2)

**Goal**: After applying a wallpaper, copy the image file to a greeter cache location (configurable, disableable)

**Independent Test**: Run with greeter sync enabled — verify `~/.cache/greeter-wallpaper` is updated. Run with `--no-greeter-sync` — verify file is not touched.

### Implementation for User Story 2

- [x] T009 [US2] Implement `greeter_sync(source: &Path, dest: &Path) -> Result<()>` in `client/src/random.rs` — create parent directories if needed, copy file to destination path, report errors to stderr without aborting (non-fatal hook)
- [x] T010 [US2] Wire greeter sync into the `Commands::Random` handler in `client/src/main.rs` — after successful wallpaper application, if `--no-greeter-sync` is NOT set, call `greeter_sync()` with the selected wallpaper path and the `--greeter-path` value

**Checkpoint**: Greeter sync works when enabled, is skipped when disabled, custom path works.

---

## Phase 5: User Story 3 — Shell/Desktop Integration Notification (Priority: P3)

**Goal**: After applying a wallpaper, write the selected image's absolute path to a notification file (configurable, disableable)

**Independent Test**: Run with notification enabled — verify `~/.cache/quickshell-wallpaper-path` contains the path. Run with `--no-notify` — verify file is not touched.

### Implementation for User Story 3

- [x] T011 [US3] Implement `write_notify(wallpaper_path: &Path, notify_file: &Path) -> Result<()>` in `client/src/random.rs` — create parent directories if needed, write absolute path as UTF-8 text (no trailing newline, matching original script behavior), report errors to stderr without aborting (non-fatal hook)
- [x] T012 [US3] Wire notification into the `Commands::Random` handler in `client/src/main.rs` — after successful wallpaper application (and after greeter sync if enabled), if `--no-notify` is NOT set, call `write_notify()` with the selected wallpaper path and the `--notify-path` value

**Checkpoint**: Notification file works when enabled, is skipped when disabled, custom path works.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Final validation and cleanup

- [x] T013 Verify `cargo clippy -- -D warnings` passes with all new code in `client/src/random.rs` and `client/src/cli.rs`
- [x] T014 Verify `cargo fmt --check` passes for all modified files
- [x] T015 Run end-to-end validation per `specs/7-random-wallpaper/quickstart.md` — test basic usage, hooks enabled, hooks disabled, custom paths, missing directories

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Phase 1 completion — BLOCKS all user stories
- **User Story 1 (Phase 3)**: Depends on Phase 2 — core wallpaper flow
- **User Story 2 (Phase 4)**: Depends on Phase 2 only (greeter sync is independent of US1 at code level, but logically runs after wallpaper apply)
- **User Story 3 (Phase 5)**: Depends on Phase 2 only (notification is independent of US1/US2 at code level)
- **Polish (Phase 6)**: Depends on all user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) — no dependencies on other stories
- **User Story 2 (P2)**: Can start after Foundational (Phase 2) — the `greeter_sync()` function is independent, but wiring it in T010 modifies the same handler as T006, so execute after US1
- **User Story 3 (P3)**: Can start after Foundational (Phase 2) — same handler dependency as US2, execute after US2

### Within Each User Story

- Implementation tasks within a story are sequential (they modify the same files)
- Hook functions (T009, T011) can be written in parallel [P] but wiring depends on the handler existing

### Parallel Opportunities

- T001 and T002 are sequential (T002 depends on T001 for the module to compile)
- T004 and T005 can be written in parallel [P] (different functions, same file but no interdependency)
- T009 and T011 can be written in parallel [P] (independent hook functions)
- T013 and T014 can run in parallel [P] (different linters)

---

## Parallel Example: Foundational Phase

```bash
# After T003 (CLI definition), these can be written in parallel:
Task T004: "Implement scan_directories() in client/src/random.rs"
Task T005: "Implement pick_random() in client/src/random.rs"
```

## Parallel Example: Hook Functions

```bash
# These hook implementations are independent:
Task T009: "Implement greeter_sync() in client/src/random.rs"
Task T011: "Implement write_notify() in client/src/random.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (T001-T002)
2. Complete Phase 2: Foundational (T003-T005)
3. Complete Phase 3: User Story 1 (T006-T008)
4. **STOP and VALIDATE**: Test `wl random ~/pic/wl` — wallpaper applies with transition
5. Deploy/demo if ready — this alone replaces the core nushell script functionality

### Incremental Delivery

1. Complete Setup + Foundational -> Foundation ready
2. Add User Story 1 -> Test independently -> MVP: random wallpaper works
3. Add User Story 2 -> Test independently -> Greeter sync added
4. Add User Story 3 -> Test independently -> Shell notification added
5. Polish -> Lint, format, end-to-end validation
6. Each story adds value without breaking previous stories

---

## Notes

- All changes are client-side only — no daemon modifications
- The `Commands::Random` handler in `main.rs` is the integration point where all stories converge
- Hook functions are non-fatal — errors are warned on stderr but don't prevent wallpaper application
- Transition options are inherited directly from `img` command definitions — avoid duplicating clap attributes if possible (extract shared struct or flatten)
