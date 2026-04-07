from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(220506)
    created_at = str(date.today())

    targets = {"logos": 4, "people": 8, "shop": 8, "products": 12, "diagrams": 8}
    lines = []

    def add(kind: str, idx: int, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind}-{idx:03d}.png"
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
        elif kind == "products":
            out_path = f"assets/images/products/product-{idx:03d}.png"
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
                "style_tags": ["ridgeway", "job-shop", "steel-orange"],
                "source_case_id": "s-manufacturing-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable serial numbers, license plates, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    for i in range(1, targets["logos"] + 1):
        add("logos", i, "Flat vector logo for a metal fabrication shop named 'Ridgeway Metal Works'. Steel gray and orange accents on white background, simple bracket icon, no mockups.", neg_flat, 1024, 1024, 30, 5.5)

    roles=["owner","office/ops","lead fabricator","welder","machine operator"]
    for i in range(1, targets["people"] + 1):
        role=roles[(i-1)%len(roles)]
        add("people", i, f"Photoreal headshot of a {role} at a small fabrication shop. Industrial background blur, friendly and practical, clearly synthetic.", neg_photo, 1024, 1024, 32, 6.0)

    scenes=["small fabrication shop interior with welders and metal racks","laser cutter area with safety guarding","shipping corner with pallets (no readable labels)","workbench with tools and parts"]
    for i in range(1, targets["shop"] + 1):
        add("shop", i, f"Photoreal scene: {scenes[(i-1)%len(scenes)]}. Industrial job shop vibe, no real brands.", neg_photo, 1344, 768, 35, 6.0)

    parts=["laser cut steel bracket on white background","stack of small plates with holes","welded frame corner","bag of fasteners in tray"]
    for i in range(1, targets["products"] + 1):
        add("products", i, f"Photoreal product/part photo: {parts[(i-1)%len(parts)]}. Clean catalog lighting, no real logos.", neg_photo, 1024, 1024, 35, 6.0)

    diag=["job board kanban layout","production schedule table layout","simple job costing chart","quote breakdown diagram","vendor lead time chart","incident log table layout","inventory count sheet layout","material price change note diagram"]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative manufacturing diagram in steel gray/orange palette: {diag[i-1]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 30, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
