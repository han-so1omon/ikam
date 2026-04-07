# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: l-construction-v01
- Business name: Ironwood Ridge Constructors
- Industry / domain: Construction (commercial GC + tenant improvements)
- Business model: project-based (GMP + fixed bid) + small service team
- Locations / operating region: Southwest US (AZ/NM), 2 offices
- Size tier: l
- Org maturity: established, but lots of project variance and spreadsheet reality

## High-level description
- One-liner: A large-ish regional GC with many active jobs and predictable mess: schedule drift, CO confusion, and PM forecasts that don’t match accounting.

## Operating model
- Functions:
  - Precon/estimating
  - Operations (PMs + supers)
  - Safety
  - Accounting/WIP
  - Procurement
  - Legal/contracts (light)
- Key roles:
  - Erin Caldwell — President
  - Mateo Reyes — VP Operations
  - Kira Olsen — Director of Precon
  - Nolan Price — Controller
  - Leah Kim — Safety Manager

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **4**
- Rationale: many moving parts, but fewer systems than XL; still multiple truths.
- Where chaos lives:
  - [x] schedule versions (owner vs internal)
  - [x] CO totals (approved vs pending vs forecast)
  - [x] job-cost forecast vs WIP
  - [x] RFI/submittal tracking outside the main PM system

## Intentional contradictions
1) Substantial Completion:
   - Owner update email: 2026-10-18
   - Internal schedule: 2026-11-05 (weather + inspections)
2) Change Orders:
   - CO log approved totals: $740k
   - Exec summary says $1.05M (includes pending)
3) Margin:
   - PM forecast: 6.2%
   - WIP report: 4.6%

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md
- org-structure.json

### Estimating + contracts
- company-capabilities-one-pager.pdf
- estimate-template.xlsx
- contract-summary-ownerletter.pdf

### Delivery (primary project)
Project: Desert Vista Medical Office (Job #DV-3291)
- project-plan-desert-vista.md
- master-schedule-desert-vista.xlsx
- owner-update-email-2026-09-12.md
- lookahead-schedule-desert-vista.xlsx
- rfi-log-desert-vista.xlsx
- submittal-log-desert-vista.xlsx
- change-order-log-desert-vista.xlsx
- meeting-notes-desert-vista-2026-09-06.md
- meeting-notes-desert-vista-2026-10-04.md
- exec-summary-email-2026-10-05.md

### Finance
- job-costing-desert-vista.xlsx
- wip-report-2026-09.xlsx
- accounts-receivable-2026-09.xlsx
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- kpi-definitions.json

### Safety + quality
- safety-walk-template.md
- safety-incident-log-2026.xlsx
- qaqc-punchlist-template.xlsx

### Templates
- rfi-template.md
- subcontractor-evaluation-template.md
- owner-change-request-form.json

### Word documents (.docx)

- milestone-report-2026-02-20.docx

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

Targets (exact):
- logos: 8
- people: 18
- site: 26
- diagrams: 16
- social: 8

### Word documents (.docx)

- milestone-report-2026-02-20.docx

## Timeline of significant events
- 2025-07: Expanded into TI work; more parallel jobs.
- 2026-08: Desert Vista long-lead MEP equipment slips.
- 2026-09: Owner requests earlier substantial completion.
- 2026-10: CO totals and margin questioned in exec review.

## Benchmark goals (IKAM)
- Queries:
  1) “What’s the latest substantial completion date and why do sources differ?”
  2) “Approved vs pending CO totals — what’s the real number?”
  3) “Reconcile PM forecast margin vs accounting WIP.”
  4) “Top open RFIs/submittals and owners.”
