# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: m-healthcare-clinic-v01
- Business name: Mesa Ridge Urgent & Primary Care
- Industry / domain: Healthcare clinic group (urgent care + primary care)
- Business model: Insurance billing + self-pay + small employer contracts
- Locations / operating region: Denver, CO (3 clinics)
- Size tier: m
- Org maturity: emerging; some policy docs, lots of local variation

## High-level description
- One-liner: Three clinics with shared billing and uneven operational consistency.

## Operating model
- Functions:
  - Clinic ops
  - Billing
  - Compliance/privacy (light)
  - HR
  - Finance
- Key roles:
  - Dr. Sofia Kim — Medical Director
  - Ryan Patel — Ops Manager
  - Elise Grant — Billing Lead
  - Tasha Nguyen — HR/Office Manager

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **2**
- Rationale: fewer sites than XL network; drift exists but bounded.
- Where chaos lives:
  - [x] no-show definition drift
  - [x] training tracking inconsistencies
  - [ ] massive version sprawl (limited)
  - [x] shadow spreadsheets (staffing)

## Intentional contradictions
- Light/moderate:
  - No-show: ops excludes late cancels; site managers include.
  - Collections: billing tracker differs from finance summary by a few points.

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md

### Ops
- clinic-ops-handbook-v1.md
- staffing-model.xlsx
- site-scorecard-2026-01.xlsx
- incident-log-2026-q1.xlsx

### Billing
- billing-process-map.md
- denials-tracker-2026-01.xlsx
- collections-summary-2026-01.xlsx
- kpi-definitions.json

### Compliance/HR
- privacy-policy.md
- hipaa-training-tracker-2026.xlsx
- hiring-plan-2026.xlsx
- employee-handbook-excerpt.pdf
- performance-review-template.md

### Marketing
- marketing-pitch-deck.pptx
- service-line-one-pager.pdf
- messaging-pillars.md

### Meetings + notes
- ops-call-2026-01-15.md
- ops-call-2026-02-12.md
- voice-note-transcript-2026-02-16.md

### Evaluation templates
- patient-complaint-template.md
- incident-report-template.md
- patient-intake-form.json

### Word documents (.docx)

- ops-report-2026-01.docx

### Images (mixed; generated)
Documentation:
- assets/images/README.md

Folders:
- assets/images/logos/
- assets/images/people/
- assets/images/clinics/
- assets/images/diagrams/
- assets/images/social/

Metadata:
- assets/images/prompts.jsonl

Targets (exact):
- logos: 6
- people: 18
- clinics: 8
- diagrams: 10
- social: 6

### Word documents (.docx)

- ops-report-2026-01.docx

## Timeline of significant events
- 2024-05: Third clinic opened.
- 2025-09: Billing workflow change; denials briefly spike.
- 2026-01: Ops updates no-show process; sites interpret differently.

## Benchmark goals (IKAM)
- Queries:
  1) “What is the no-show rate and why does it differ?”
  2) “What are the top denial reasons and owners?”
  3) “What is privacy training completion and who is missing?”
