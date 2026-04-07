# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: s-manufacturing-v01
- Business name: Ridgeway Metal Works
- Industry / domain: Manufacturing (small metal fabrication + short-run parts)
- Business model: Job shop (quoted work + repeat customers)
- Locations / operating region: Spokane, WA (single shop)
- Size tier: s
- Org maturity: practical, low overhead

## High-level description
- One-liner: A small fab shop that runs on a few spreadsheets, a whiteboard schedule, and tribal knowledge.

## Operating model
- Functions:
  - Shop floor
  - Quoting
  - Purchasing
  - QA (light)
  - Bookkeeping
- Key roles:
  - Hank Miller — Owner
  - Zoe Park — Office/Operations
  - Eli Reyes — Lead Fabricator
  - “North Ledger” — bookkeeper

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **2**
- Rationale: small shop; docs are sparse but consistent.
- Where chaos lives:
  - [x] informal scheduling
  - [x] inventory counts manual
  - [ ] major version sprawl

## Intentional contradictions
- Deliberate contradictions: **no**
- Minor friction:
  - Material cost assumptions differ between quote and actual invoice due to steel price change.

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md

### Sales/quoting
- capabilities-one-pager.pdf
- quote-template.xlsx
- sample-quote-bracket-run.pdf
- customer-list.json

### Ops
- job-board-snapshot-2026-02.json
- production-schedule-2026-02.xlsx
- vendor-list.json
- inventory-count-sheet.xlsx
- incident-log-2026-q1.xlsx

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- job-costing-sample.xlsx
- kpi-definitions.json

### Templates
- supplier-evaluation-template.md
- nonconformance-report-template.md
- customer-intake-form.json

### Word documents (.docx)

- ops-brief-2026-q1.docx

### Images (mixed; generated)
Documentation:
- assets/images/README.md

Folders:
- assets/images/logos/
- assets/images/people/
- assets/images/shop/
- assets/images/products/
- assets/images/diagrams/

Metadata:
- assets/images/prompts.jsonl

Targets (exact):
- logos: 4
- people: 8
- shop: 8
- products: 12
- diagrams: 8

### Word documents (.docx)

- ops-brief-2026-q1.docx

## Timeline of significant events
- 2024-03: New laser cutter added.
- 2025-08: Repeat customer increases volume.
- 2026-01: Steel price bump impacts quotes.

## Benchmark goals (IKAM)
- Queries:
  1) “What jobs are in progress and what’s next?”
  2) “What vendors supply steel and what are lead times?”
  3) “Why did job margin differ from the quote?”
