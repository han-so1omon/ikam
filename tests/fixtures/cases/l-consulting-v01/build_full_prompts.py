from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(44060205)
    created_at = str(date.today())

    targets = {"logos": 6, "people": 22, "social": 18, "diagrams": 18}
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
                "style_tags": ["greybridge", "consulting", "premium", "charcoal-gold"],
                "source_case_id": "l-consulting-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, celebrity, uncanny, distorted face, extra fingers, illegible text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    # logos
    logo_prompts = [
        "Flat vector logo for a consulting firm named 'Greybridge Partners'. Minimal bridge icon, charcoal and muted gold on off-white background. No mockups.",
        "Wordmark logo: 'Greybridge Partners' in classic serif with a thin gold underline. Flat design.",
        "Circular seal logo: 'Greybridge Partners' around edge, 'Chicago IL' small text, bridge icon. Flat, high contrast.",
        "Monogram logo: 'GP' in a classic serif inside a thin circle, charcoal on off-white.",
        "Abstract logo: two parallel lines forming a bridge span + wordmark; minimal, premium.",
        "Alternate mark: small arch bridge pictogram + wordmark; charcoal/gold.",
    ]
    for i in range(1, targets["logos"] + 1):
        add("logos", i, logo_prompts[i - 1], neg_flat, 1024, 1024, 30, 5.5)

    # people
    roles = ["managing partner", "partner", "practice lead", "director", "finance director", "talent lead", "consultant", "analyst"]
    for i in range(1, targets["people"] + 1):
        role = roles[(i - 1) % len(roles)]
        prompt = (
            f"Photoreal headshot of a {role} at a premium consulting firm. Neutral background, soft studio lighting, calm confident expression, business attire. Clearly synthetic."
        )
        add("people", i, prompt, neg_photo, 1024, 1024, 32, 6.0)

    # social
    for i in range(1, targets["social"] + 1):
        if i % 3 == 1:
            prompt = "Illustrative LinkedIn post template: off-white background, charcoal header bar, muted gold accent line, space for a chart thumbnail, clean typography blocks (not readable)."
        elif i % 3 == 2:
            prompt = "Illustrative quote card template: charcoal background with off-white text blocks (not readable), muted gold accents, premium consulting aesthetic."
        else:
            prompt = "Mixed-media social template: subtle paper texture off-white, charcoal blocks, muted gold highlights, minimal line icons (bridge, chart, checklist)."
        add("social", i, prompt, neg_flat, 1080, 1080, 28, 5.5)

    # diagrams
    diag = [
        "2x2 matrix: urgency vs impact",
        "turnaround operating cadence calendar",
        "benefits tracker dashboard layout",
        "pipeline funnel chart",
        "utilization definition comparison diagram",
        "workplan timeline roadmap",
        "cash stabilization flowchart",
        "stakeholder RACI chart layout",
        "risk register table layout",
        "issue log dashboard layout",
        "value bridge waterfall chart",
        "savings initiative prioritization chart",
        "meeting structure swimlane diagram",
        "success fee recognition timeline diagram",
        "artifact map: SOW -> deliverables -> closeout",
        "practice overlap Venn diagram",
        "template version control process diagram",
        "client naming alias rules diagram",
    ]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative consulting diagram in charcoal/gold palette: {diag[i-1]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 30, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
