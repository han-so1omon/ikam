from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(44020605)
    created_at = str(date.today())

    targets = {"logos": 7, "people": 20, "products": 30, "stores": 8, "social": 15, "diagrams": 12}
    lines = []

    def add(kind: str, idx: int, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind}-{idx:03d}.png"
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
        elif kind == "products":
            out_path = f"assets/images/products/product-{idx:03d}.png"
        elif kind == "stores":
            out_path = f"assets/images/stores/store-{idx:03d}.png"
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
                "style_tags": ["juniper-juno", "boutique", "sage-cream-terracotta"],
                "source_case_id": "l-local-retail-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable personal info, license plates, celebrity, uncanny, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    # logos
    for i in range(1, targets["logos"] + 1):
        prompt = "Flat vector logo for a boutique named 'Juniper & Juno Goods'. Sage green and terracotta accents on cream background, minimal leaf + monogram icon, no mockups."
        add("logos", i, prompt, neg_flat, 1024, 1024, 30, 5.5)

    # people
    roles=["founder/creative director","ops manager","merchandising lead","marketing lead","store manager","retail associate","fulfillment lead"]
    for i in range(1, targets["people"] + 1):
        role = roles[(i-1)%len(roles)]
        prompt = f"Photoreal staff headshot of a {role} at a boutique retail company. Warm natural light, friendly and competent, clearly synthetic."
        add("people", i, prompt, neg_photo, 1024, 1024, 32, 6.0)

    # products
    items=["sage scented candle","ceramic mug","stationery notebook","gift bundle box","linen throw blanket","minimalist vase","hand soap bottle"]
    for i in range(1, targets["products"] + 1):
        item = items[(i-1)%len(items)]
        prompt = f"Photoreal product photo of a {item} in a warm boutique aesthetic, cream background, subtle sage/terracotta props, no real brands."
        add("products", i, prompt, neg_photo, 1024, 1024, 35, 6.0)

    # stores
    scenes=[
        "DTLA boutique store interior with warm lighting and shelves",
        "Silver Lake boutique exterior street-level (no readable address)",
        "Venice store interior with seasonal display table",
        "small fulfillment backroom with shelves and packing table",
    ]
    for i in range(1, targets["stores"] + 1):
        scene = scenes[(i-1)%len(scenes)]
        prompt = f"Photoreal scene: {scene}. Warm boutique vibe, no real brands, no readable personal data."
        add("stores", i, prompt, neg_photo, 1344, 768, 35, 6.0)

    # social
    for i in range(1, targets["social"] + 1):
        prompt = "Illustrative social template for a boutique: cream background, sage header bar, terracotta accent line, space for product photo, minimal icons."
        add("social", i, prompt, neg_flat, 1080, 1080, 28, 5.5)

    # diagrams
    diag=["promo calendar timeline","inventory adjustment flow diagram","gross vs net revenue comparison chart","sell-through dashboard layout","store performance comparison table","replenishment process flow"]
    for i in range(1, targets["diagrams"] + 1):
        prompt = f"Clean illustrative retail ops diagram in sage/cream/terracotta palette: {diag[(i-1)%len(diag)]}. Slide-ready, minimal text."
        add("diagrams", i, prompt, neg_flat, 1344, 768, 30, 5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
