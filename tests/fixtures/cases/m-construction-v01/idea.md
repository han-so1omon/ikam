# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: m-construction-v01
- Business name: Beaconline Construction Co.
- Industry / domain: Construction (commercial TI + light industrial + multifamily common areas)
- Business model: Project-based (GMP + fixed bid + T&M for service)
- Locations / operating region: Seattle, WA metro
- Size tier: m
- Org maturity: mixed (some process, lots of exceptions)

## High-level description
- One-liner: A mid-sized GC juggling many subcontractors and a busy backlog; documentation exists but drifts across tools.
- What they deliver:
  - Tenant improvements (office/retail)
  - Light industrial buildouts
  - Multifamily common area refreshes

## Operating model
- Functions:
  - Estimating
  - Precon
  - PMs
  - Superintendents
  - Safety
  - Accounting/AP
- Key roles (named people + titles):
  - Kim Alvarez — Owner / President
  - Nate Sorensen — Director of Precon
  - Sara Ito — Senior PM
  - Devon Price — Superintendent
  - Leah Moore — Safety Manager
  - Priyanka Nair — Controller

## Chaos profile (pick + tracked)
- Overall chaos level (1–5): **3**
- Rationale: more moving parts than the small GC; process exists but isn’t consistently followed.
- Where chaos lives:
  - [x] versioning / duplicates (schedule v2 vs v4, “final” pay app)
  - [x] conflicting metrics (project cost report vs accounting WIP)
  - [x] shadow spreadsheets (submittal and RFI tracking)
  - [x] messy meeting notes (some structured, some not)
  - [ ] missing docs (some gaps)

## Intentional contradictions
Deliberate contradictions: **yes** (moderate)
1) Schedule milestone date:
   - Weekly meeting notes say Substantial Completion is 2026-03-28
   - Schedule XLSX shows 2026-04-05 (re-baselined)
2) Change order totals:
   - CO log sums to $118,400
   - Owner’s summary email claims $124,900 (includes pending CO)
3) Cost-to-complete:
   - PM cost report shows 6% fee margin
   - Accounting WIP implies margin compression to 3–4%

## Artifact set (what documents should exist)

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md

### Estimating + contracts
- company-capabilities-one-pager.pdf
- estimate-template.xlsx
- sample-estimate-retail-ti-pine-st.pdf
- contract-summary-retail-ti-pine-st.pdf

### Project delivery (primary project)
Project: Retail TI — Pine St
- project-plan-retail-ti-pine-st.md
- schedule-retail-ti-pine-st.xlsx
- submittal-log-retail-ti-pine-st.xlsx
- rfi-log-retail-ti-pine-st.xlsx
- change-order-log-retail-ti-pine-st.xlsx
- meeting-notes-retail-ti-pine-st-2026-01-09.md
- meeting-notes-retail-ti-pine-st-2026-02-06.md
- meeting-notes-retail-ti-pine-st-2026-03-06.md
- owner-summary-email-2026-03-07.md

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- job-costing-retail-ti-pine-st.xlsx
- wip-report-2026-02.xlsx
- accounts-receivable-2026-02.xlsx
- kpi-definitions.json

### Safety + evaluation templates
- subcontractor-evaluation-template.md
- safety-walk-template.md
- incident-report-template.md
- client-intake-form.json

### Word documents (.docx)

- project-status-memo-2026-02-17.docx

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
- logos: 5
- people: 14
- site: 20
- diagrams: 12
- social: 8

### Word documents (.docx)

- project-status-memo-2026-02-17.docx

## Timeline of significant events
- 2024-10: Controller hire (Priyanka) tightens WIP process.
- 2025-05: Safety manager hired; safety walks standardized.
- 2026-01: Retail TI — Pine St kicks off.
- 2026-03: Re-baseline schedule after long lead storefront glass.

## Benchmark goals (IKAM)
- Queries:
  1) “What is the latest Substantial Completion date and why does it differ?”
  2) “What change orders are approved vs pending, and what is the true total?”
  3) “What does margin look like per PM vs accounting WIP?”
  4) “What are the top open RFIs/submittals and owners?”
