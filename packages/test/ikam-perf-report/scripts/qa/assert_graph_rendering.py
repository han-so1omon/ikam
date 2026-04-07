from __future__ import annotations

from pathlib import Path
import math

try:
    from PIL import Image
except Exception as exc:  # pragma: no cover - defensive import guard
    raise RuntimeError("Pillow is required for graph rendering assertions") from exc


def assert_graph_crop_non_uniform(
    screenshot_path: Path,
    crop_x: int,
    crop_y: int,
    crop_w: int,
    crop_h: int,
    *,
    min_unique_colors: int = 8,
) -> dict[str, float]:
    image = Image.open(screenshot_path)
    width, height = image.size

    left = max(0, min(crop_x, width - 1))
    top = max(0, min(crop_y, height - 1))
    right = max(left + 1, min(left + crop_w, width))
    bottom = max(top + 1, min(top + crop_h, height))

    cropped = image.crop((left, top, right, bottom)).convert("RGB")
    # Sample for speed on large captures while preserving enough variation signal.
    sample = cropped.resize((max(20, cropped.width // 4), max(20, cropped.height // 4)))
    colors = sample.getcolors(maxcolors=1_000_000)
    palette = colors or []
    unique = len(palette)
    if unique < min_unique_colors:
        raise AssertionError(
            {
                "error": "graph_crop_low_variation",
                "unique_colors": unique,
                "min_unique_colors": min_unique_colors,
                "crop": {"x": left, "y": top, "w": right - left, "h": bottom - top},
            }
        )

    total_pixels = max(1, sample.width * sample.height)
    dominant_ratio = (max((count for count, _ in palette), default=0) / total_pixels) if palette else 1.0

    luminances = []
    for count, rgb in palette:
        r, g, b = rgb
        y = 0.2126 * r + 0.7152 * g + 0.0722 * b
        luminances.extend([y] * count)

    if luminances:
        mean_l = sum(luminances) / len(luminances)
        variance = sum((value - mean_l) ** 2 for value in luminances) / len(luminances)
        luminance_stddev = math.sqrt(variance)
    else:
        luminance_stddev = 0.0

    return {
        "unique_colors": float(unique),
        "crop_width": float(right - left),
        "crop_height": float(bottom - top),
        "dominant_color_ratio": float(dominant_ratio),
        "luminance_stddev": float(luminance_stddev),
    }
