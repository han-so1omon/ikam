from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"


def main():
    rng = random.Random(260205)
    created_at = str(date.today())

    targets = {
        "logos": 5,
        "people": 15,
        "facility": 10,
        "products": 25,
        "diagrams": 15,
        "social": 10,
    }

    lines = []

    def add(kind: str, idx: int, *, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
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
                "style_tags": ["cff", "manufacturing", "industrial", "steel-orange", "pNW"],
                "source_case_id": "l-manufacturing-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable serial numbers, readable badges, license plates, celebrity, uncanny, extra fingers, distorted text"
    neg_flat = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo"

    # LOGOS
    logo_prompts = [
        "Flat vector logo for a manufacturing company named 'Cascadia Fasteners & Forming'. Simple icon of a bolt head or stamped metal part, navy/steel gray with safety orange accent on white background. No mockup.",
        "Minimal wordmark logo: 'Cascadia Fasteners & Forming' with a small bolt icon and orange underline. Flat design.",
        "Seal-style logo: circular stamp with 'Cascadia Fasteners & Forming' and 'Tacoma, WA' around edge, bolt icon in center. Flat, high contrast.",
        "Monogram logo: 'CFF' in an industrial sans-serif inside a rectangle; steel gray and orange. Flat design.",
        "Alternate icon: abstract formed bracket silhouette + wordmark; steel gray + orange accent.",
    ]
    for i in range(1, targets["logos"] + 1):
        add("logos", i, prompt=logo_prompts[i - 1], negative=neg_flat, w=1024, h=1024, steps=30, guidance=5.5)

    # PEOPLE
    roles = ["gm", "plant manager", "qa manager", "procurement lead", "controller", "sales director", "machine operator", "quality tech"]
    for i in range(1, targets["people"] + 1):
        role = roles[(i - 1) % len(roles)]
        prompt = (
            f"Photoreal headshot of a {role} at a manufacturing company. Neutral background or factory blur, safety-conscious vibe, business casual or workwear. "
            "Looks like a staff bio photo, clearly synthetic."
        )
        add("people", i, prompt=prompt, negative=neg_photo, w=1024, h=1024, steps=32, guidance=6.0)

    # FACILITY
    facility_scenes = [
        "photoreal interior of a small industrial manufacturing plant with press lines, clean but worn, safety orange accents",
        "warehouse aisle with pallets and labeled bins (labels not readable), industrial lighting",
        "press line close-up with guarding and safety signage (text not readable)",
        "shipping/receiving dock with shrink-wrapped pallets, no readable plates",
        "quality inspection bench with calipers and gauge blocks",
        "break room bulletin board (no readable personal info)",
        "exterior of an industrial building in Tacoma, overcast PNW day, no real logos",
        "coil steel storage area, organized racks",
        "packaging/kitting station with boxes and labels (unreadable)",
        "3PL warehouse vibe shot, wide angle",
    ]
    for i in range(1, targets["facility"] + 1):
        prompt = facility_scenes[i - 1] + ". Natural colors, realistic, no real brands."
        add("facility", i, prompt=prompt, negative=neg_photo, w=1344, h=768, steps=35, guidance=6.0)

    # PRODUCTS
    product_items = [
        "M8 bolt 30mm zinc plated on white background",
        "small L-bracket steel part on white background",
        "HVAC mounting clip close-up on white background",
        "assorted fasteners in a small tray, clean catalog photo",
        "formed metal bracket with caliper measurement next to it",
    ]
    for i in range(1, targets["products"] + 1):
        item = product_items[(i - 1) % len(product_items)]
        prompt = (
            f"Photoreal catalog-style product photo: {item}. Sharp focus, clean lighting, subtle shadow, industrial feel, no real logos."
        )
        add("products", i, prompt=prompt, negative=neg_photo, w=1024, h=1024, steps=35, guidance=6.0)

    # DIAGRAMS
    diagram_types = [
        "process flow diagram: order-to-ship with QA gates",
        "OEE dashboard layout with KPI tiles and trend chart",
        "SPC control chart template (clean, minimal)",
        "fishbone diagram template for scrap root cause",
        "value stream map simplified for press line",
        "inventory cycle count checklist diagram",
        "OTD definition comparison diagram (customer vs internal)",
        "nonconformance workflow swimlane",
        "capex request decision flow",
        "supplier risk matrix (2x2)",
        "production schedule Gantt layout",
        "warehouse bin location map schematic",
        "quality incident timeline layout",
        "rework vs scrap definition diagram",
        "SOP version adoption tracker chart",
    ]
    for i in range(1, targets["diagrams"] + 1):
        prompt = (
            f"Clean illustrative manufacturing diagram on white/steel palette with safety orange accents: {diagram_types[i-1]}. "
            "Slide-ready, minimal text, high contrast."
        )
        add("diagrams", i, prompt=prompt, negative=neg_flat, w=1344, h=768, steps=30, guidance=5.5)

    # SOCIAL
    for i in range(1, targets["social"] + 1):
        if i % 2 == 0:
            prompt = "Illustrative recruiting post template for a manufacturing company: steel gray background, orange accent shapes, space for photo, clean sans typography blocks (not readable text)."
        else:
            prompt = "Illustrative customer update template: on-time delivery and quality improvements theme, steel/white background with orange accent line, space for a simple chart thumbnail."
        add("social", i, prompt=prompt, negative=neg_flat, w=1080, h=1080, steps=28, guidance=5.5)

    PROMPTS_PATH.write_text("\n".join(json.dumps(x, ensure_ascii=False) for x in lines) + "\n", encoding="utf-8")
    print(f"Wrote {len(lines)} prompts (target {sum(targets.values())}) -> {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
