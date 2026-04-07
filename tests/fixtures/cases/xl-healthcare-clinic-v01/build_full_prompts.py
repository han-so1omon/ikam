from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(440405)
    created_at = str(date.today())

    targets = {"logos": 10, "people": 30, "clinics": 15, "diagrams": 20, "social": 10}
    lines = []

    def add(kind: str, idx: int, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind}-{idx:03d}.png"
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
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
                "style_tags": ["hfcn", "healthcare", "clinic-network", "blue-teal"],
                "source_case_id": "xl-healthcare-clinic-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable addresses, readable patient data, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    # logos
    for i in range(1, targets["logos"] + 1):
        prompt = "Flat vector logo for a healthcare clinic network named 'Harborview Family Clinics Network'. Calm clinic blue and teal on white background, simple cross + wave icon, no mockups."
        add("logos", i, prompt, neg_flat, 1024, 1024, 30, 5.5)

    # people
    roles=["chief medical officer","COO","revenue cycle director","compliance officer","HR director","finance lead","clinic site director","nurse","front desk staff"]
    for i in range(1, targets["people"] + 1):
        role = roles[(i-1)%len(roles)]
        prompt = f"Photoreal staff headshot of a {role} at a healthcare clinic network. Calm, professional, neutral background, clearly synthetic."
        add("people", i, prompt, neg_photo, 1024, 1024, 32, 6.0)

    # clinics
    scenes=[
        "clinic exterior in suburban Phoenix, clean signage with fictional name, no readable address",
        "clinic reception area with calm colors and seating",
        "exam room interior with basic equipment, no patient present",
        "billing office desks and computers with blurred screens",
        "training room with projector (no readable text)",
    ]
    for i in range(1, targets["clinics"] + 1):
        scene=scenes[(i-1)%len(scenes)]
        prompt=f"Photoreal scene: {scene}. Calm healthcare vibe, no real brands, no PHI."
        add("clinics", i, prompt, neg_photo, 1344, 768, 35, 6.0)

    # diagrams
    diag=[
        "billing process map diagram",
        "denials management workflow diagram",
        "no-show definition comparison chart",
        "collections definition comparison chart",
        "HIPAA training completion dashboard layout",
        "site director scorecard dashboard layout",
        "privacy policy rollout timeline",
        "incident timeline diagram",
    ]
    for i in range(1, targets["diagrams"] + 1):
        prompt=f"Clean illustrative healthcare ops diagram in blue/teal palette: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text."
        add("diagrams", i, prompt, neg_flat, 1344, 768, 30, 5.5)

    # social
    for i in range(1, targets["social"] + 1):
        if i % 2 == 0:
            prompt="Illustrative hiring post template for a clinic network: white background, clinic-blue header, teal accents, space for photo, minimal icons."
        else:
            prompt="Illustrative patient communication template: appointment reminders and privacy assurance theme, blue/teal palette, clean layout, no real personal data."
        add("social", i, prompt, neg_flat, 1080, 1080, 28, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
