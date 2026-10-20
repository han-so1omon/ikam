# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: m-software-v01
- Business name: LatticeOps
- Industry / domain: Software (B2B ops workflow + light analytics)
- Business model: SaaS subscription (mid-market)
- Locations / operating region: US (hybrid; one HQ)
- Size tier: m
- Org maturity: emerging product org; decent hygiene but still messy around metrics

## High-level description
- One-liner: A mid-sized SaaS company shipping regularly, with some roadmap drift and metric definition inconsistencies.

## Operating model
- Functions:
  - Product
  - Engineering
  - Sales
  - Customer Success
  - Finance
  - Security (small)
- Key roles:
  - Jamie Ross — CEO
  - Priya Mehta — CTO
  - Alex Chen — Head of Product
  - Sam Rivera — Eng Manager
  - Taylor Kim — Head of CS
  - Morgan Shah — Finance Lead
  - Rina Patel — Security (part-time)

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **3**
- Rationale: fewer artifacts than l/xl; still multiple sources for KPIs and roadmaps.
- Where chaos lives:
  - [x] KPI definition mismatch (active users vs active accounts)
  - [x] roadmap vs sprint reality drift
  - [x] inconsistent incident follow-through

## Intentional contradictions
1) Ship date:
   - Roadmap deck: Feature “Garnet” ships in April
   - Sprint log shows it pushed to May
2) KPI:
   - CS uses “active accounts”
   - Product uses “weekly active users”
3) Incident:
   - Postmortem narrative blames vendor
   - Metrics show internal backlog spike

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md
- org-chart.json

### Product
- product-overview.md
- roadmap-2026-h1.pptx
- prd-garnet-v1.md
- prd-garnet-v3-FINAL.md
- release-notes-2025-q4.md

### Engineering
- sprint-log-2026-q1.xlsx
- kanban-snapshot-2026-02.json
- runbook-backlog.md
- incident-2026-02-09-postmortem.md
- incident-2026-02-09-metrics.json

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
- exec-notes-2026-02-03.md
- product-review-notes-2026-02-17.md
- voice-note-transcript-2026-02-21.md

### Evaluation templates
- incident-postmortem-template.md
- prd-template.md
- customer-intake-form.json

### Word documents (.docx)

- feature-garnet-status-2026-02-14.docx

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
- logos: 6
- people: 18
- product-ui: 16
- diagrams: 12
- social: 6

### Word documents (.docx)

- feature-garnet-status-2026-02-14.docx

## Timeline of significant events
- 2025-11: Introduced usage-based add-on.
- 2026-02-09: Backlog spike incident.
- 2026-02-21: KPI definition debate escalates.

## Benchmark goals (IKAM)
- Queries:
  1) “What is Feature Garnet status and ship date?”
  2) “What caused the Feb 9 incident?”
  3) “What are the KPI definitions and how do they differ by team?”
