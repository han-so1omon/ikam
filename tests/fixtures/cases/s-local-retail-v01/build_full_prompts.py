from __future__ import annotations

import json
import random
from datetime import date
from pathlib import Path

CASE_DIR = Path(__file__).resolve().parent
PROMPTS_PATH = CASE_DIR / "assets/images/prompts.jsonl"

PALETTE = {
    "bramble_green": "#1F3D2B",
    "cream": "#F4EFE6",
    "copper": "#B87333",
    "ink": "#1A1A1A",
    "fog": "#D7D2C8",
}


def j(obj):
    return json.dumps(obj, ensure_ascii=False)


def main():
    rng = random.Random(1337)
    created_at = str(date.today())

    # Exact targets (pick specific counts within the spec ranges)
    targets = {
        "products": 40,
        "storefront": 8,
        "people": 15,
        "social": 30,
        "logos": 7,
    }

    lines = []

    def add(kind: str, idx: int, *, prompt: str, negative: str, w: int, h: int, steps: int, guidance: float):
        out_path = f"assets/images/{kind}/{kind[:-1]}-{idx:03d}.png" if kind.endswith("s") else f"assets/images/{kind}/{kind}-{idx:03d}.png"
        # normalize singular folder names
        if kind == "people":
            out_path = f"assets/images/people/person-{idx:03d}.png"
        elif kind == "products":
            out_path = f"assets/images/products/product-{idx:03d}.png"
        elif kind == "storefront":
            out_path = f"assets/images/storefront/storefront-{idx:03d}.png"
        elif kind == "social":
            out_path = f"assets/images/social/social-{idx:03d}.png"
        elif kind == "logos":
            out_path = f"assets/images/logos/logo-{idx:03d}.png"

        lines.append(
            {
                "id": f"{kind[:-1] if kind.endswith('s') else kind}-{idx:03d}",
                "out_path": out_path,
                "kind": kind[:-1] if kind.endswith("s") else kind,
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
                "style_tags": [
                    "bramble&bitters",
                    "oakland",
                    "cozy",
                    "curated",
                    f"palette:{PALETTE['bramble_green']},{PALETTE['cream']},{PALETTE['copper']}",
                ],
                "source_case_id": "s-local-retail-v01",
            }
        )

    neg_photo = "watermark, blurry, low-res, real brand logos, readable license plates, celebrity, uncanny, extra fingers, distorted text"
    neg_logo = "photorealistic, 3D, bevel, embossed, mockup, watermark, stock logo, copyrighted brand names"
    neg_social = "photoreal, cluttered, watermark, illegible text, real brand marks"

    # LOGOS (flat/illustrative)
    logo_prompts = [
        "Flat vector-style wordmark logo: 'Bramble & Bitters' classic serif wordmark with a small ampersand. Minimal leaf sprig accent. Cream background, bramble green text, copper accent. No mockups.",
        "Flat icon + wordmark: a simple bramble leaf silhouette forming an ampersand, paired with 'Bramble & Bitters'. Modern-classic look, two-color (bramble green + copper) on cream.",
        "Stamp-style circular logo: 'Bramble & Bitters' and 'Oakland, CA' around the edge; minimal leaf in center. Flat design, bramble green ink on cream.",
        "Monogram logo: 'B' and '&' intertwined in a classic serif style, with small 'Bramble & Bitters' below. Flat, minimal, bramble green + copper accents.",
        "Simple shelf-label logo: small serif wordmark 'Bramble & Bitters' with a thin copper underline; very minimal.",
        "Illustrative logo: a small sprig of bramble berries drawn in clean line art, paired with 'Bramble & Bitters'. Flat, high contrast.",
        "Alternate logo lockup: stacked words 'Bramble' over '& Bitters' in a classic serif, centered; copper dot accents.",
    ]
    for i in range(1, targets["logos"] + 1):
        add(
            "logos",
            i,
            prompt=logo_prompts[i - 1],
            negative=neg_logo,
            w=1024,
            h=1024,
            steps=30,
            guidance=5.5,
        )

    # PRODUCTS (photoreal-ish)
    product_items = [
        "apricot-almond jam jar",
        "smoked chili hot sauce bottle",
        "sea salt dark chocolate bar",
        "jasmine pearl tea tin",
        "mini gift bundle box tied with twine",
        "classic gift bundle box with tissue paper",
        "deluxe gift bundle with copper ribbon",
        "small-batch olive oil bottle",
        "artisan crackers box",
        "seasonal honey jar",
        "spiced nuts pouch",
        "gourmet mustard jar",
        "handmade soap bar",
        "candied citrus peel jar",
        "small coffee sampler bag",
    ]

    for i in range(1, targets["products"] + 1):
        item = product_items[(i - 1) % len(product_items)]
        style = [
            "warm natural light",
            "soft shadow on cream background",
            "catalog photo",
            "50mm look",
            "high detail",
        ]
        # Mix in a few lifestyle shots
        if i % 10 == 0:
            scene = "on a small wooden counter with a hint of shop shelves blurred in the background"
        else:
            scene = "on a cream seamless background"
        prompt = (
            f"Photoreal product photo of a {item} {scene}. "
            "Indie boutique brand aesthetic, clean composition, subtle grain, realistic label reading 'Bramble & Bitters'. "
            f"{', '.join(style)}."
        )
        add("products", i, prompt=prompt, negative=neg_photo, w=1024, h=1024, steps=35, guidance=6.0)

    # STOREFRONT (photoreal-ish)
    storefront_scenes = [
        "street-level exterior storefront with bramble-green sign 'Bramble & Bitters', golden hour",
        "interior shelves with curated pantry items, warm lighting, cozy boutique",
        "wrapping station with tissue paper and copper ribbon, hands wrapping a gift",
        "window display with seasonal gift bundles, subtle plants",
        "countertop checkout area, small signage, minimal clutter",
        "small tasting table with samples, warm inviting vibe",
        "evening exterior with lights on, rainy street reflections",
        "interior wide shot, cream walls, bramble green accents, plants",
    ]
    for i in range(1, targets["storefront"] + 1):
        prompt = (
            f"Photoreal scene: {storefront_scenes[i-1]}. Oakland neighborhood feel, no real brands, no readable license plates. "
            "Cinematic but realistic, natural colors, medium depth of field."
        )
        add("storefront", i, prompt=prompt, negative=neg_photo, w=1344, h=768, steps=35, guidance=6.0)

    # PEOPLE (synthetic staff headshots)
    roles = [
        "small business owner",
        "shift lead",
        "marketing contractor",
        "seasonal associate",
        "bookkeeper (office portrait)",
    ]
    for i in range(1, targets["people"] + 1):
        role = roles[(i - 1) % len(roles)]
        prompt = (
            f"Photoreal headshot of a {role} at a cozy specialty shop. Warm natural light, friendly expression, simple clothing. "
            "Looks like a real staff bio photo but clearly synthetic, shallow depth of field, high detail."
        )
        add("people", i, prompt=prompt, negative=neg_photo, w=1024, h=1024, steps=35, guidance=6.0)

    # SOCIAL (mixed illustrative/templates)
    for i in range(1, targets["social"] + 1):
        if i % 3 == 1:
            prompt = (
                "Illustrative Instagram post template for Bramble & Bitters: cream background, bramble green header bar, copper accent line, "
                "space for a square photo, minimal leaf motif, clean typography, plenty of whitespace."
            )
        elif i % 3 == 2:
            prompt = (
                "Mixed-media Instagram story template: paper texture background in cream/fog, bramble green blocks, copper line accents, "
                "simple sticker shapes, placeholder text areas (not real readable text), boutique seasonal vibe."
            )
        else:
            prompt = (
                "Illustrative promotional flyer layout (square): 'seasonal box' promo vibe, cream background, bramble green headline area, copper button shape, "
                "small line-art leaf illustrations, modern classic aesthetic, no real logos."
            )
        add("social", i, prompt=prompt, negative=neg_social, w=1080, h=1080, steps=30, guidance=5.5)

    # Write JSONL
    PROMPTS_PATH.parent.mkdir(parents=True, exist_ok=True)
    PROMPTS_PATH.write_text("\n".join(j(x) for x in lines) + "\n", encoding="utf-8")

    total = sum(targets.values())
    print(f"Wrote {len(lines)} prompts (target {total}) to {PROMPTS_PATH}")


if __name__ == "__main__":
    main()
