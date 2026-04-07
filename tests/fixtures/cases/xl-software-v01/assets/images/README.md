# Image Pack — Meridian SignalWorks (xl-software-v01)

Synthetic images for IKAM multimodal benchmarks.

## Folder structure
- `logos/` — flat/illustrative logo explorations
- `people/` — synthetic staff headshots (no real persons)
- `product-ui/` — synthetic UI screenshots/illustrations (dashboards, settings, flows)
- `diagrams/` — architecture diagrams, incident timelines, KPI dashboards
- `social/` — social templates (launch posts, hiring, event promos)

## Metadata (required)
- `prompts.jsonl` — one JSON object per image.

Required keys: id, out_path, kind, prompt, negative_prompt, model, checkpoint, sampler, steps, guidance, width, height, seed, created_at
Optional: style_tags, notes, source_case_id

## House style
- Vibe: modern B2B SaaS, crisp, friendly, “enterprise-grade” but not sterile.
- Palette: dark navy + teal + off-white.

## Safety
- No real company names/logos.
- Avoid realistic personal data in UIs.
- People should be clearly synthetic.
