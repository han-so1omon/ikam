# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: l-manufacturing-v01
- Business name: Cascadia Fasteners & Forming (CFF)
- Industry / domain: Manufacturing (metal fasteners + small formed components)
- Business model: B2B supply (contract manufacturing + catalog SKUs)
- Locations / operating region: Tacoma, WA (main plant) + 3PL warehouse (Kent, WA)
- Size tier: l
- Org maturity: process-driven (ISO-ish instincts), but paperwork drifts

## High-level description
- One-liner: A regional manufacturer producing standard and custom fasteners/components for construction, HVAC, and light industrial customers.
- What they sell:
  - Standard SKUs (bolts, brackets, clips)
  - Custom runs (small formed parts)
  - Kitting/packaging for distributors
- Target customers:
  - Distributors
  - Construction supply
  - OEMs (HVAC, equipment)
- Differentiators:
  - Short lead times for custom runs
  - Practical engineering support
  - Reliable documentation (in theory)

## Operating model
- Functions:
  - Plant ops (stamping/forming)
  - QA
  - Procurement
  - Sales/CS
  - Finance
  - Engineering (light)
- Key roles (named people + titles):
  - Dana Whitaker — GM
  - Luis Ortega — Plant Manager
  - Anika Patel — QA Manager
  - Samir Khan — Procurement Lead
  - Renee Brooks — Finance Controller
  - Jun Park — Sales Director

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **2**
- Rationale: most processes exist and are followed, but cross-system reconciliation is imperfect.
- Where chaos lives:
  - [x] conflicting metrics (ERP vs spreadsheet rollups)
  - [x] versioning drift (SOP revisions not propagated)
  - [ ] naming conventions (mostly consistent)
  - [ ] missing documents (rare)
  - [x] shadow spreadsheets (inventory adjustments / expedite tracker)
  - [ ] messy meeting notes (generally structured)

## Intentional contradictions (smaller than chaos=4 case, but real)
1) Inventory valuation:
   - Finance sheet uses standard cost
   - Ops adjustment log uses last purchase price for some SKUs
2) On-time delivery (OTD):
   - Customer scorecard shows 96%
   - Internal dashboard shows 92% (different definition re: partial shipments)
3) Scrap rate:
   - QA report excludes rework
   - Ops report includes rework as scrap

## Artifact set (what documents should exist)
Mix of Markdown, JSON, XLSX, PPTX, and PDF.

### Strategy + identity
- mission-vision-values.md
- brand-guide.md (tone + palette + slide guidance)
- high-level-strategy-2026.md
- product-catalog-overview.md

### Sales + customer
- marketing-pitch-deck.pptx
- customer-scorecard-2025-q4.pdf
- top-customers.json
- price-list-2026-q1.xlsx

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- inventory-valuation-2025-12.xlsx
- capex-plan-2026.xlsx
- kpi-definitions.json

### Ops + QA
- production-plan-2026-h1.xlsx
- qa-incidents-log-2025.xlsx
- sop-press-line-setup-v3.md
- sop-press-line-setup-v4.md (updated but not fully adopted)
- vendor-list.json

### Meetings + planning
- weekly-ops-review-2025-12-03.md
- weekly-ops-review-2026-01-14.md
- voice-note-transcript-2026-01-22.md

### Evaluation templates
- supplier-evaluation-template.md
- nonconformance-report-template.md
- expedite-request-form.json

### Word documents (.docx)

- production-summary-2026-01.docx

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

Targets (exact for this case):
- logos: 5
- people: 15
- facility: 10
- products: 25
- diagrams: 15
- social: 10

## Entity glossary (KG grounding)
- People: Dana Whitaker, Luis Ortega, Anika Patel, Samir Khan, Renee Brooks, Jun Park
- Sites: Tacoma Plant, Kent 3PL
- Machines: Press Line 2, Press Line 4, Tumbler A
- SKUs: BOLT-M8-30, BRKT-L-02, CLIP-HVAC-07, (plus 10–15 more)
- Vendors: steel coil suppliers, plating vendor, packaging vendor
- Customers: 8–12 named accounts (distributors + OEMs)
- KPIs: OTD, scrap rate, rework rate, inventory turns, gross margin

### Word documents (.docx)

- production-summary-2026-01.docx

## Timeline of significant events
- 2024-05: GM change (Dana appointed).
- 2024-11: New plating vendor onboarded; early quality issue.
- 2025-04: Press Line 4 added (capacity increase).
- 2025-09: Customer complaint spike about delayed partial shipments (definition dispute begins).
- 2025-12: Year-end inventory adjustments done in spreadsheet; finance disagrees on valuation method.
- 2026-01: SOP v4 drafted for press setup but floor still using v3.
- 2026-02: Capex request for QA measurement equipment.

## Benchmark goals (IKAM)
- Queries:
  1) “What is the current SOP for press line setup, and which version is actually used?”
  2) “Why do OTD metrics disagree (96% vs 92%)?”
  3) “What vendors are highest risk and why?”
  4) “Reconcile inventory valuation differences.”
- Ambiguities to resolve/flag:
  - definitions (OTD, scrap)
  - SOP version adoption vs publication
