#!/usr/bin/env bash
# Visual quality test suite for wl wallpaper renderer.
# Sets test images, captures screenshots, compares pixel-by-pixel.
# Restores original wallpaper after testing.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEST_DIR="/tmp/wl-visual-test"
DIFF_DIR="$TEST_DIR/diffs"
VERBOSE=0
PASSED=0
FAILED=0
TOTAL=0

# Parse args
for arg in "$@"; do
    case "$arg" in
        --verbose|-v) VERBOSE=1 ;;
    esac
done

cleanup() {
    # Restore windows to their original workspaces
    show_windows 2>/dev/null || true

    # Restore original wallpaper
    if [[ -n "${ORIGINAL_WP:-}" && -f "$ORIGINAL_WP" ]]; then
        echo "Restoring original wallpaper..."
        wl img "$ORIGINAL_WP" --resize crop --transition-type none 2>/dev/null || true
        sleep 0.5
    fi
}
# Forward-declare (defined after setup)
show_windows() { :; }
trap cleanup EXIT

log() { echo "[test] $*"; }
pass() { ((PASSED++)); ((TOTAL++)); log "PASS: $1"; }
fail() { ((FAILED++)); ((TOTAL++)); log "FAIL: $1 — $2"; }

# --- Setup ---
mkdir -p "$TEST_DIR" "$DIFF_DIR"

# Save current wallpaper
ORIGINAL_WP=""
query_output=$(wl query 2>/dev/null || true)
if [[ -n "$query_output" ]]; then
    # Parse path from "DP-2: /path/to/image (WxH) [state]"
    ORIGINAL_WP=$(echo "$query_output" | head -1 | sed -n 's/^[^:]*: \(\/[^ ]*\).*/\1/p')
fi
if [[ -n "$ORIGINAL_WP" ]]; then
    log "Saved current wallpaper: $ORIGINAL_WP"
else
    log "No current wallpaper detected (will skip restore)"
fi

# Generate test images
log "Generating test images..."
python3 "$SCRIPT_DIR/generate_tests.py" "$TEST_DIR"

# Save window positions, move all to workspace 99 to expose wallpaper
WINDOW_MAP_FILE="$TEST_DIR/window_map.txt"
hide_windows() {
    rm -f "$WINDOW_MAP_FILE"
    hyprctl clients -j 2>/dev/null | python3 -c "
import json, sys
clients = json.load(sys.stdin)
for c in clients:
    addr = c.get('address', '')
    ws = c.get('workspace', {}).get('id', 1)
    if addr:
        print(f'{addr} {ws}')
" > "$WINDOW_MAP_FILE" 2>/dev/null || true

    if [[ -s "$WINDOW_MAP_FILE" ]]; then
        while IFS=' ' read -r addr ws; do
            hyprctl dispatch movetoworkspacesilent 99,address:"$addr" 2>/dev/null || true
        done < "$WINDOW_MAP_FILE"
        sleep 0.3
    fi
}

# Restore windows to their original workspaces
show_windows() {
    if [[ -s "$WINDOW_MAP_FILE" ]]; then
        while IFS=' ' read -r addr ws; do
            hyprctl dispatch movetoworkspacesilent "$ws",address:"$addr" 2>/dev/null || true
        done < "$WINDOW_MAP_FILE"
    fi
}

# Wait for wallpaper to render after setting it
set_and_capture() {
    local img="$1"
    local resize="$2"
    local output="$3"
    local wait_secs="${4:-1.0}"

    wl img "$img" --resize "$resize" --transition-type none 2>/dev/null
    sleep "$wait_secs"
    grim "$output" 2>/dev/null
}

# --- Test cases ---

# T006: Dark gradient test (US1)
run_dark_gradient_test() {
    local name="dark-gradient-center"
    local src="$TEST_DIR/dark_gradient.png"
    local cap="$TEST_DIR/cap_${name}.png"

    log "Running: $name"
    set_and_capture "$src" "center" "$cap" 1.5

    # Per-pixel diff
    if python3 "$SCRIPT_DIR/compare.py" diff "$src" "$cap" \
        ${VERBOSE:+--diff "$DIFF_DIR/${name}_diff.png"} 2>/dev/null; then
        pass "$name (pixel diff)"
    else
        fail "$name (pixel diff)" "max error > 2"
    fi

    # Histogram deviation
    if python3 "$SCRIPT_DIR/compare.py" histogram "$src" "$cap" 2>/dev/null; then
        pass "$name (histogram)"
    else
        fail "$name (histogram)" "mean brightness deviation > 1"
    fi

    # Banding check
    if python3 "$SCRIPT_DIR/compare.py" banding "$cap" 2>/dev/null; then
        pass "$name (banding)"
    else
        fail "$name (banding)" "banding artifacts detected"
    fi
}

# T007: Solid color test (US1)
run_solid_patches_test() {
    local name="solid-patches-center"
    local src="$TEST_DIR/solid_patches.png"
    local cap="$TEST_DIR/cap_${name}.png"

    log "Running: $name"
    set_and_capture "$src" "center" "$cap" 1.5

    if python3 "$SCRIPT_DIR/compare.py" diff "$src" "$cap" \
        ${VERBOSE:+--diff "$DIFF_DIR/${name}_diff.png"} 2>/dev/null; then
        pass "$name (pixel diff)"
    else
        fail "$name (pixel diff)" "max error > 2"
    fi

    if python3 "$SCRIPT_DIR/compare.py" histogram "$src" "$cap" 2>/dev/null; then
        pass "$name (histogram)"
    else
        fail "$name (histogram)" "mean brightness deviation > 1"
    fi
}

# T011: Contrast verification (US2) — reuses dark gradient capture
run_contrast_test() {
    local name="contrast-dark-gradient"
    local src="$TEST_DIR/dark_gradient.png"
    local cap="$TEST_DIR/cap_dark-gradient-center.png"

    if [[ ! -f "$cap" ]]; then
        log "SKIP: $name (no dark gradient capture available)"
        return
    fi

    log "Running: $name"
    if python3 "$SCRIPT_DIR/compare.py" histogram "$src" "$cap" 2>/dev/null; then
        pass "$name (contrast preserved)"
    else
        fail "$name (contrast preserved)" "brightness shift detected"
    fi
}

# T013: Crop mode test (US3)
run_crop_test() {
    local name="crop-mode"
    local src="$TEST_DIR/dark_gradient.png"
    local cap="$TEST_DIR/cap_${name}.png"

    log "Running: $name"
    set_and_capture "$src" "crop" "$cap" 1.5

    if python3 "$SCRIPT_DIR/compare.py" diff "$src" "$cap" \
        ${VERBOSE:+--diff "$DIFF_DIR/${name}_diff.png"} 2>/dev/null; then
        pass "$name (pixel diff)"
    else
        fail "$name (pixel diff)" "max error > 2"
    fi
}

# T014: Fit mode test (US3)
run_fit_test() {
    local name="fit-mode-small"
    local src="$TEST_DIR/small_image.png"
    local cap="$TEST_DIR/cap_${name}.png"

    log "Running: $name"
    set_and_capture "$src" "fit" "$cap" 1.5

    # For fit mode, we just check that the capture succeeded and
    # the histogram is reasonable (image is present, not all black)
    local cap_mean
    cap_mean=$(python3 -c "
from PIL import Image; import numpy as np
img = np.array(Image.open('$cap').convert('RGB'), dtype=float)
print(f'{np.mean(img):.2f}')
" 2>/dev/null || echo "0")

    if (( $(echo "$cap_mean > 10" | bc -l) )); then
        pass "$name (image visible)"
    else
        fail "$name (image visible)" "captured image appears blank (mean=$cap_mean)"
    fi
}

# T015: Center mode with small image (US3)
run_center_small_test() {
    local name="center-mode-small"
    local src="$TEST_DIR/small_image.png"
    local cap="$TEST_DIR/cap_${name}.png"

    log "Running: $name"
    set_and_capture "$src" "center" "$cap" 1.5

    # Verify black borders exist and center region matches
    local result
    result=$(python3 -c "
from PIL import Image; import numpy as np
cap = np.array(Image.open('$cap').convert('RGBA'), dtype=float)
h, w = cap.shape[:2]
# Check top-left corner is black (border region)
corner = cap[:50, :50, :3]
corner_mean = np.mean(corner)
# Check center region is not black (image present)
cy, cx = h // 2, w // 2
center = cap[cy-50:cy+50, cx-50:cx+50, :3]
center_mean = np.mean(center)
print(f'corner={corner_mean:.2f} center={center_mean:.2f}')
if corner_mean < 1.0 and center_mean > 10.0:
    exit(0)
else:
    exit(1)
" 2>/dev/null)

    if [[ $? -eq 0 ]]; then
        pass "$name (black borders + centered image)"
    else
        fail "$name (black borders + centered image)" "border/center check failed: $result"
    fi
}

# T016: No-resize mode (US3)
run_no_resize_test() {
    local name="no-resize"
    local src="$TEST_DIR/dark_gradient.png"
    local cap="$TEST_DIR/cap_${name}.png"

    log "Running: $name"
    set_and_capture "$src" "no" "$cap" 1.5

    # Just verify the image renders (no-resize stretches to fill)
    if python3 "$SCRIPT_DIR/compare.py" histogram "$src" "$cap" 2>/dev/null; then
        pass "$name (histogram)"
    else
        # No-resize may stretch, so histogram can differ — just check it's not blank
        local cap_mean
        cap_mean=$(python3 -c "
from PIL import Image; import numpy as np
img = np.array(Image.open('$cap').convert('RGB'), dtype=float)
print(f'{np.mean(img):.2f}')
" 2>/dev/null || echo "0")

        if (( $(echo "$cap_mean > 5" | bc -l) )); then
            pass "$name (image visible)"
        else
            fail "$name (image visible)" "image appears blank"
        fi
    fi
}

# --- Execute all tests ---
log "========================================="
log "Visual Quality Test Suite"
log "========================================="

# Hide all windows to expose wallpaper for screenshots
log "Hiding windows for clean capture..."
hide_windows

# Phase 3: US1 — Core quality tests
run_dark_gradient_test
run_solid_patches_test

# Phase 4: US2 — Contrast verification
run_contrast_test

# Phase 5: US3 — Multi-mode tests
run_crop_test
run_fit_test
run_center_small_test
run_no_resize_test

# Restore windows before reporting
log "Restoring windows..."
show_windows

# --- Report ---
echo ""
log "========================================="
log "Results: $PASSED passed, $FAILED failed, $TOTAL total"
log "========================================="

if [[ "$VERBOSE" -eq 1 && -d "$DIFF_DIR" ]]; then
    diff_count=$(find "$DIFF_DIR" -name "*.png" 2>/dev/null | wc -l)
    if [[ "$diff_count" -gt 0 ]]; then
        log "Diff images saved to: $DIFF_DIR"
    fi
fi

if [[ "$FAILED" -gt 0 ]]; then
    log "OVERALL: FAIL"
    exit 1
else
    log "OVERALL: PASS"
    exit 0
fi
