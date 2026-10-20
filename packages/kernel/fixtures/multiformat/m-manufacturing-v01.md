# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: m-manufacturing-v01
- Business name: Northpoint Packaging & Plastics
- Industry / domain: Manufacturing (packaging components: trays, inserts, custom plastic parts)
- Business model: B2B supply + custom runs
- Locations / operating region: Columbus, OH (plant + warehouse)
- Size tier: m
- Org maturity: process-emerging; ERP + lots of side spreadsheets

## High-level description
- One-liner: A mid-sized manufacturer juggling forecast volatility, scrap/rework, and vendor lead times.

## Operating model
- Functions:
  - Plant ops
  - QA
  - Procurement
  - Planning
  - Sales/CS
  - Finance
- Key roles:
  - Simone Grant — GM
  - Luis Park — Plant Manager
  - Anya Chen — QA Lead
  - Derek Rao — Procurement
  - Maya Ortiz — Planner
  - Jordan Blake — Finance

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **3**
- Rationale: some structure, but multiple systems and definition drift.
- Where chaos lives:
  - [x] scrap vs rework definitions differ between ops and QA
  - [x] inventory adjustments kept in side sheet
  - [x] vendor lead time tracking inconsistent
  - [x] forecast vs actual disagreements

## Intentional contradictions
- Moderate:
  1) Scrap rate: QA excludes rework; ops includes.
  2) Inventory valuation: finance uses standard cost; ops uses last purchase for resin.
  3) On-time delivery: customer scorecard counts partial shipments as on-time.

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md

### Sales + customer
- marketing-pitch-deck.pptx
- customer-scorecard-2025-q4.pdf
- top-customers.json

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- inventory-valuation-2025-12.xlsx
- kpi-definitions.json

### Ops + QA
- production-plan-2026-h1.xlsx
- qa-incidents-log-2025.xlsx
- scrap-rework-tracker-2026-01.xlsx
- sop-line-changeover-v2.md
- sop-line-changeover-v3.md (draft)
- vendor-list.json

### Meetings + notes
- weekly-ops-review-2026-01-06.md
- weekly-ops-review-2026-02-03.md
- voice-note-transcript-2026-02-04.md

### Templates
- supplier-evaluation-template.md
- nonconformance-report-template.md
- expedite-request-form.json

### Word documents (.docx)

- ops-review-2026-02.docx

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
- logos: 6
- people: 18
- facility: 10
- products: 20
- diagrams: 12
- social: 8

### Word documents (.docx)

- ops-review-2026-02.docx

## Timeline of significant events
- 2024-09: Resin supplier change after cost spike.
- 2025-04: New line added; changeover SOP drafted.
- 2025-11: Customer complaints on partial shipments.
- 2026-01: QA tightens incident logging; ops tracker still old.

## Benchmark goals (IKAM)
- Queries:
  1) “What is scrap rate and why do QA and ops disagree?”
  2) “Which vendors are highest risk?”
  3) “Why do inventory valuation methods differ?”
  4) “What SOP version is actually used for changeover?”
