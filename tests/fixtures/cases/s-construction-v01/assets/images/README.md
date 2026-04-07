# Image Pack — Alder Ridge Builders (s-construction-v01)

Synthetic images for IKAM multimodal benchmarks.

## Folder structure
- `logos/` — flat/illustrative logo explorations
- `people/` — synthetic staff headshots (no real persons)
- `site/` — synthetic jobsite/progress photos (no readable addresses/plates)
- `diagrams/` — construction workflows (schedule snippets, budget tables, RFI flow)
- `social/` — simple social templates (project updates, hiring)

## Metadata (required)
- `prompts.jsonl` — one JSON object per image.

Required keys: id, out_path, kind, prompt, negative_prompt, model, checkpoint, sampler, steps, guidance, width, height, seed, created_at
Optional: style_tags, notes, source_case_id

## House style
- Vibe: trustworthy local GC, clean documentation, pragmatic.
- Palette: forest green, warm gray, off-white.

## Safety
- No real company logos.
- No readable street addresses, license plates, or personal data.
