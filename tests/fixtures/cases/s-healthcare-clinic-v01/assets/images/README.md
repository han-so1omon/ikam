# Image Pack — Pinecrest Family Clinic (s-healthcare-clinic-v01)

Synthetic images for IKAM multimodal benchmarks.

## Folders
- `logos/` — simple logo explorations
- `people/` — synthetic staff headshots
- `clinic/` — clinic interior/exterior (no readable address)
- `diagrams/` — basic process + KPI diagrams
- `social/` — simple patient reminder / hiring templates

## Metadata
- `prompts.jsonl` (one JSON object per line)

Required keys: id, out_path, kind, prompt, negative_prompt, model, checkpoint, sampler, steps, guidance, width, height, seed, created_at
Optional: style_tags, notes, source_case_id

## Style
- Vibe: calm, friendly neighborhood clinic.
- Palette: blue + teal + white.

## Safety
- No real clinic names.
- Avoid PHI.
