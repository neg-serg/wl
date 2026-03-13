# Feature Specification: Random Wallpaper Command

**Feature Branch**: `7-random-wallpaper`
**Created**: 2026-03-13
**Status**: Draft
**Input**: User description: "Integrate nushell wl script functionality into wl as a built-in subcommand with configurable optional hooks"

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Set a Random Wallpaper (Priority: P1)

A user wants to set a random wallpaper from their image directories with a single command. The system recursively scans configured directories for supported image files, picks one at random, and applies it as the wallpaper with a smooth transition. If the daemon is not running, it is started automatically.

**Why this priority**: This is the core functionality — without it, the other features have no purpose.

**Independent Test**: Can be fully tested by running the command with a directory containing images and verifying that a wallpaper is applied.

**Acceptance Scenarios**:

1. **Given** directories with image files exist, **When** the user runs the random wallpaper command, **Then** a randomly selected image is applied as the wallpaper with a transition.
2. **Given** the daemon is not running, **When** the user runs the random wallpaper command, **Then** the daemon is started automatically before applying the wallpaper.
3. **Given** the user provides custom directories via options, **When** the command runs, **Then** only images from the specified directories are considered.

---

### User Story 2 - Greeter Wallpaper Sync (Priority: P2)

A user wants their login screen greeter to show the same wallpaper they last selected. After applying a wallpaper, the system copies the selected image to a cache location so the greeter can read it on boot. This behavior is optional and can be disabled.

**Why this priority**: Provides a polished visual experience across login and desktop, but is not essential for wallpaper functionality itself.

**Independent Test**: Can be tested by running the command with greeter sync enabled and verifying the cache file is updated, then running with it disabled and verifying no copy occurs.

**Acceptance Scenarios**:

1. **Given** greeter sync is enabled (default), **When** a wallpaper is applied, **Then** the selected image is copied to the greeter cache location.
2. **Given** greeter sync is disabled via option, **When** a wallpaper is applied, **Then** no greeter cache copy occurs.
3. **Given** greeter sync is enabled and a custom cache path is specified, **When** a wallpaper is applied, **Then** the image is copied to the custom path.

---

### User Story 3 - Shell/Desktop Integration Notification (Priority: P3)

A user runs a desktop shell (e.g., quickshell) that reads the current wallpaper path to derive accent colors or other theming. After applying a wallpaper, the system writes the selected image path to a notification file so the desktop shell can react. This behavior is optional and can be disabled.

**Why this priority**: Enables advanced desktop integration, but is niche and not required for basic wallpaper management.

**Independent Test**: Can be tested by running the command with notification enabled and verifying the path file is written, then running with it disabled and verifying no file is written.

**Acceptance Scenarios**:

1. **Given** notification is enabled (default), **When** a wallpaper is applied, **Then** the wallpaper path is written to the notification file.
2. **Given** notification is disabled via option, **When** a wallpaper is applied, **Then** no notification file is written.
3. **Given** a custom notification file path is specified, **When** a wallpaper is applied, **Then** the path is written to the custom location.

---

### Edge Cases

- What happens when no image files are found in the specified directories? The command exits with an error message and non-zero exit code.
- What happens when a specified directory does not exist? The command warns about missing directories and continues scanning remaining ones. If no valid directories remain, it exits with an error.
- What happens when the greeter cache directory does not exist? The command creates intermediate directories as needed, or reports an error if it cannot.
- What happens when the selected image file is deleted between scanning and applying? The command retries with another random pick, up to a reasonable limit.
- What happens when no directories are configured and no defaults exist? The command exits with an error explaining that directories must be specified.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST provide a subcommand that recursively scans one or more directories for image files and applies a randomly selected one as the wallpaper.
- **FR-002**: System MUST support these image formats for scanning: BMP, GIF, HDR, ICO, JPEG (jpg/jpeg), PNG, TIFF (tif/tiff), WebP.
- **FR-003**: System MUST auto-start the daemon if it is not already running when the random wallpaper command is invoked.
- **FR-004**: System MUST allow the user to specify which directories to scan, with a sensible default (e.g., `~/pic/wl` and `~/pic/black`).
- **FR-005**: System MUST apply the wallpaper using a smooth transition (consistent with existing `img` command behavior).
- **FR-006**: System MUST provide an option to copy the selected wallpaper to a configurable cache path (greeter sync), enabled by default.
- **FR-007**: System MUST provide an option to write the selected wallpaper's file path to a configurable notification file (shell integration), enabled by default.
- **FR-008**: System MUST allow the user to disable greeter sync via a command-line option.
- **FR-009**: System MUST allow the user to disable shell notification via a command-line option.
- **FR-010**: System MUST exit with a non-zero exit code and descriptive error when no images are found.
- **FR-011**: All existing `img` command transition options (transition type, FPS, duration, etc.) MUST be passable to the random wallpaper command.

### Key Entities

- **Image Directory**: A user-specified path to recursively scan for wallpaper candidates. Supports multiple directories.
- **Wallpaper Candidate**: A file within the scanned directories whose extension matches the supported image formats list.
- **Greeter Cache**: A file path where the selected wallpaper image is copied for login screen display.
- **Notification File**: A file path where the selected wallpaper's absolute path is written for desktop shell integration.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Users can set a random wallpaper with a single command in under 2 seconds (excluding daemon startup time).
- **SC-002**: The command correctly identifies and considers 100% of supported image formats in scanned directories.
- **SC-003**: Each optional hook (greeter sync, shell notification) can be independently enabled or disabled, verified by the presence or absence of the corresponding file after execution.
- **SC-004**: The command replaces the existing external nushell script entirely — all original behaviors are reproducible via command-line options.

## Assumptions

- Default scan directories are `~/pic/wl` and `~/pic/black` (matching the original script behavior).
- Default greeter cache path is `~/.cache/greeter-wallpaper`.
- Default notification file path is `~/.cache/quickshell-wallpaper-path`.
- The greeter sync copies the actual image file (not a symlink).
- The notification file contains the absolute path of the selected wallpaper as plain text.
- Transition FPS defaults to the project's default (240 FPS).
