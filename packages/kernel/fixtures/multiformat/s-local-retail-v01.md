# IKAM Benchmark Case Spec (.idea)

## Identity
- Case ID: s-local-retail-v01
- Business name: Bramble & Bitters
- Industry / domain: Local retail (specialty foods + gifts)
- Business model: Product retail + small subscription boxes + local corporate gift bundles
- Locations / operating region: Oakland, CA (one storefront) + ship-to (CA/US limited)
- Size tier: s
- Org maturity: emerging (owner-led, light process)

## High-level description
- One-liner: A neighborhood specialty shop selling small-batch pantry items, giftable goods, and seasonal subscription boxes.
- What they sell / deliver:
  - In-store retail: jams, hot sauces, chocolates, tea/coffee accessories, gift bundles
  - Online: seasonal boxes, limited shipping, local delivery
  - Corporate: small “thank you” gift packs for local companies
- Target customers:
  - Local foot traffic (30–55)
  - Gift buyers (holidays, birthdays)
  - Local small businesses ordering employee gifts
- Competitors / alternatives:
  - Other boutiques
  - Farmers markets
  - Big-box + Amazon gift bundles
- Differentiators:
  - Curated selection, rotating local makers
  - Friendly in-store experience
  - Seasonal curation + “tasting notes” style product descriptions

## Operating model
- Teams / functions:
  - Owner/GM
  - Retail ops (part-time)
  - Marketing (contractor)
  - Bookkeeping (external)
- Key roles (named people + titles):
  - Maya Chen — Owner / General Manager
  - Jordan Reyes — Shift Lead (part-time → promoted)
  - Talia Singh — Marketing Contractor (social + email)
  - “North Ledger LLC” — Bookkeeping firm
- Tools (realism anchors):
  - POS: Square
  - Email: Google Workspace
  - Docs: Google Drive + exported PDFs
  - Social: Instagram, occasional TikTok
  - Light project tracking: a shared “To Do” note + occasional kanban screenshot

## Chaos profile (level 3; deliberate contradictions)
- Overall chaos level (1–5): 3
- Where chaos lives:
  - [x] naming conventions (products/projects have 2–3 names)
  - [x] versioning / duplicates (multiple “final” pitch decks)
  - [x] conflicting metrics (revenue differs by doc source)
  - [x] stale strategy/roadmap (holiday plan not updated after pivot)
  - [ ] missing documents (most exist, but some are partial)
  - [x] shadow IT / side spreadsheets (inventory + gifting kept in separate sheets)
  - [x] messy meeting notes (voice-note transcriptions, informal bullets)
  - [ ] unclear ownership/RACI (small team; mostly clear)
- Intentional contradictions to include across artifacts:
  1) 2025 Q4 revenue:
     - Pitch deck claims: $310k
     - Bookkeeping summary shows: $286k
     - POS export totals: $295k (excludes returns handled manually)
  2) Subscription program name:
     - “Seasonal Pantry Box” vs “Bramble Box” vs “Quarterly Crate”
  3) Product margin:
     - Strategy doc states target gross margin 52%
     - Finance sheet uses 48% blended margin assumption
  4) Headcount:
     - Brand one-pager lists “5 employees” (counts contractors)
     - HR notes list 3 staff + 1 contractor

## Financial shape (benchmark intent)
- Currency: USD
- Revenue streams:
  - In-store retail
  - Online orders (shipping + local delivery)
  - Subscription boxes
  - Corporate gifting bundles
- Cost centers:
  - COGS (makers/vendors)
  - Rent + utilities
  - Labor
  - Packaging + shipping
  - Marketing spend
- Reporting cadence:
  - Formal: quarterly (bookkeeper)
  - Informal: monthly “gut check” notes by owner

## Artifact set (what documents should exist)
Generate documents with a mix of polish and mess. Include both Markdown and JSON.

### Strategy + identity
- mission-vision-values.md
- brand-guide.md (lightweight; includes tone, colors as names not hex)
- high-level-strategy-2026.md
- annual-goals-okrs-2026.md

### Marketing + sales
- marketing-pitch-deck.pptx (v1)
- marketing-pitch-deck-final-final.pptx (v2, with slightly different numbers)
- messaging-pillars.md
- customer-personas.md
- corporate-gifting-one-pager.pdf (exported; may be dated)

### Finance
- projected-revenue-2026.xlsx
- quarterly-revenue-history-2024-2025.xlsx
- bookkeeping-q4-2025-summary.pdf
- pos-export-2025-q4.json (raw-ish export)
- kpi-definitions.json (definitions + formulas as strings)

### Ops + planning
- roadmap-2026-h1.md
- inventory-plan-spring-2026.xlsx
- vendor-list.json
- prior-development-cycles.md (describes how they run “sprints” around holidays)

### Meetings + notes
- meeting-notes-2025-10-15.md (vendor planning)
- meeting-notes-2025-11-20.md (holiday staffing)
- voice-note-transcript-2026-01-07.md (owner ramble; high signal)

### Evaluation templates
- vendor-evaluation-template.md
- corporate-order-intake-form.json
- campaign-postmortem-template.md

### Word documents (.docx)

- holiday-recap-2025.docx
- spring-promo-brief-2026.docx

### Images (mixed; generated)
Generate a case-consistent image pack for benchmarking multimodal ingestion.

Documentation:
- assets/images/README.md (house style, safety constraints, naming)

Folder convention:
- assets/images/products/
- assets/images/storefront/
- assets/images/people/
- assets/images/social/
- assets/images/logos/

Metadata (required):
- assets/images/prompts.jsonl (one JSON object per line)
  - required keys: id, out_path, kind, prompt, negative_prompt, model, sampler, steps, guidance, width, height, seed, created_at
  - optional keys: style_tags, notes, source_case_id

Targets (approx):
- products: 30–50 (photoreal-ish)
- storefront: 5–10 (photoreal-ish)
- people: 10–20 (staff headshots; clearly synthetic)
- social: 20–40 (illustrative + photo-mixed templates)
- logos: 5–10 (flat/illustrative variations)

## Entity glossary (for knowledge graph grounding)
- People:
  - Maya Chen
  - Jordan Reyes
  - Talia Singh
  - (North Ledger LLC — org)
- Teams/functions:
  - Retail Ops
  - Marketing
  - Bookkeeping
- Products/services:
  - Subscription boxes (aliases listed above)
  - Corporate gift bundles
  - In-store curated retail assortment
- Customers:
  - Walk-in retail customers (aggregate)
  - A few named corporate clients (3–5)
- Vendors:
  - 10–15 local makers (some with duplicate names / LLC vs brand)
- Projects/initiatives:
  - Holiday 2025 push
  - “Subscription refresh” (aka Box Revamp)
  - Local delivery pilot
- KPIs/metrics:
  - Revenue (POS vs bookkeeping)
  - Gross margin
  - Average order value (AOV)
  - Subscription churn (rough)
  - Corporate order lead time

### Word documents (.docx)

- holiday-recap-2025.docx
- spring-promo-brief-2026.docx

## Timeline of significant events (required)
- 2024-03: Opened storefront (soft launch) — Maya — references in origin story + early revenue notes.
- 2024-08: First corporate gifting order (Bayworks Co.) — created a simple intake spreadsheet.
- 2025-02: Hired Jordan as part-time associate — later becomes shift lead.
- 2025-06: Switched packaging vendor after cost increase — affects margin assumptions.
- 2025-09: Launched subscription program (name varies) — early churn noted informally.
- 2025-11: Holiday staffing crunch — added two seasonal associates (names appear inconsistently).
- 2025-12: Big holiday season; returns handled manually for a week due to Square workflow issue — explains POS vs bookkeeping mismatch.
- 2026-01: Marketing contractor Talia engaged (trial) — changes voice + cadence; brand guide updated but not propagated.
- 2026-02: Pivot: reduce shipping footprint (only CA + nearby) due to fulfillment pain — roadmap doc partially updated.

## Benchmark goals (IKAM)
- Queries IKAM should answer:
  1) “What is the subscription program called, and when did it launch?” (should return aliases + launch month + confidence)
  2) “Why do Q4 2025 revenue numbers disagree?” (should attribute to source definitions/returns)
  3) “What are 2026 H1 priorities and who owns them?”
  4) “Which vendors are highest risk and why?” (from evaluations + notes)
- Hard queries (cross-doc reasoning):
  - Connect staffing timeline to service levels and corporate order lead time.
  - Reconcile margin target vs margin used in projections.
- Ambiguities IKAM must resolve (or flag):
  - POS export vs bookkeeping totals
  - Contractor vs employee counting
  - Project codenames ("Subscription refresh" vs "Box Revamp")

## Dedup opportunities (explicit CAS anchors)

To force measurable fragment dedup in this case, repeat the exact canonical snippets below across multiple artifacts (Markdown, JSON, PDF source markdown, and slides notes) instead of paraphrasing.

- Canonical people block (copy verbatim in 5+ files):
  - `Maya Chen | Owner / General Manager`
  - `Jordan Reyes | Shift Lead`
  - `Talia Singh | Marketing Contractor`
  - `North Ledger LLC | Bookkeeping`

- Canonical KPI block (copy verbatim in 4+ files):
  - `Q4 2025 Revenue (POS): 295000`
  - `Q4 2025 Revenue (Bookkeeping): 286000`
  - `Q4 2025 Revenue (Pitch): 310000`
  - `Target Gross Margin: 52%`
  - `Projection Gross Margin: 48%`

- Canonical initiative alias block (copy verbatim in 4+ files):
  - `Subscription Program Aliases: Seasonal Pantry Box | Bramble Box | Quarterly Crate`
  - `Initiative Alias: Subscription Refresh | Box Revamp`

- Canonical contradiction explanation line (copy verbatim in 3+ files):
  - `Returns were processed manually for one week in Dec 2025, causing POS and bookkeeping variance.`

- Canonical ownership line for roadmap rows (copy verbatim in 3+ files):
  - `Owner: Maya Chen; Support: Jordan Reyes; Advisor: Talia Singh`

Additional candidates from the current case directory worth normalizing into repeated canonical snippets:
- Packaging vendor switch event (currently phrased differently across notes/strategy/roadmap)
- Shipping footprint pivot (`CA + nearby`) references
- Corporate gifting lead-time language (`intake`, `fulfillment`, `approval`) across templates and one-pager
- Subscription churn phrasing between voice notes and KPI definitions
