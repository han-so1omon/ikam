# Image Pack — Sunbeam Market Collective (xl-local-retail-v01)

Synthetic images for IKAM multimodal benchmarks.

## Folders
- `logos/` — brand/logo explorations
- `people/` — synthetic staff headshots
- `products/` — product shots (prepared foods, grocery items, bundles)
- `stores/` — store interiors/exteriors, commissary kitchen vibe
- `social/` — promo templates
- `diagrams/` — ops/finance dashboards, supply chain maps, SOP flow diagrams

## Metadata
- `prompts.jsonl` (one JSON object per line)

Required keys: id, out_path, kind, prompt, negative_prompt, model, checkpoint, sampler, steps, guidance, width, height, seed, created_at
Optional: style_tags, notes, source_case_id

## Style
- Vibe: bright, friendly specialty grocer; operationally serious.
- Palette: sun-yellow + deep green + off-white.

## Safety
- No real brands, no readable personal info.
