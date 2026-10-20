# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: s-healthcare-clinic-v01
- Business name: Pinecrest Family Clinic
- Industry / domain: Healthcare clinic (primary care)
- Business model: Insurance billing + self-pay
- Locations / operating region: Sacramento, CA (1 clinic)
- Size tier: s
- Org maturity: small, consistent, low process overhead

## High-level description
- One-liner: A single-location primary care clinic with simple operations and relatively consistent documentation.

## Operating model
- Functions:
  - Front desk
  - Clinical staff
  - Billing (part-time / outsourced)
- Key roles:
  - Dr. Elena Vargas — Owner/Physician
  - Jordan Kim — Office Manager
  - Maya Singh — Medical Assistant
  - “ClearPath Billing” — billing service

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **1**
- Rationale: small clinic, few systems, most docs consistent.
- Where chaos lives (minor):
  - [x] appointment reminder tracking is informal
  - [ ] metric definition drift (minimal)
  - [ ] version sprawl (minimal)

## Intentional contradictions
- Deliberate contradictions: **no**
- Allowed minor friction:
  - One month of collections lags due to insurer processing delay.

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md

### Ops
- clinic-ops-handbook.md
- staffing-schedule-template.xlsx
- incident-log-2026-q1.xlsx

### Billing
- billing-process-map.md
- collections-summary-2026-01.xlsx
- denials-tracker-2026-01.xlsx
- kpi-definitions.json

### Compliance/HR
- privacy-policy.md
- hipaa-training-tracker-2026.xlsx
- employee-handbook-excerpt.pdf
- performance-review-template.md

### Marketing
- marketing-pitch-deck.pptx
- service-line-one-pager.pdf
- messaging-pillars.md

### Meetings + notes
- staff-huddle-notes-2026-01-09.md
- staff-huddle-notes-2026-02-06.md

### Evaluation templates
- patient-complaint-template.md
- incident-report-template.md
- patient-intake-form.json

### Word documents (.docx)

- ops-summary-2026-01.docx

### Images (mixed; generated)
Documentation:
- assets/images/README.md

Folders:
- assets/images/logos/
- assets/images/people/
- assets/images/clinic/
- assets/images/diagrams/
- assets/images/social/

Metadata:
- assets/images/prompts.jsonl

Targets (exact):
- logos: 4
- people: 8
- clinic: 6
- diagrams: 6
- social: 4

### Word documents (.docx)

- ops-summary-2026-01.docx

## Timeline of significant events
- 2024-09: Office manager hired (documentation improves).
- 2025-12: Reminder workflow switched to a simpler SMS tool.
- 2026-01: Billing vendor changes denial categorization.

## Benchmark goals (IKAM)
- Queries:
  1) “What are the top denial reasons in Jan 2026?”
  2) “What is training completion status?”
  3) “What is collections performance and what caused delays?”
