# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: xl-manufacturing-v01
- Business name: Titan River Components
- Industry / domain: Manufacturing (automotive + industrial components)
- Business model: B2B supply (multi-plant, long-term contracts + custom runs)
- Locations / operating region: Midwest US (3 plants) + central HQ
- Size tier: xl
- Org maturity: heavy process surface area + complex systems + local workarounds

## High-level description
- One-liner: A multi-plant manufacturer with strict customer requirements, lots of KPIs, and constant reconciliation work across ERP/MES/quality systems.

## Operating model
- Functions:
  - Plant ops (3 plants)
  - Quality
  - Supply chain
  - Engineering
  - Finance
  - Program management
  - Safety
- Key roles:
  - Erica Shaw — COO
  - Victor Chen — VP Supply Chain
  - Amina Patel — VP Quality
  - Jonah Brooks — Finance Director
  - Luis Ramirez — Plant Manager (Plant A)
  - Priya Singh — Plant Manager (Plant B)
  - Omar Khan — Plant Manager (Plant C)

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **5**
- Rationale: enterprise-scale operations with multiple sources of truth.
- Where chaos lives:
  - [x] conflicting metrics (OEE, scrap, PPM definitions)
  - [x] version sprawl (SOPs, control plans)
  - [x] shadow spreadsheets (expedites, downtime logs)
  - [x] missing postmortems / incomplete CAPAs
  - [x] local plant workarounds

## Intentional contradictions
1) OEE:
   - Plant dashboards show OEE 78%
   - Finance ops review shows 72% (different availability assumptions)
2) Scrap/PPM:
   - Quality report excludes rework
   - Ops report includes rework as scrap
3) Inventory:
   - ERP valuation uses standard cost
   - Plant-level sheet uses last purchase price for key materials
4) Customer delivery:
   - Customer scorecard counts partial shipments as on-time
   - Internal counts full order completion date

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md
- org-structure.json

### Ops + production
- production-plan-2026-h1.xlsx
- downtime-log-plant-a-2026-01.xlsx
- oee-dashboard-2026-01.xlsx
- plant-scorecard-2026-01.xlsx
- safety-incident-log-2025.xlsx

### Quality
- qa-incidents-log-2025.xlsx
- capa-tracker-2026.xlsx
- control-plan-widget-line-v3.md
- control-plan-widget-line-v4.md (draft)
- customer-scorecard-2025-q4.pdf

### Supply chain
- vendor-risk-register.xlsx
- vendor-list.json
- expedite-tracker-2026-02.xlsx
- lead-time-dashboard-2026-01.xlsx

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- inventory-valuation-2025-12.xlsx
- kpi-definitions.json
- board-update-2026-02.pdf

### Program management
- program-roadmap-2026.pptx
- meeting-notes-program-review-2026-01-20.md
- meeting-notes-ops-review-2026-02-06.md

### Templates
- nonconformance-report-template.md
- supplier-evaluation-template.md
- expedite-request-form.json
- change-request-template.md

### Word documents (.docx)

- exec-ops-report-2026-02.docx

### Images (mixed; generated)
Documentation:
- assets/images/README.md

Folders:
- assets/images/logos/
- assets/images/people/
- assets/images/facility/
- assets/images/products/
- assets/images/diagrams/
- assets/images/social/

Metadata:
- assets/images/prompts.jsonl

Targets (exact):
- logos: 10
- people: 30
- facility: 20
- products: 30
- diagrams: 25
- social: 10

### Word documents (.docx)

- exec-ops-report-2026-02.docx

## Timeline of significant events
- 2024-06: ERP upgrade; reporting definitions drift.
- 2024-11: Customer scorecard dispute on partial shipments.
- 2025-05: Plant B adds a new line; downtime spikes.
- 2025-10: Audit findings require control plan updates (v4 drafted).
- 2026-01: CAPA backlog flagged.

## Benchmark goals (IKAM)
- Queries:
  1) “What is OEE and why do dashboards disagree?”
  2) “Which CAPAs are overdue and why?”
  3) “What control plan version is in use?”
  4) “What vendors are highest risk and what expedites are active?”
