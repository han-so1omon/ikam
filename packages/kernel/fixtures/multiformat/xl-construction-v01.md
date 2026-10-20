# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: xl-construction-v01
- Business name: SummitSpan Builders Group
- Industry / domain: Construction (commercial GC + multifamily + light civil)
- Business model: Project-based (GMP, design-build, fixed bid) + service division
- Locations / operating region: Pacific Northwest (regional), 4 offices
- Size tier: xl
- Org maturity: heavy process surface + constant exceptions

## High-level description
- One-liner: A large regional GC with many concurrent projects, lots of documentation, and multiple parallel tracking systems.

## Operating model
- Functions:
  - Precon/estimating
  - PMO
  - Project teams (PMs, supers)
  - Safety
  - Quality
  - Accounting/WIP
  - Legal/Contracts
  - HR
- Key roles:
  - Dana Whitfield — CEO
  - Marcus Lee — COO
  - Priya Nair — VP Precon
  - Elena Ortiz — VP Operations
  - Sam Cho — Controller
  - Tasha Reed — Safety Director

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **5**
- Rationale: many projects, many systems, conflicting definitions and version sprawl.
- Where chaos lives:
  - [x] schedule version drift (P6 vs Excel vs meeting notes)
  - [x] change order totals (approved vs pending vs forecast)
  - [x] cost-to-complete differences (PM forecast vs accounting WIP)
  - [x] RFI/submittal tracking in shadow sheets
  - [x] messy meeting notes and emails
  - [x] inconsistent project naming and job numbers

## Intentional contradictions
1) Substantial Completion date:
   - PM weekly notes say 2026-07-15
   - Master schedule shows 2026-08-02 (re-baselined)
2) Change orders:
   - CO log sums approved to $2.1M
   - Exec summary claims $2.6M (includes pending)
3) Margin:
   - PM forecast shows 5.5% fee
   - Accounting WIP implies 3.8% margin

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md
- org-structure.json

### Estimating + contracts
- company-capabilities-one-pager.pdf
- estimate-template.xlsx
- sample-estimate-multifamily-riverwalk.pdf
- contract-summary-multifamily-riverwalk.pdf

### Delivery (primary project)
Project: Multifamily — Riverwalk (Job #RW-4412)
- project-plan-riverwalk.md
- master-schedule-riverwalk.xlsx
- lookahead-schedule-riverwalk.xlsx
- rfi-log-riverwalk.xlsx
- submittal-log-riverwalk.xlsx
- change-order-log-riverwalk.xlsx
- meeting-notes-riverwalk-2026-05-08.md
- meeting-notes-riverwalk-2026-06-05.md
- meeting-notes-riverwalk-2026-07-03.md
- exec-summary-email-2026-07-04.md

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- job-costing-riverwalk.xlsx
- wip-report-2026-06.xlsx
- accounts-receivable-2026-06.xlsx
- kpi-definitions.json

### Safety + quality
- safety-walk-template.md
- incident-report-template.md
- safety-incident-log-2026.xlsx
- qaqc-punchlist-template.xlsx

### Templates
- subcontractor-evaluation-template.md
- owner-change-request-form.json
- rfi-template.md

### Word documents (.docx)

- enterprise-project-summary-2026-02.docx

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
- people: 20
- site: 30
- diagrams: 18
- social: 10

### Word documents (.docx)

- enterprise-project-summary-2026-02.docx

## Timeline of significant events
- 2024-10: ERP/WIP reporting overhaul (definition drift begins).
- 2025-05: New safety director (more standardized safety walks).
- 2026-04: Riverwalk project hits long-lead window issue.
- 2026-06: Schedule re-baseline.
- 2026-07: Exec pushes for accurate CO totals.

## Benchmark goals (IKAM)
- Queries:
  1) “What is the latest substantial completion date and why does it differ?”
  2) “What are approved vs pending change orders and true totals?”
  3) “Reconcile PM forecast margin vs accounting WIP.”
  4) “Top open RFIs/submittals and owners.”
