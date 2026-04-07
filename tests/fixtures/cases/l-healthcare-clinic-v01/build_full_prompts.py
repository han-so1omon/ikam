from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(110206)
    created_at = str(date.today())

    targets = {"logos": 8, "people": 22, "clinic": 22, "diagrams": 16, "social": 8}
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
                "style_tags": ["hcp", "clinic-network", "sea-blue-slate"],
                "source_case_id": "l-healthcare-clinic-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable patient info, readable signage, addresses, license plates, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    for i in range(1, targets["logos"] + 1):
        add("logos", i, "Flat vector logo for a clinic network named 'Harborview Care Partners'. Sea-blue and slate palette, simple wave/harbor icon, friendly and professional, no mockups.", neg_flat, 1024, 1024, 30, 5.5)

    roles=["medical director","COO","nurse manager","front desk lead","medical assistant","revenue cycle manager","compliance officer","provider"]
    for i in range(1, targets["people"] + 1):
        role=roles[(i-1)%len(roles)]
        add("people", i, f"Photoreal headshot of a {role} working at a modern outpatient clinic. Warm lighting, professional, clearly synthetic.", neg_photo, 1024, 1024, 32, 6.0)

    scenes=["clinic exterior with glass entrance (no readable signage)","reception/check-in desk with privacy screens","patient exam room clean modern","nursing station corridor","urgent care room setup","waiting room with chairs and posters (text not readable)"]
    for i in range(1, targets["clinic"] + 1):
        add("clinic", i, f"Photoreal clinic scene: {scenes[(i-1)%len(scenes)]}. Sea-blue accents, no real brands.", neg_photo, 1344, 768, 35, 6.0)

    diag=["no-show definition comparison late-cancel included vs excluded","HIPAA training compliance dashboard 98% vs 91% roster mismatch","wait time definition door-to-doc vs arrival-to-departure","patient flow diagram check-in to checkout","denials trend chart and top reasons","triage SOP v2 vs v3 rollout timeline"]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative healthcare operations diagram in sea-blue/slate palette: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 30, 5.5)

    for i in range(1, targets["social"] + 1):
        add("social", i, "Illustrative patient outreach/recruiting social template for a clinic network: white background, sea-blue header, slate text blocks, space for photo, minimal icons.", neg_flat, 1080, 1080, 28, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
