# Image Pack — Cascadia Fasteners & Forming (l-manufacturing-v01)

Synthetic images for IKAM multimodal benchmarks.

## Folder structure
- `logos/` — flat/illustrative logo explorations
- `people/` — synthetic staff headshots (no real persons)
- `facility/` — plant / warehouse vibe shots (photoreal-ish)
- `products/` — product/part photos (fasteners, brackets, clips)
- `diagrams/` — manufacturing ops diagrams (process flow, OEE dashboard, SPC chart templates)
- `social/` — recruiting + customer update templates

## Metadata (required)
- `prompts.jsonl` — one JSON object per image.

Required keys: id, out_path, kind, prompt, negative_prompt, model, checkpoint, sampler, steps, guidance, width, height, seed, created_at
Optional: style_tags, notes, source_case_id

## House style
- Vibe: industrial, practical, safety-conscious, PNW manufacturing.
- Palette: steel gray, safety orange accent, white.

## Safety
- No real logos/brands.
- No readable serial numbers, badges, or license plates.
- People should be clearly synthetic.
