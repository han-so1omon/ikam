# Image Pack — Ridgeway Metal Works (s-manufacturing-v01)

Synthetic images for IKAM multimodal benchmarks.

## Folders
- `logos/` — logo explorations
- `people/` — synthetic staff headshots
- `shop/` — small fab shop scenes
- `products/` — parts/product shots (brackets, plates)
- `diagrams/` — simple job board, schedule, job costing diagrams

## Metadata
- `prompts.jsonl` (JSON per line)

Required keys: id, out_path, kind, prompt, negative_prompt, model, checkpoint, sampler, steps, guidance, width, height, seed, created_at
Optional: style_tags, notes, source_case_id

## Style
- Vibe: practical job shop.
- Palette: steel gray + orange.

## Safety
- No real brands.
- No readable serial numbers.
