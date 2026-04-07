# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: l-software-v01
- Business name: QuillStack Systems
- Industry / domain: Software (B2B workflow + reporting)
- Business model: SaaS subscription
- Locations / operating region: Remote-first (US)
- Size tier: l
- Org maturity: process-emerging (real product org, not enterprise)

## High-level description
- One-liner: A mature-ish SaaS company with a platform product, regular releases, and occasional incident-driven chaos.

## Operating model
- Functions:
  - Product
  - Engineering
  - Customer Success
  - Sales
  - Security
  - Finance
- Key roles:
  - Paige Turner — CEO
  - Arman Liu — CTO
  - Chloe Reyes — Head of Product
  - Devin Shaw — Eng Manager
  - Nia Brooks — Security Lead
  - Morgan Patel — Finance Lead

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **4**
- Rationale: lots of product artifacts + Jira + docs; version drift and KPI definition mismatches.
- Where chaos lives:
  - [x] PRD version sprawl
  - [x] roadmap vs delivery drift
  - [x] KPI definition mismatch (MAU/active accounts)
  - [x] incident docs inconsistent

## Intentional contradictions
1) Roadmap:
   - Roadmap deck says Feature “Fable” ships in May
   - Sprint log shows it pushed to June
2) KPI:
   - CS dashboard uses “active accounts”
   - Finance uses “billable accounts”
3) Incident:
   - Postmortem says third-party API outage
   - Metrics show internal queue saturation

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md
- org-chart.json

### Product
- product-overview.md
- roadmap-2026-h1.pptx
- prd-fable-v1.md
- prd-fable-v4-FINAL.md
- release-notes-2025-q4.md

### Engineering
- sprint-log-2026-q1.xlsx
- kanban-snapshot-2026-02.json
- runbook-queue-saturation.md
- incident-2026-02-11-postmortem.md
- incident-2026-02-11-metrics.json

### Sales + CS
- sales-deck.pptx
- cs-qbr-template.pptx
- top-customers.json
- pipeline-2026-q2.xlsx
- renewal-risk.xlsx

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- arr-model-2026.xlsx
- kpi-definitions.json

### Meetings + notes
- exec-notes-2026-02-01.md
- product-review-notes-2026-02-18.md
- voice-note-transcript-2026-02-20.md

### Evaluation templates
- incident-postmortem-template.md
- prd-template.md
- customer-intake-form.json

### Word documents (.docx)

- feature-fable-update-2026-02-18.docx

### Images (mixed; generated)
Documentation:
- assets/images/README.md

Folders:
- assets/images/logos/
- assets/images/people/
- assets/images/product-ui/
- assets/images/diagrams/
- assets/images/social/

Metadata:
- assets/images/prompts.jsonl

Targets (exact):
- logos: 7
- people: 22
- product-ui: 20
- diagrams: 16
- social: 8

### Word documents (.docx)

- feature-fable-update-2026-02-18.docx

## Timeline of significant events
- 2025-10: Reorg: platform team split.
- 2025-12: Feature Fable scope expands.
- 2026-02-11: Queue saturation incident.
- 2026-02-20: Finance debates active vs billable accounts definition.

## Benchmark goals (IKAM)
- Queries:
  1) “What is Feature Fable status and ship date?”
  2) “What caused the 2026-02-11 incident?”
  3) “What is the definition of active vs billable accounts?”
