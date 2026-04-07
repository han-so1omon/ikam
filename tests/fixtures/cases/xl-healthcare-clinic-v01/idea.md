# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: xl-healthcare-clinic-v01
- Business name: Harborview Family Clinics Network (HFCN)
- Industry / domain: Healthcare clinic network (primary care + urgent care + a few specialty services)
- Business model: Insurance billing + membership pilot + occupational health contracts
- Locations / operating region: Greater Phoenix, AZ (9 clinics) + central billing office
- Size tier: xl
- Org maturity: heavy policy surface + uneven on-the-ground adherence

## High-level description
- One-liner: A regional clinic network with lots of policies, templates, and compliance requirements — and lots of day-to-day operational drift.
- Services:
  - Primary care
  - Urgent care
  - Occupational health
  - Basic labs / referrals

## Operating model
- Functions:
  - Clinic operations (site directors)
  - Medical staff
  - Revenue cycle / billing
  - Compliance / privacy
  - HR
  - Marketing
  - Finance
- Key roles (named people + titles):
  - Dr. Renee Lawson — Chief Medical Officer
  - Marco Diaz — COO
  - Tara Nguyen — Director of Revenue Cycle
  - Omar Shah — Compliance Officer
  - Jenna Cole — HR Director
  - Vinh Tran — Finance Lead

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **4**
- Rationale: many sites + billing complexity + policy drift + multiple systems.
- Where chaos lives:
  - [x] conflicting metrics (no-show rate, collections, patient volume)
  - [x] version sprawl (policies, intake forms, training decks)
  - [x] shadow spreadsheets (staffing, denials, no-show tracking)
  - [x] messy meeting notes (site director calls)
  - [x] missing docs (postmortems; incomplete audit trails)
  - [x] naming drift (program names, clinic nicknames)

## Intentional contradictions
1) No-show rate:
   - Ops dashboard says 7.5%
   - Site director report says 10–12% (definition differs; includes late cancels)
2) Collections:
   - Finance summary reports 94% collections
   - Revenue cycle tracker shows 89% (excludes one-off catch-up payments)
3) Privacy training:
   - Policy says annual training required by Jan 31
   - HR tracker shows many staff incomplete; compliance memo claims “on track”
4) Membership pilot naming:
   - “Harborview+” vs “ClinicPlus” vs “DirectCare Pilot”

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md
- org-structure.json

### Clinical ops
- clinic-ops-handbook-v2.md
- clinic-ops-handbook-v3.md (draft)
- staffing-model.xlsx
- site-director-scorecard-2026-01.xlsx
- incident-log-2026-q1.xlsx

### Revenue cycle / billing
- kpi-definitions.json
- denials-tracker-2026-01.xlsx
- billing-process-map.md
- payer-mix-2025.xlsx

### Compliance / privacy
- privacy-policy-v3.md
- privacy-policy-v4.md (updated but not fully rolled out)
- audit-memo-2026-02.pdf
- hipaa-training-tracker-2026.xlsx

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- collections-summary-2026-01.xlsx
- board-update-2026-02.pdf

### HR
- hiring-plan-2026.xlsx
- performance-review-template.md
- employee-handbook-excerpt.pdf

### Marketing
- marketing-pitch-deck.pptx
- service-line-one-pager.pdf
- messaging-pillars.md

### Meetings + notes
- site-director-call-2026-01-11.md
- site-director-call-2026-02-08.md
- voice-note-transcript-2026-02-14.md

### Evaluation templates
- patient-complaint-template.md
- incident-report-template.md
- new-hire-onboarding-checklist.md
- patient-intake-form.json

### Word documents (.docx)

- network-report-2026-01.docx

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
- logos: 10
- people: 30
- clinics: 15
- diagrams: 20
- social: 10

### Word documents (.docx)

- network-report-2026-01.docx

## Timeline of significant events
- 2024-02: Acquired 2 urgent care sites (integration pain).
- 2024-09: Revenue cycle system change; denials spike.
- 2025-04: Membership pilot launched (name varies).
- 2025-11: Privacy audit findings; policy v4 drafted.
- 2026-01: Ops handbook v3 draft circulated; sites still use v2.
- 2026-02: Board update uses optimistic collections definition.

## Benchmark goals (IKAM)
- Queries:
  1) “What is the real no-show rate and why does it differ across reports?”
  2) “What privacy policy version is in effect vs in use?”
  3) “How do collections metrics differ and what’s the correct value?”
  4) “What is the membership pilot called and how is it performing?”
