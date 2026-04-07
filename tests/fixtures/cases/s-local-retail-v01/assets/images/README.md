# Image Pack — Bramble & Bitters (s-local-retail-v01)

This folder contains **synthetic** images intended for IKAM multimodal ingestion benchmarks.

## Goals
- Provide realistic brand-adjacent visuals (product shots, storefront, staff, social templates).
- Maintain internal consistency (palette + vibe), while allowing variation.
- Include machine-readable metadata linking each image to its generation prompt.

## Folder structure
- `products/` — photoreal-ish product images (square, catalog-style)
- `storefront/` — photoreal-ish exterior/interior vibe shots (landscape)
- `people/` — staff headshots (square); avoid any real person resemblance
- `social/` — illustrative or mixed-media social templates (square + story)
- `logos/` — flat/illustrative logo explorations

## Metadata (required)
- `prompts.jsonl` — one JSON object per image.

Required keys:
- `id`
- `out_path`
- `kind` (product|storefront|person|social|logo)
- `prompt`
- `negative_prompt`
- `model`
- `sampler`
- `steps`
- `guidance`
- `width`, `height`
- `seed`
- `created_at`

Optional keys:
- `style_tags`
- `notes`
- `source_case_id`

## House style (guidance)
Palette (preferred):
- Bramble Green `#1F3D2B`
- Cream `#F4EFE6`
- Copper `#B87333`
- Ink `#1A1A1A`
- Fog `#D7D2C8`

Vibe keywords:
- cozy, curated, warm natural light, neighborhood shop, seasonal

## Safety + realism constraints
- No real brands, logos, or identifiable places.
- No celebrity likeness prompts.
- Keep faces and signage clearly synthetic.
- Avoid generating realistic IDs, license plates, or personal data.

## Naming convention
Use `kind-###.png` (or `.jpg`) and keep `id` aligned with the filename.

Examples:
- `products/product-014.png`
- `social/social-008.png`
