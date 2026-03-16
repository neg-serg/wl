#!/usr/bin/env python3
"""Generate synthetic test images for visual quality testing."""

import json
import subprocess
import sys
from pathlib import Path

try:
    from PIL import Image
    import numpy as np
except ImportError:
    print("ERROR: Pillow and numpy required. Install: pip install Pillow numpy", file=sys.stderr)
    sys.exit(1)


def get_effective_resolution() -> tuple[int, int]:
    """Query effective resolution from hyprctl or fall back to defaults."""
    try:
        result = subprocess.run(
            ["hyprctl", "monitors", "-j"],
            capture_output=True, text=True, timeout=5
        )
        if result.returncode == 0:
            monitors = json.loads(result.stdout)
            if monitors:
                m = monitors[0]
                w = int(m["width"])
                h = int(m["height"])
                return (w, h)
    except (subprocess.TimeoutExpired, FileNotFoundError, json.JSONDecodeError, KeyError):
        pass

    # Fallback
    return (3840, 2160)


def generate_dark_gradient(width: int, height: int, output_path: str):
    """Generate horizontal dark gradient from black (0) to dark gray (60).

    This tests banding/quantization in the most sensitive tonal range.
    """
    arr = np.zeros((height, width, 4), dtype=np.uint8)

    for x in range(width):
        val = int(60.0 * x / (width - 1))
        arr[:, x, 0] = val  # R
        arr[:, x, 1] = val  # G
        arr[:, x, 2] = val  # B
        arr[:, x, 3] = 255  # A

    Image.fromarray(arr, "RGBA").save(output_path)


def generate_solid_patches(width: int, height: int, output_path: str):
    """Generate a grid of solid color patches with known values.

    Tests color accuracy — any shift in RGB values indicates pipeline problems.
    """
    arr = np.zeros((height, width, 4), dtype=np.uint8)
    arr[:, :, 3] = 255  # Full alpha

    # 4x4 grid of test colors
    colors = [
        (0, 0, 0),       (255, 255, 255), (128, 128, 128), (64, 64, 64),
        (255, 0, 0),     (0, 255, 0),     (0, 0, 255),     (255, 255, 0),
        (255, 0, 255),   (0, 255, 255),   (10, 10, 10),    (20, 20, 20),
        (30, 30, 30),    (40, 40, 40),    (50, 50, 50),    (100, 100, 100),
    ]

    rows, cols = 4, 4
    patch_h = height // rows
    patch_w = width // cols

    for i, (r, g, b) in enumerate(colors):
        row = i // cols
        col = i % cols
        y0 = row * patch_h
        x0 = col * patch_w
        arr[y0 : y0 + patch_h, x0 : x0 + patch_w, 0] = r
        arr[y0 : y0 + patch_h, x0 : x0 + patch_w, 1] = g
        arr[y0 : y0 + patch_h, x0 : x0 + patch_w, 2] = b

    Image.fromarray(arr, "RGBA").save(output_path)


def generate_high_contrast(width: int, height: int, output_path: str):
    """Generate alternating black/white vertical stripes (2px wide).

    Tests interpolation blur and pixel alignment.
    """
    arr = np.zeros((height, width, 4), dtype=np.uint8)
    arr[:, :, 3] = 255

    for x in range(width):
        val = 255 if (x // 2) % 2 == 0 else 0
        arr[:, x, 0] = val
        arr[:, x, 1] = val
        arr[:, x, 2] = val

    Image.fromarray(arr, "RGBA").save(output_path)


def generate_small_image(output_path: str):
    """Generate a small 800x600 test image with colored quadrants.

    Used for testing center mode (should appear centered with black borders)
    and fit mode (should be scaled up).
    """
    width, height = 800, 600
    arr = np.zeros((height, width, 4), dtype=np.uint8)
    arr[:, :, 3] = 255

    half_h = height // 2
    half_w = width // 2

    # Red top-left
    arr[:half_h, :half_w, 0] = 200
    # Green top-right
    arr[:half_h, half_w:, 1] = 200
    # Blue bottom-left
    arr[half_h:, :half_w, 2] = 200
    # White bottom-right
    arr[half_h:, half_w:, :3] = 200

    Image.fromarray(arr, "RGBA").save(output_path)


if __name__ == "__main__":
    output_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("/tmp/wl-visual-test")
    output_dir.mkdir(parents=True, exist_ok=True)

    width, height = get_effective_resolution()
    print(f"Generating test images at {width}x{height}")

    generate_dark_gradient(width, height, str(output_dir / "dark_gradient.png"))
    print(f"  dark_gradient.png")

    generate_solid_patches(width, height, str(output_dir / "solid_patches.png"))
    print(f"  solid_patches.png")

    generate_high_contrast(width, height, str(output_dir / "high_contrast.png"))
    print(f"  high_contrast.png")

    generate_small_image(str(output_dir / "small_image.png"))
    print(f"  small_image.png (800x600)")

    print(f"Done. Images in {output_dir}")
