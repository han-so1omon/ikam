# Image Pack — Harborview Care Partners (l-healthcare-clinic-v01)

Synthetic images for IKAM multimodal benchmarks.

## Folders
- `logos/` — logo explorations
- `people/` — synthetic clinicians/staff headshots
- `clinic/` — clinic interiors/exteriors (no readable signage/addresses)
- `diagrams/` — KPI definition diagrams, compliance dashboards, patient flow
- `social/` — hiring + patient outreach templates

## Metadata
- `prompts.jsonl` (JSON per line)

Required keys: id, out_path, kind, prompt, negative_prompt, model, checkpoint, sampler, steps, guidance, width, height, seed, created_at
Optional: style_tags, notes, source_case_id

## Style
- Vibe: modern outpatient clinics.
- Palette: sea blue + slate + white.

## Safety
- No real clinic names/brands.
- No readable patient data.
