from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(1060605)
    created_at = str(date.today())

    targets = {"logos": 4, "people": 10, "site": 12, "diagrams": 8, "social": 6}
    lines = []

    def add(kind: str, idx: int, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind}-{idx:03d}.png"
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
        elif kind == "logos":
            out_path = f"assets/images/logos/logo-{idx:03d}.png"
        lines.append(
            {
                "id": f"{kind}-{idx:03d}",
                "out_path": out_path,
                "kind": kind,
                "prompt": prompt,
                "negative_prompt": negative,
                "model": "sdxl",
                "checkpoint": "SSD-1B.safetensors",
                "sampler": "dpmpp_2m",
                "steps": steps,
                "guidance": guidance,
                "width": w,
                "height": h,
                "seed": rng.randrange(10_000, 9_999_999),
                "created_at": created_at,
                "style_tags": ["alder-ridge-builders", "construction", "local-gc", "green-gray"],
                "source_case_id": "s-construction-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable addresses, license plates, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    # logos
    logo_prompts = [
        "Flat vector logo for a local construction company named 'Alder Ridge Builders'. Simple house/roof line icon with an alder leaf accent. Palette: forest green, warm gray, off-white background. No mockups.",
        "Minimal wordmark logo: 'Alder Ridge Builders' with a thin green underline and small leaf icon. Flat design.",
        "Circular stamp logo: 'Alder Ridge Builders' around edge, 'Portland OR' small text, simple house icon. Flat, high contrast.",
        "Monogram logo: 'ARB' in a clean serif inside a square with a leaf corner accent. Flat design.",
    ]
    for i in range(1, targets["logos"] + 1):
        add("logos", i, logo_prompts[i - 1], neg_flat, 1024, 1024, 30, 5.5)

    # people
    roles = ["owner / project manager", "project coordinator", "site supervisor", "lead carpenter", "estimator"]
    for i in range(1, targets["people"] + 1):
        role = roles[(i - 1) % len(roles)]
        prompt = (
            f"Photoreal headshot of a {role} at a small construction company. Neutral background, natural light, friendly professional expression, workwear or business casual. Clearly synthetic."
        )
        add("people", i, prompt, neg_photo, 1024, 1024, 32, 6.0)

    # site photos
    site_scenes = [
        "jobsite exterior progress photo of a small backyard ADU under construction, framing visible",
        "interior framing progress photo, clean site, tools neatly staged",
        "foundation/pour day progress photo, overcast PNW weather",
        "drywall stage interior photo, taped seams, unfinished",
        "finish carpentry stage, cabinets being installed",
        "bathroom tile work progress photo",
        "window installation progress photo (no readable address)",
        "site safety walk photo: PPE and cones, no readable signage",
        "material delivery photo with pallets, no readable labels",
        "final walkthrough vibe photo, clean interior",
        "small office TI progress photo, commercial interior",
        "before/after split-style photo layout (synthetic) for a remodel",
    ]
    for i in range(1, targets["site"] + 1):
        add("site", i, site_scenes[i - 1] + ". Photoreal, natural light, no real brands.", neg_photo, 1344, 768, 35, 6.0)

    # diagrams
    diag = [
        "simple Gantt schedule snippet diagram for an ADU project",
        "change order approval workflow diagram",
        "RFI log table layout",
        "job costing budget vs actual bar chart",
        "payment schedule milestone chart",
        "subcontractor scope map diagram",
        "safety checklist layout",
        "permit inspection timeline diagram",
    ]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative construction diagram in green/gray palette: {diag[i-1]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 30, 5.5)

    # social
    for i in range(1, targets["social"] + 1):
        prompt = (
            "Illustrative social post template for a local GC: off-white background, forest green header bar, warm gray accents, space for project photo, minimal icons."
        )
        add("social", i, prompt, neg_flat, 1080, 1080, 28, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
