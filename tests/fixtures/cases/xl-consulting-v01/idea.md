# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: xl-consulting-v01
- Business name: Alderpoint Strategy & Operations (ASO)
- Industry / domain: Consulting (enterprise strategy + ops transformation)
- Business model: project-based advisory + PMO + analytics enablement
- Locations / operating region: North America (multi-office)
- Size tier: xl
- Org maturity: huge template surface area + multiple practice groups + version sprawl

## High-level description
- One-liner: A large consulting firm running multiple workstreams for a client program, with conflicting numbers across decks, trackers, and finance.

## Operating model
- Functions:
  - Delivery (workstreams)
  - Sales/BD
  - Finance
  - Knowledge management
  - People ops
- Key roles:
  - Colleen Park — Managing Partner
  - Devin Holt — Program Director
  - Mira Shah — Workstream Lead (Ops)
  - Andre Kim — Workstream Lead (Finance)
  - Talia Reed — PMO Lead
  - Jordan Vega — Finance Ops

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **5**
- Rationale: many documents, many “final” versions, and inconsistent KPI / savings math.
- Where chaos lives:
  - [x] savings definitions (gross vs net, annualized vs run-rate)
  - [x] milestone dates differ across RAID log vs deck vs email
  - [x] timesheets vs invoices vs revenue recognition
  - [x] staffing plans drift weekly

## Intentional contradictions
1) Savings:
   - Exec deck shows $18.4M annualized savings
   - Finance model shows $14.9M net savings (different baseline + one-time costs)
2) Milestone date:
   - RAID log says phase 1 complete 2026-03-15
   - Email says 2026-03-29 (re-baseline)
3) Billing:
   - Timesheet totals 2,120 hours
   - Invoice bills 1,980 hours (write-offs + cap)

## Artifact set

### Firm identity
- mission-vision-values.md
- brand-guide.md
- practice-catalog.pdf
- org-structure.json

### Client program (primary)
Client: GraniteWorks Utilities
Program: Operations Transformation 2026
- sow-master.pdf
- msa.pdf
- statement-of-work-change-log.xlsx
- program-charter.md
- pmo-raID-log.xlsx
- master-program-plan-2026.xlsx
- exec-deck-2026-03.pptx
- workstream-deck-ops-v7.pptx
- workstream-deck-finance-v5.pptx
- client-email-thread-2026-03-18.md

### Analytics + savings
- savings-model-2026.xlsx
- benefits-tracker-2026-03.xlsx
- kpi-definitions.json

### Finance + delivery ops
- timesheet-2026-03.xlsx
- invoice-summary-2026-03.pdf
- revenue-history-2024-2025.xlsx
- pipeline-enterprise.xlsx

### Knowledge management
- deliverable-index.json
- slide-template-guidelines.md

### Templates
- meeting-notes-template.md
- raid-template.md
- issue-escalation-template.md

### Word documents (.docx)

- q4-impact-summary-2025.docx

### Images (mixed; generated)
Documentation:
- assets/images/README.md

Folders:
- assets/images/logos/
- assets/images/people/
- assets/images/office/
- assets/images/diagrams/
- assets/images/social/

Metadata:
- assets/images/prompts.jsonl

Targets (exact):
- logos: 8
- people: 24
- office: 14
- diagrams: 18
- social: 10

### Word documents (.docx)

- q4-impact-summary-2025.docx

## Timeline of significant events
- 2025-12: ASO wins GraniteWorks program.
- 2026-02: Baseline finalized (but later disputed).
- 2026-03: Phase 1 milestone re-baselined.

## Benchmark goals (IKAM)
- Queries:
  1) “What’s the true savings number and how is it defined?”
  2) “What is the current phase 1 milestone date and why does it differ?”
  3) “Why do billed hours differ from timesheets?”
  4) “Which deck versions are the latest across workstreams?”
