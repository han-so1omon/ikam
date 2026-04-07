from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(55030605)
    created_at = str(date.today())

    targets = {"logos": 10, "people": 30, "facility": 20, "products": 30, "diagrams": 25, "social": 10}
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
                "style_tags": ["titan-river", "enterprise-manufacturing", "steel-blue-orange"],
                "source_case_id": "xl-manufacturing-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable serial numbers, badges, license plates, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    for i in range(1, targets["logos"] + 1):
        add("logos", i, "Flat vector logo for an enterprise manufacturer named 'Titan River Components'. Steel gray with blue and safety orange accents, minimal gear/river icon, no mockups.", neg_flat, 1024, 1024, 30, 5.5)

    roles=["COO","VP supply chain","VP quality","finance director","plant manager","quality engineer","planner","line operator","safety lead"]
    for i in range(1, targets["people"] + 1):
        role=roles[(i-1)%len(roles)]
        add("people", i, f"Photoreal headshot of a {role} at an enterprise manufacturing company. Professional, industrial background blur, clearly synthetic.", neg_photo, 1024, 1024, 32, 6.0)

    scenes=["large factory floor with multiple lines","warehouse aisle with pallets (labels unreadable)","QA lab with measurement equipment","shipping dock with trailers (no readable plates)","plant exterior industrial building"]
    for i in range(1, targets["facility"] + 1):
        add("facility", i, f"Photoreal facility scene: {scenes[(i-1)%len(scenes)]}. Enterprise manufacturing vibe, no real brands.", neg_photo, 1344, 768, 35, 6.0)

    parts=["machined metal component on white background","stamped part close-up on white background","boxed components on pallet","assembly fixture close-up"]
    for i in range(1, targets["products"] + 1):
        add("products", i, f"Photoreal component photo: {parts[(i-1)%len(parts)]}. Clean catalog lighting, no logos.", neg_photo, 1024, 1024, 35, 6.0)

    diag=["OEE dashboard layout plant vs finance","scrap vs rework definition diagram","PPM definition comparison chart","inventory valuation standard vs last purchase","CAPA backlog chart","control plan version rollout timeline","vendor risk register dashboard","expedite tracker dashboard","OTD partial vs complete definition diagram","downtime Pareto chart"]
    for i in range(1, targets["diagrams"] + 1):
        add("diagrams", i, f"Clean illustrative enterprise manufacturing diagram in steel gray/blue/orange palette: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text.", neg_flat, 1344, 768, 30, 5.5)

    for i in range(1, targets["social"] + 1):
        add("social", i, "Illustrative recruiting/customer update template for an enterprise manufacturer: white background, steel gray header, blue accent line, orange highlight, space for photo, minimal icons.", neg_flat, 1080, 1080, 28, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
