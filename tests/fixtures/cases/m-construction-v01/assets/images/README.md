# Image Pack — Beaconline Construction Co. (m-construction-v01)

Synthetic images for IKAM multimodal benchmarks.

## Folder structure
- `logos/` — flat/illustrative logo explorations
- `people/` — synthetic staff headshots (no real persons)
- `site/` — synthetic jobsite/progress photos (no readable addresses/plates)
- `diagrams/` — workflows (schedule snippets, WIP/margin chart, CO flow, RFI flow)
- `social/` — social templates (project updates, hiring, safety)

## Metadata (required)
- `prompts.jsonl` — one JSON object per image.

Required keys: id, out_path, kind, prompt, negative_prompt, model, checkpoint, sampler, steps, guidance, width, height, seed, created_at
Optional: style_tags, notes, source_case_id

## House style
- Vibe: professional mid-sized GC, practical, safety-forward.
- Palette: navy + safety yellow + off-white.

## Safety
- No real company names/logos.
- No readable street addresses, license plates, or personal data.
