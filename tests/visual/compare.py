#!/usr/bin/env python3
"""Image comparison utilities for visual quality testing."""

import sys
from pathlib import Path

try:
    from PIL import Image
    import numpy as np
except ImportError:
    print("ERROR: Pillow and numpy required. Install: pip install Pillow numpy", file=sys.stderr)
    sys.exit(1)


def load_image(path: str) -> np.ndarray:
    """Load image as RGBA numpy array."""
    img = Image.open(path).convert("RGBA")
    return np.array(img, dtype=np.float64)


def per_pixel_diff(source: np.ndarray, captured: np.ndarray) -> dict:
    """Compute per-pixel difference metrics between two images.

    Both images must have the same dimensions. Compares RGB channels only.
    Returns dict with max_error, mean_error per channel and overall.
    """
    # Use only RGB (ignore alpha)
    src_rgb = source[:, :, :3]
    cap_rgb = captured[:, :, :3]

    if src_rgb.shape != cap_rgb.shape:
        return {
            "error": f"dimension mismatch: source {src_rgb.shape} vs captured {cap_rgb.shape}",
            "passed": False,
        }

    diff = np.abs(src_rgb - cap_rgb)

    return {
        "max_error": float(np.max(diff)),
        "mean_error": float(np.mean(diff)),
        "max_error_per_channel": [float(np.max(diff[:, :, c])) for c in range(3)],
        "mean_error_per_channel": [float(np.mean(diff[:, :, c])) for c in range(3)],
        "passed": True,
    }


def histogram_deviation(source: np.ndarray, captured: np.ndarray) -> dict:
    """Compare per-channel mean brightness between source and captured.

    Detects contrast loss or color shifts.
    """
    src_rgb = source[:, :, :3]
    cap_rgb = captured[:, :, :3]

    if src_rgb.shape != cap_rgb.shape:
        return {
            "error": f"dimension mismatch: source {src_rgb.shape} vs captured {cap_rgb.shape}",
            "passed": False,
        }

    src_means = [float(np.mean(src_rgb[:, :, c])) for c in range(3)]
    cap_means = [float(np.mean(cap_rgb[:, :, c])) for c in range(3)]
    deviations = [abs(s - c) for s, c in zip(src_means, cap_means)]

    return {
        "source_means": src_means,
        "captured_means": cap_means,
        "deviations": deviations,
        "max_deviation": max(deviations),
        "passed": True,
    }


def check_banding(captured: np.ndarray, region: tuple = None) -> dict:
    """Check for banding artifacts in gradient regions.

    Examines adjacent rows for sudden brightness jumps (> threshold).
    region: (x, y, w, h) or None for full image.
    """
    img = captured[:, :, :3]
    if region:
        x, y, w, h = region
        img = img[y : y + h, x : x + w, :]

    # Compute per-row mean brightness
    row_means = np.mean(img, axis=(1, 2))

    # Check adjacent row differences
    row_diffs = np.abs(np.diff(row_means))
    max_jump = float(np.max(row_diffs)) if len(row_diffs) > 0 else 0.0
    mean_jump = float(np.mean(row_diffs)) if len(row_diffs) > 0 else 0.0

    # Banding threshold: adjacent rows in a smooth gradient should differ by <= 2
    banding_threshold = 2.0
    banding_count = int(np.sum(row_diffs > banding_threshold))

    return {
        "max_row_jump": max_jump,
        "mean_row_jump": mean_jump,
        "banding_count": banding_count,
        "banding_threshold": banding_threshold,
        "passed": banding_count == 0,
    }


def compare_region(source: np.ndarray, captured: np.ndarray,
                   src_region: tuple, cap_region: tuple) -> dict:
    """Compare a specific region of source with a region of captured image.

    Useful for verifying center mode, crop mode, etc.
    Regions: (x, y, w, h)
    """
    sx, sy, sw, sh = src_region
    cx, cy, cw, ch = cap_region

    src_crop = source[sy : sy + sh, sx : sx + sw, :3]
    cap_crop = captured[cy : cy + ch, cx : cx + cw, :3]

    if src_crop.shape != cap_crop.shape:
        return {
            "error": f"region mismatch: source {src_crop.shape} vs captured {cap_crop.shape}",
            "passed": False,
        }

    diff = np.abs(src_crop - cap_crop)
    return {
        "max_error": float(np.max(diff)),
        "mean_error": float(np.mean(diff)),
        "passed": True,
    }


def save_diff_image(source: np.ndarray, captured: np.ndarray, output_path: str, scale: int = 10):
    """Save a diff visualization image (differences amplified by scale)."""
    src_rgb = source[:, :, :3]
    cap_rgb = captured[:, :, :3]

    if src_rgb.shape != cap_rgb.shape:
        return

    diff = np.abs(src_rgb - cap_rgb) * scale
    diff = np.clip(diff, 0, 255).astype(np.uint8)

    # Add alpha channel
    alpha = np.full((*diff.shape[:2], 1), 255, dtype=np.uint8)
    diff_rgba = np.concatenate([diff, alpha], axis=2)

    Image.fromarray(diff_rgba, "RGBA").save(output_path)


if __name__ == "__main__":
    if len(sys.argv) < 4:
        print(f"Usage: {sys.argv[0]} <command> <source.png> <captured.png> [--diff output.png]")
        print("Commands: diff, histogram, banding")
        sys.exit(1)

    cmd = sys.argv[1]
    source_path = sys.argv[2]
    captured_path = sys.argv[3]

    source = load_image(source_path)
    captured = load_image(captured_path)

    if cmd == "diff":
        result = per_pixel_diff(source, captured)
        print(f"max_error={result['max_error']:.1f}")
        print(f"mean_error={result['mean_error']:.4f}")
        for i, ch in enumerate(["R", "G", "B"]):
            print(f"max_{ch}={result['max_error_per_channel'][i]:.1f} mean_{ch}={result['mean_error_per_channel'][i]:.4f}")

        # Save diff image if requested
        if "--diff" in sys.argv:
            idx = sys.argv.index("--diff")
            if idx + 1 < len(sys.argv):
                save_diff_image(source, captured, sys.argv[idx + 1])
                print(f"diff_image={sys.argv[idx + 1]}")

        sys.exit(0 if result["max_error"] <= 2.0 else 1)

    elif cmd == "histogram":
        result = histogram_deviation(source, captured)
        print(f"max_deviation={result['max_deviation']:.4f}")
        for i, ch in enumerate(["R", "G", "B"]):
            print(f"{ch}: source={result['source_means'][i]:.2f} captured={result['captured_means'][i]:.2f} dev={result['deviations'][i]:.4f}")
        sys.exit(0 if result["max_deviation"] <= 1.0 else 1)

    elif cmd == "banding":
        result = check_banding(captured)
        print(f"max_row_jump={result['max_row_jump']:.4f}")
        print(f"mean_row_jump={result['mean_row_jump']:.4f}")
        print(f"banding_count={result['banding_count']}")
        sys.exit(0 if result["passed"] else 1)

    else:
        print(f"Unknown command: {cmd}", file=sys.stderr)
        sys.exit(1)
