# Image Pack — Northlake Advisory Group (m-consulting-v01)

Synthetic images for IKAM multimodal benchmarks.

## Folder structure
- `logos/` — flat/illustrative logo explorations
- `people/` — synthetic staff headshots (no real persons)
- `social/` — social templates (mostly illustrative/mixed)
- `diagrams/` — consulting-style frameworks (swimlanes, matrices, funnels, dashboards)

## Metadata (required)
- `prompts.jsonl` — one JSON object per image.

Required keys: id, out_path, kind, prompt, negative_prompt, model, checkpoint, sampler, steps, guidance, width, height, seed, created_at
Optional: style_tags, notes, source_case_id

## House style (guidance)
- Vibe: crisp, professional, mid-market, “operator-led”, minimal but confident.
- Palette (suggested): navy, off-white, teal accent.

## Safety
- No real client logos or identifiable people.
- No real company names beyond the fictional ones in this case.
