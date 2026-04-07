# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: l-healthcare-clinic-v01
- Business name: Harborview Care Partners (HCP)
- Industry / domain: Healthcare clinic (multi-site primary care + urgent care)
- Business model: outpatient care; mix of insurance + self-pay; limited employer contracts
- Locations / operating region: Northern California (6 clinics)
- Size tier: l
- Org maturity: established operations; policy/procedure drift across sites

## High-level description
- One-liner: A growing clinic network with operational and compliance artifacts spread across HR, clinical operations, and billing—definitions drift and different “latest” docs circulate.

## Operating model
- Functions:
  - Clinical operations
  - Nursing
  - Front desk / scheduling
  - Billing / revenue cycle
  - Compliance / privacy
  - HR / recruiting
  - Finance
- Key roles:
  - Dr. Lila Moreno — Medical Director
  - Ethan Park — COO
  - Naomi Chen — Director of Nursing
  - Sofia Alvarez — Revenue Cycle Manager
  - Jordan Blake — Compliance Officer
  - Priya Desai — HR Manager

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **4**
- Rationale: more sites → more SOP versions; billing rules and clinical policies drift.
- Where chaos lives:
  - [x] policy version confusion (handbooks + clinical SOPs)
  - [x] inconsistent KPI definitions (no-show rate, wait time)
  - [x] incident reporting variability (near-miss vs reportable)
  - [x] scheduling templates maintained locally

## Intentional contradictions
1) No-show rate:
   - Operations dashboard excludes late-cancels
   - Revenue cycle report includes late-cancels as no-shows
2) HIPAA training compliance:
   - HR spreadsheet shows 98% completion
   - Compliance audit memo cites 91% (different roster)
3) Patient wait time:
   - Clinic A reports “door-to-doc”
   - Exec scorecard uses “arrival-to-departure”

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md
- org-structure.json

### Clinical ops
- clinical-sop-triage-v2.md
- clinical-sop-triage-v3-DRAFT.md
- urgent-care-protocols.pdf
- clinic-site-operations-playbook.md

### Scheduling + front desk
- scheduling-template-clinic-a.xlsx
- scheduling-template-clinic-d.xlsx
- call-center-script.md
- patient-intake-form.json

### Compliance + HR
- hipaa-training-tracker-2026.xlsx
- compliance-audit-memo-2026-01.md
- employee-handbook-v4.md
- employee-handbook-v5.md

### Revenue cycle
- billing-kpi-definitions.json
- denial-log-2025-q4.xlsx
- no-show-report-2026-01.xlsx
- payer-mix-2025.xlsx

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- clinic-pnl-2025.xlsx
- exec-scorecard-2026-01.pdf

### Meetings + notes
- ops-review-notes-2026-01-22.md
- nursing-huddle-notes-2026-02-04.md
- voice-note-transcript-2026-02-10.md

### Templates
- incident-report-template.md
- patient-complaint-template.md
- provider-onboarding-checklist.md

### Word documents (.docx)

- network-ops-summary-2026-01.docx

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
- logos: 8
- people: 22
- clinic: 22
- diagrams: 16
- social: 8

### Word documents (.docx)

- network-ops-summary-2026-01.docx

## Timeline of significant events
- 2024-08: Added two new clinic sites.
- 2025-03: Updated triage SOP (v2).
- 2025-11: Billing denial rates rose with payer policy change.
- 2026-01: Compliance audit highlights training roster mismatch.
- 2026-02: Exec requests unified KPI definitions.

## Benchmark goals (IKAM)
- Queries:
  1) “What is the no-show rate and why do reports differ?”
  2) “Is HIPAA training compliant and what’s the true completion %?”
  3) “Which triage SOP version is current?”
  4) “What’s the patient wait time definition on the exec scorecard?”
