# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: s-construction-v01
- Business name: Alder Ridge Builders
- Industry / domain: Construction (residential remodeling + small commercial tenant improvements)
- Business model: Project-based (fixed bid + time & materials change orders)
- Locations / operating region: Portland, OR metro
- Size tier: s
- Org maturity: emerging but disciplined (owner-led with strong job-cost habits)

## High-level description
- One-liner: A small GC running 6–12 active jobs/year with tight subcontractor relationships and fairly clean documentation.
- What they deliver:
  - Kitchen/bath remodels
  - ADUs
  - Small office tenant improvements
- Target customers:
  - Homeowners
  - Small property managers

## Operating model
- Functions:
  - Estimating
  - Project management
  - Field supervision
  - Bookkeeping
- Key roles (named people + titles):
  - Alex Rivera — Owner / Lead PM
  - Brooke Kim — Project Coordinator
  - Marcus Holt — Site Supervisor (lead carpenter)
  - “LedgerCraft Bookkeeping” — external bookkeeper

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **1**
- Rationale: small company, mostly single source of truth; docs are complete and consistent.
- Where chaos lives (minimal):
  - [x] minor naming drift (job names vs client legal names)
  - [ ] conflicting metrics (rare)
  - [ ] version sprawl (rare)
  - [ ] missing docs (rare)

## Intentional contradictions
- Deliberate contradictions: **no**
- Allowed minor real-world friction:
  - One invoice date mismatch vs payment received date
  - Job nickname vs legal name

## Artifact set (what documents should exist)
Mix of Markdown, JSON, XLSX, PPTX, and PDF.

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md

### Estimating + contracts
- company-capabilities-one-pager.pdf
- estimate-template.xlsx
- sample-estimate-adu-cedar-st.pdf
- contract-summary-adu-cedar-st.pdf
- change-order-log-adu-cedar-st.xlsx

### Project planning + delivery
- project-plan-adu-cedar-st.md
- schedule-adu-cedar-st.xlsx
- subcontractor-list.json
- rfi-log-adu-cedar-st.xlsx
- meeting-notes-adu-cedar-st-2026-01-10.md
- meeting-notes-adu-cedar-st-2026-02-07.md

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- job-costing-adu-cedar-st.xlsx
- accounts-receivable-2026-02.xlsx
- kpi-definitions.json

### Evaluation templates
- subcontractor-evaluation-template.md
- safety-walk-template.md
- client-intake-form.json

### Word documents (.docx)

- subcontractor-checklist-2026-02.docx

### Images (mixed; generated)
Documentation:
- assets/images/README.md

Folders:
- assets/images/logos/
- assets/images/people/
- assets/images/site/
- assets/images/diagrams/
- assets/images/social/

Metadata:
- assets/images/prompts.jsonl

Targets (exact for this case):
- logos: 4
- people: 10
- site: 12 (jobsite/exterior/interior progress; synthetic)
- diagrams: 8 (schedule snippets, budget breakdowns, process flow)
- social: 6

## Entity glossary (KG grounding)
- People: Alex Rivera, Brooke Kim, Marcus Holt
- Subcontractors: plumbing, electrical, HVAC, drywall, painter (5–10)
- Jobs:
  - ADU — Cedar St (primary)
  - Kitchen remodel — Hawthorne (secondary)
- Artifacts: estimates, change orders, RFIs, schedules, invoices, job costing

### Word documents (.docx)

- subcontractor-checklist-2026-02.docx

## Timeline of significant events
- 2024-04: Brooke hired (project coordination improves documentation).
- 2025-06: Marcus promoted to site supervisor (safety checklists introduced).
- 2026-01: ADU — Cedar St kicks off.
- 2026-02: Mid-project change order (scope add: upgraded windows).

## Benchmark goals (IKAM)
- Queries:
  1) “What is the latest schedule for ADU — Cedar St and what are the next milestones?”
  2) “What change orders exist and how do they affect budget + schedule?”
  3) “Which subcontractors are on this job and what are their scopes?”
  4) “What is job-to-date margin on ADU — Cedar St?”
