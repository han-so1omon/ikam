# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: l-local-retail-v01
- Business name: Juniper & Juno Goods
- Industry / domain: Local retail (home goods + small gifts + seasonal pop-ups)
- Business model: Retail (3 locations) + online + occasional wholesale
- Locations / operating region: Los Angeles, CA (3 stores) + ship-to (US)
- Size tier: l
- Org maturity: mixed — strong merchandising instincts, uneven ops discipline

## High-level description
- One-liner: A small chain boutique with three locations and an online store; promotions and inventory are a constant tug-of-war.
- What they sell:
  - Home goods, candles, stationery, small gifts
  - Seasonal pop-up collections
  - Limited wholesale to 2–3 partner stores

## Operating model
- Functions:
  - Store ops
  - Merchandising/buying
  - Marketing
  - Finance/bookkeeping
  - Fulfillment
- Key roles (named people + titles):
  - Lila Moreno — Founder / Creative Director
  - Ben Carter — Ops Manager
  - Noor Patel — Merchandising Lead
  - Keisha Wong — Marketing Lead (contractor)
  - “West Ledger Co.” — Bookkeeping
  - Store managers: Aria (DTLA), Mateo (Silver Lake), Jules (Venice)

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **4**
- Rationale: multiple locations + promos + inventory = lots of drift.
- Where chaos lives:
  - [x] conflicting metrics (sell-through vs margin reporting)
  - [x] version sprawl (promo calendar, brand guide, pricing sheets)
  - [x] shadow spreadsheets (inventory adjustments, vendor POs)
  - [x] messy meeting notes (store calls)
  - [x] naming drift (collections, SKUs, promo names)
  - [ ] missing docs (some, especially postmortems)

## Intentional contradictions
1) Revenue reporting:
   - Ops weekly report uses gross sales
   - Bookkeeping uses net sales (returns + discounts)
2) Inventory accuracy:
   - Store counts say candles are overstocked
   - Warehouse sheet shows backorder
3) Promo naming:
   - “Spring Reset” vs “Refresh Week” vs “New Season Drop”
4) Headcount:
   - Staffing plan counts temps as staff
   - HR roster excludes temps

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md

### Marketing + merchandising
- marketing-pitch-deck.pptx
- promo-calendar-2026-h1.xlsx
- campaign-brief-spring.md
- messaging-pillars.md
- product-style-guide.md

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- store-performance-weekly-2026-02.xlsx
- kpi-definitions.json

### Ops
- vendor-list.json
- inventory-replenishment-plan.xlsx
- inventory-adjustments-2026-01.xlsx
- labor-scheduling-template.xlsx
- incident-log-2026-q1.xlsx

### Meetings + notes
- store-manager-call-2026-01-09.md
- store-manager-call-2026-02-06.md
- voice-note-transcript-2026-02-13.md

### Evaluation templates
- vendor-scorecard-template.md
- promo-postmortem-template.md
- wholesale-order-intake-form.json

### Word documents (.docx)

- weekly-ops-report-2026-02-16.docx

### Images (mixed; generated)
Documentation:
- assets/images/README.md

Folders:
- assets/images/logos/
- assets/images/people/
- assets/images/products/
- assets/images/stores/
- assets/images/social/
- assets/images/diagrams/

Metadata:
- assets/images/prompts.jsonl

Targets (exact):
- logos: 7
- people: 20
- products: 30
- stores: 8
- social: 15
- diagrams: 12

### Word documents (.docx)

- weekly-ops-report-2026-02-16.docx

## Timeline of significant events
- 2024-08: Opened 3rd location (Venice) — inventory complexity jumps.
- 2025-03: Began online shipping expansion.
- 2025-09: Major promo created returns spike; net vs gross debate.
- 2026-01: Ops manager tries to standardize SKU naming; partial adoption.
- 2026-02: Merchandising pushes a new collection naming scheme.

## Benchmark goals (IKAM)
- Queries:
  1) “Why do sales numbers differ between ops and bookkeeping?”
  2) “Which SKUs are overstocked vs backordered and why?”
  3) “What promos are planned and what are their names/aliases?”
  4) “Which store is underperforming in Feb 2026 and why?”
