# Image Pack — Driftwood & Oak (m-local-retail-v01)

Synthetic images for IKAM multimodal benchmarks.

## Folders
- `logos/` — brand/logo explorations
- `people/` — synthetic staff headshots
- `products/` — product shots (home goods, gift items)
- `stores/` — store interiors/exteriors (2 locations)
- `social/` — promo templates
- `diagrams/` — ops dashboards, promo calendar, inventory flow

## Metadata
- `prompts.jsonl` (one JSON object per line)

Required keys: id, out_path, kind, prompt, negative_prompt, model, checkpoint, sampler, steps, guidance, width, height, seed, created_at
Optional: style_tags, notes, source_case_id

## Style
- Vibe: modern warm boutique.
- Palette: oak brown + cream + muted blue.

## Safety
- No real brands, no readable personal info.
