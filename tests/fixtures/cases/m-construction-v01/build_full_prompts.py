from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(3330605)
    created_at = str(date.today())

    targets = {"logos": 5, "people": 14, "site": 20, "diagrams": 12, "social": 8}
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
                "style_tags": ["beaconline", "construction", "mid-sized-gc", "navy-yellow"],
                "source_case_id": "m-construction-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable addresses, license plates, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    # logos
    logo_prompts = [
        "Flat vector logo for a construction company named 'Beaconline Construction Co.' Simple beacon/line icon, navy with safety yellow accent on off-white background. No mockups.",
        "Minimal wordmark logo: 'Beaconline Construction' with a thin yellow underline and small beacon icon. Flat design.",
        "Circular stamp logo: 'Beaconline Construction Co.' around edge, 'Seattle WA' small text, beacon icon. Flat, high contrast.",
        "Monogram logo: 'BCC' in a clean serif inside a square with a thin yellow border. Flat design.",
        "Alternate logo: abstract skyline + beacon line motif, navy/yellow.",
    ]
    for i in range(1, targets["logos"] + 1):
        add("logos", i, logo_prompts[i - 1], neg_flat, 1024, 1024, 30, 5.5)

    # people
    roles = [
        "owner/president",
        "director of preconstruction",
        "senior project manager",
        "superintendent",
        "safety manager",
        "controller",
        "project engineer",
        "field foreman",
    ]
    for i in range(1, targets["people"] + 1):
        role = roles[(i - 1) % len(roles)]
        prompt = (
            f"Photoreal headshot of a {role} at a mid-sized construction company. Neutral background, natural light, professional expression, workwear or business casual. Clearly synthetic."
        )
        add("people", i, prompt, neg_photo, 1024, 1024, 32, 6.0)

    # site
    site_scenes = [
        "commercial tenant improvement jobsite interior, framed walls and MEP rough-in",
        "storefront glass installation scene, construction crew (faces not detailed)",
        "site progress photo of demo debris managed neatly, safety cones",
        "night work interior with work lights, no readable signage",
        "finish stage: flooring install in retail space",
        "paint stage with drop cloths",
        "inspection walkthrough vibe, clipboards, no readable personal data",
        "safety walk scene with PPE, hard hats",
        "material delivery pallets, labels not readable",
        "jobsite trailer interior with schedule board (unreadable text)",
    ]
    for i in range(1, targets["site"] + 1):
        scene = site_scenes[(i - 1) % len(site_scenes)]
        add("site", i, f"Photoreal construction photo: {scene}. Seattle vibe, realistic, no real brands.", neg_photo, 1344, 768, 35, 6.0)

    # diagrams
    diag = [
        "Gantt schedule snippet for a tenant improvement project",
        "change order approval flow diagram (approved vs pending)",
        "RFI log table layout diagram",
        "submittal workflow diagram",
        "WIP margin chart comparing PM vs accounting",
        "long-lead tracker diagram for storefront glass",
        "site safety checklist layout",
        "cost-to-complete waterfall chart",
        "meeting cadence calendar diagram",
        "issue log dashboard layout",
        "baseline vs re-baseline milestone comparison diagram",
        "payment application timeline diagram",
    ]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative construction diagram in navy/yellow palette: {diag[i-1]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 30, 5.5)

    # social
    for i in range(1, targets["social"] + 1):
        if i % 2 == 0:
            prompt = "Illustrative hiring post template for a construction company: off-white background, navy header, safety yellow accents, space for photo, minimal icons."
        else:
            prompt = "Illustrative project update social template: navy background with yellow accent line, space for progress photo, clean typography blocks (not readable)."
        add("social", i, prompt, neg_flat, 1080, 1080, 28, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
