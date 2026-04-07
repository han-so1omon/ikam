from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(20260205)
    created_at = str(date.today())

    targets = {
        "logos": 6,
        "people": 18,
        "social": 24,
        "diagrams": 12,
    }

    lines = []

    def add(kind: str, idx: int, *, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind[:-1]}-{idx:03d}.png" if kind.endswith('s') else f"assets/images/{kind}/{kind}-{idx:03d}.png"
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
        elif kind == "logos":
            out_path = f"assets/images/logos/logo-{idx:03d}.png"
        elif kind == "social":
            out_path = f"assets/images/social/social-{idx:03d}.png"
        elif kind == "diagrams":
            out_path = f"assets/images/diagrams/diagram-{idx:03d}.png"

        lines.append(
            {
                "id": f"{kind[:-1]}-{idx:03d}",
                "out_path": out_path,
                "kind": kind[:-1],
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
                "style_tags": ["northlake-advisory", "consulting", "crisp", "professional", "navy-teal"],
                "source_case_id": "m-consulting-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, celebrity, uncanny, distorted face, extra fingers, illegible text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    # LOGOS
    logo_prompts = [
        "Flat vector logo for a consulting firm named 'Northlake Advisory Group'. Minimal wordmark with a simple compass or north star icon. Palette: navy, teal accent, off-white background. No mockups.",
        "Flat icon + wordmark: abstract 'N' mark suggesting a lake horizon line. Clean corporate style, navy + teal.",
        "Stamp-style circular seal logo: 'Northlake Advisory Group' around edge, small north star in center. Flat, high contrast.",
        "Minimal monogram: 'NAG' in a tasteful serif inside a thin circle. Navy on off-white.",
        "Wordmark only: 'Northlake Advisory Group' with a thin teal underline and small dot accent.",
        "Modern classic logo: north arrow icon + wordmark. Flat, minimal, consulting aesthetic.",
    ]
    for i in range(1, targets["logos"] + 1):
        add("logos", i, prompt=logo_prompts[i - 1], negative=neg_flat, w=1024, h=1024, steps=30, guidance=5.5)

    # PEOPLE
    roles = [
        "managing partner",
        "partner",
        "engagement manager",
        "senior consultant",
        "analyst",
        "ops manager",
    ]
    for i in range(1, targets["people"] + 1):
        role = roles[(i - 1) % len(roles)]
        prompt = (
            f"Photoreal headshot of a {role} at a professional consulting firm. Neutral background, soft studio lighting, friendly but serious expression, business casual. "
            "Looks like a staff bio photo, clearly synthetic."
        )
        add("people", i, prompt=prompt, negative=neg_photo, w=1024, h=1024, steps=32, guidance=6.0)

    # SOCIAL (templates)
    social_prompts = []
    for i in range(targets["social"]):
        if i % 3 == 0:
            social_prompts.append(
                "Illustrative LinkedIn post template: off-white background, navy header, teal accent line, space for a chart thumbnail, clean sans typography, minimal icons."
            )
        elif i % 3 == 1:
            social_prompts.append(
                "Illustrative quote-card template: navy background with off-white text blocks (text not readable), teal accent shapes, professional consulting aesthetic, lots of whitespace."
            )
        else:
            social_prompts.append(
                "Mixed-media social template: paper texture off-white, navy blocks, teal highlight, small abstract line icons (compass, chart, checklist)."
            )
    for i in range(1, targets["social"] + 1):
        add("social", i, prompt=social_prompts[i - 1], negative=neg_flat, w=1080, h=1080, steps=28, guidance=5.5)

    # DIAGRAMS (consulting frameworks)
    diagram_types = [
        "2x2 matrix: Impact vs Effort with labeled quadrants",
        "swimlane process diagram for order-to-cash",
        "funnel chart: pipeline stages with conversion rates",
        "dashboard layout: KPI tiles + trend chart + variance table",
        "value stream map style diagram (simplified)",
        "RACI chart layout template",
        "timeline roadmap with milestones",
        "cost breakdown waterfall chart",
        "org chart template",
        "weekly cadence calendar (meetings + artifacts)",
        "risk register table template",
        "data reconciliation flow diagram",
    ]
    for i in range(1, targets["diagrams"] + 1):
        prompt = (
            f"Clean illustrative consulting diagram on off-white background in navy/teal palette: {diagram_types[i-1]}. "
            "Minimal, readable shapes, professional slide-ready style. Avoid real brand marks and avoid tiny unreadable text."
        )
        add("diagrams", i, prompt=prompt, negative=neg_flat, w=1344, h=768, steps=30, guidance=5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    total = sum(targets.values())
    print(f"Wrote {len(lines)} prompts (target {total}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
