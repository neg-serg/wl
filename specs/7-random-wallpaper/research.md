# Research: random-wallpaper

**Branch**: `7-random-wallpaper` | **Date**: 2026-03-13

## Decision 1: Recursive Directory Scanning

**Decision**: Use the `walkdir` crate for recursive directory traversal.

**Rationale**: `walkdir` is the de facto Rust solution for recursive directory walking. It handles symlinks, permission errors gracefully, and provides streaming iteration without loading the entire tree into memory. It's lightweight (~200 lines of core logic) and has no transitive dependencies beyond `same-file`.

**Alternatives considered**:
- `std::fs::read_dir` with manual recursion — more code, error-prone symlink handling, no depth control
- `glob` crate — pattern-based, not suited for recursive multi-directory scanning with extension filtering
- `ignore` crate — designed for gitignore-aware traversal, overkill for this use case

## Decision 2: Random Selection

**Decision**: Use the `getrandom` crate (already a dependency in the workspace via daemon) for generating a random index.

**Rationale**: `getrandom` provides OS-level randomness. It's already used in the daemon for random transition selection (`transition.rs`). Using it in the client avoids adding a new RNG dependency. For selecting one item from a list, a simple `random_u64 % count` is sufficient — no need for a full RNG framework like `rand`.

**Alternatives considered**:
- `rand` crate — full-featured but heavy dependency for a single random pick
- `fastrand` — lightweight but adds another dependency when `getrandom` already exists

## Decision 3: Post-Apply Hooks Architecture

**Decision**: Implement greeter sync and shell notification as simple functions called after wallpaper application, controlled by CLI flags.

**Rationale**: The hooks are straightforward file operations (copy file, write text). No plugin system or extensibility framework is needed. CLI flags (`--no-greeter-sync`, `--no-notify`) provide clean opt-out. Custom paths are handled via `--greeter-path` and `--notify-path` options.

**Alternatives considered**:
- Generic hook system with user-defined scripts — violates Principle V (Minimal Complexity) for two known hooks
- Configuration file for hook settings — over-engineering for a CLI tool; flags are simpler and more discoverable

## Decision 4: Default Directories

**Decision**: No hardcoded default directories. The user MUST specify at least one directory as a positional argument or via repeated `--dir` flags.

**Rationale**: Hardcoding `~/pic/wl` and `~/pic/black` would be user-specific and not useful for anyone else. The original script's directories were personal conventions. A CLI tool should require explicit input. The user can alias or wrap the command to set their preferred defaults.

**Alternatives considered**:
- Default to `~/Pictures` (XDG standard) — too presumptuous; many users organize differently
- Default to `~/pic/wl` and `~/pic/black` (from original script) — too specific to one user's setup
- Config file with saved defaults — over-engineering per Principle V

## Decision 5: Daemon Auto-Start

**Decision**: Reuse the existing `daemon::init()` function from `client/src/daemon.rs` which already handles spawning `wl-daemon` and waiting for the socket.

**Rationale**: The `img` command already auto-starts the daemon in `main.rs` (the client checks if daemon is running and spawns it if not). The `random` command will follow the same pattern — no new daemon startup logic needed.

**Alternatives considered**:
- None — reusing existing code is the clear choice

## Decision 6: Image Extension Filtering

**Decision**: Filter by file extension (case-insensitive) matching the formats supported by the `image` crate and listed in the spec: bmp, gif, hdr, ico, jpg, jpeg, png, tif, tiff, webp.

**Rationale**: Extension-based filtering is fast (no file I/O needed beyond directory listing) and matches what the daemon's image decoder supports. The original nushell script used the same approach.

**Alternatives considered**:
- MIME type detection (via `infer` or `tree_magic`) — requires reading file headers, much slower for large directories
- Rely on decoder errors — would require attempting to decode every file, wasteful
