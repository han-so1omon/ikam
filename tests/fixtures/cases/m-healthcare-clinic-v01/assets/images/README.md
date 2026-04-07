# Image Pack — Mesa Ridge Urgent & Primary Care (m-healthcare-clinic-v01)

Synthetic images for IKAM multimodal benchmarks.

## Folders
- `logos/` — brand/logo explorations
- `people/` — synthetic staff headshots
- `clinics/` — clinic interiors/exteriors (no readable addresses)
- `diagrams/` — ops/billing/privacy diagrams
- `social/` — patient comms + hiring templates

## Metadata
- `prompts.jsonl` (one JSON object per line)

Required keys: id, out_path, kind, prompt, negative_prompt, model, checkpoint, sampler, steps, guidance, width, height, seed, created_at
Optional: style_tags, notes, source_case_id

## Style
- Vibe: calm, friendly clinic group.
- Palette: blue + teal + white.

## Safety
- No real clinic names.
- Avoid any realistic PHI.
