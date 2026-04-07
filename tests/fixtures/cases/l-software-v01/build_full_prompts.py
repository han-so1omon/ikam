from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(44060206)
    created_at = str(date.today())

    targets = {"logos": 7, "people": 22, "product-ui": 20, "diagrams": 16, "social": 8}
    lines = []

    def add(kind: str, idx: int, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind}-{idx:03d}.png"
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
        elif kind == "product-ui":
            out_path = f"assets/images/product-ui/ui-{idx:03d}.png"
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
                "style_tags": ["quillstack", "b2b-saas", "navy-violet"],
                "source_case_id": "l-software-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, celebrity, uncanny, extra fingers, distorted text"
    neg_ui = "watermark, real brand logos, personal data, readable emails, phone numbers, credit cards, copyrighted UI"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    for i in range(1, targets["logos"] + 1):
        add("logos", i, "Flat vector logo for a SaaS company named 'QuillStack Systems'. Abstract quill + stack icon, navy with violet accent on off-white background. No mockups.", neg_flat, 1024, 1024, 30, 5.5)

    roles=["CEO","CTO","head of product","engineering manager","security lead","finance lead","customer success manager","engineer","product manager"]
    for i in range(1, targets["people"] + 1):
        role=roles[(i-1)%len(roles)]
        add("people", i, f"Photoreal headshot of a {role} at a modern SaaS company. Neutral background, calm confident expression, clearly synthetic.", neg_photo, 1024, 1024, 32, 6.0)

    ui=["dashboard with queue depth and latency charts","KPI definition registry settings page","incident timeline view","workflow builder canvas","customer account usage page"]
    for i in range(1, targets["product-ui"] + 1):
        add("product-ui", i, f"Synthetic UI screenshot for QuillStack: {ui[(i-1)%len(ui)]}. Modern SaaS, navy/violet palette, placeholder text only, no personal data.", neg_ui, 1344, 768, 34, 6.0)

    diag=["architecture diagram ingestion->queue->workflow","roadmap timeline May vs June","incident causal diagram internal queue saturation","KPI definition mismatch diagram active vs billable","RACI matrix layout"]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative tech diagram in navy/violet palette: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 30, 5.5)

    for i in range(1, targets["social"] + 1):
        add("social", i, "Illustrative social template for a SaaS company: off-white background, navy header, violet accent line, space for UI screenshot, minimal icons.", neg_flat, 1080, 1080, 28, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
