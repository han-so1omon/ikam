# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: xl-local-retail-v01
- Business name: Sunbeam Market Collective
- Industry / domain: Local retail (multi-location specialty grocery + prepared foods)
- Business model: Retail + catering + subscription meal kits (pilot)
- Locations / operating region: Bay Area, CA (6 locations) + commissary kitchen
- Size tier: xl
- Org maturity: corporate surface area (policies, templates) + messy reality

## High-level description
- One-liner: A fast-growing specialty grocer with multiple stores, a commissary, and a half-built subscription program.
- What they sell:
  - In-store grocery + prepared foods
  - Catering trays + corporate lunches
  - Meal-kit subscription pilot

## Operating model
- Functions:
  - Store ops (6 store managers)
  - Merchandising
  - Marketing
  - Finance
  - HR
  - Supply chain
  - Commissary kitchen
- Key roles (named people + titles):
  - Camille Torres — COO
  - Jonah Feld — CFO
  - Imani Brooks — Director of Marketing
  - Wes Park — Director of Supply Chain
  - Hana Kim — HR Lead
  - (Store managers) Priya, Marco, Elena, Devon, Sam, Tori

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **5**
- Rationale: multi-location operations + many systems + frequent promotions.
- Where chaos lives:
  - [x] conflicting metrics (same KPI defined differently by store ops vs finance)
  - [x] version sprawl (promo calendars, brand guides, SOPs)
  - [x] shadow spreadsheets (waste logs, labor scheduling)
  - [x] messy meeting notes (store calls)
  - [x] missing docs (postmortems, some SOP signoffs)
  - [x] naming drift (product lines and promo names)

## Intentional contradictions
1) Revenue reporting:
   - Weekly store ops report shows higher revenue (gross) than finance net (returns/voids)
2) Waste:
   - Commissary waste log claims waste down 20%
   - Finance shrink summary shows waste/shrink flat
3) Subscription program naming:
   - “Sunbeam Kits” vs “Market Box” vs “Meal Club”
4) Headcount:
   - HR roster counts 240 employees
   - Ops staffing plan claims 265 (includes temps)

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md
- org-structure.json

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
- shrink-waste-summary-2025.xlsx
- kpi-definitions.json

### Ops + supply chain
- vendor-list.json
- inventory-replenishment-plan.xlsx
- labor-scheduling-template.xlsx
- sop-prepared-foods-v2.md
- sop-prepared-foods-v3.md (draft rollout)
- incident-log-2026-q1.xlsx

### HR
- hiring-plan-2026.xlsx
- employee-handbook-excerpt.pdf
- performance-review-template.md

### Meetings + notes
- store-manager-call-2026-01-08.md
- store-manager-call-2026-02-05.md
- voice-note-transcript-2026-02-12.md

### Evaluation templates
- vendor-scorecard-template.md
- promo-postmortem-template.md
- catering-order-intake-form.json

### Word documents (.docx)

- weekly-ops-report-2026-02-15.docx

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
- logos: 10
- people: 35
- products: 40
- stores: 15
- social: 25
- diagrams: 20

### Word documents (.docx)

- weekly-ops-report-2026-02-15.docx

## Timeline of significant events
- 2024-03: Opened location #4; supply chain strain begins.
- 2024-10: Commissary kitchen launched; waste tracking split across teams.
- 2025-05: Meal-kit pilot launched (name varies).
- 2025-11: Holiday promo incident (stockout + customer complaints).
- 2026-01: SOP v3 drafted but stores still use v2.
- 2026-02: CFO pushes KPI definition registry; adoption uneven.

## Benchmark goals (IKAM)
- Queries:
  1) “What is the subscription program called and what’s its status?”
  2) “Why do revenue numbers differ between store ops and finance?”
  3) “Which locations are underperforming and why?”
  4) “What SOP version is actually in use for prepared foods?”
