# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: xl-software-v01
- Business name: Meridian SignalWorks
- Industry / domain: Software (B2B data + workflow platform)
- Business model: SaaS subscription + usage-based add-ons
- Locations / operating region: Distributed (US/EU) — remote-first
- Size tier: xl
- Org maturity: bureaucratic surface area + startup habits underneath

## High-level description
- One-liner: A B2B platform that ingests operational signals (events, logs, documents), normalizes them, and powers workflow automation + reporting for regulated-ish mid-market customers.
- Primary customers:
  - Fintech ops teams
  - Healthcare services (non-clinical ops)
  - Logistics & marketplaces

## Operating model
- Functions:
  - Product
  - Engineering (platform + apps)
  - Data/ML
  - Sales
  - Customer Success
  - Security/Compliance
  - Finance
- Key roles (named people + titles):
  - Harper Lin — CEO
  - Theo Ramirez — CTO
  - Mina Okafor — VP Product
  - Jules Chen — Director, Platform Eng
  - Sana Iqbal — Head of Security
  - Owen Patel — Head of CS
  - Riley Morgan — Finance Lead

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **5**
- Rationale: enterprise-scale doc sprawl + multiple sources of truth + fast changes.
- Where chaos lives:
  - [x] naming conventions (feature names vs codenames vs SKUs)
  - [x] versioning / duplicates (PRDs, decks, policies)
  - [x] conflicting metrics (ARR, NRR, MAU definitions)
  - [x] stale strategy/roadmap (roadmap slides lag Jira)
  - [x] missing documents (postmortems, decisions)
  - [x] shadow systems (side spreadsheets, personal notes)
  - [x] messy meeting notes (mixed quality)
  - [x] unclear ownership/RACI (matrix org, frequent reorg)

## Intentional contradictions to include
1) ARR discrepancy:
   - Board deck says $24.8M ARR
   - Finance model says $23.9M ARR (excludes churned annual prepaid)
2) Roadmap conflict:
   - Public roadmap says Feature “Orchid” ships Q2
   - Internal Jira epic slipped to Q3
3) Incident narrative conflict:
   - Postmortem blames upstream vendor latency
   - Metrics dashboard shows internal queue backlog as primary driver
4) Security policy drift:
   - Policy doc says 90-day key rotation
   - Engineering runbook uses 180-day cadence

## Artifact set (what documents should exist)
Mix of Markdown, JSON, XLSX, PPTX, and PDF.

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md
- org-chart.json

### Product + roadmap
- product-overview.md
- roadmap-2026-h1.pptx
- prd-orchid-v1.md
- prd-orchid-v3-FINAL.md
- release-notes-2025-q4.md

### Engineering + delivery
- sprint-log-2025-q4.xlsx
- kanban-snapshot-2026-01.json
- architecture-overview.md
- runbook-queue-backlog.md
- incident-2026-01-17-postmortem.md
- incident-2026-01-17-metrics.json

### Sales + CS
- sales-deck-v2.pptx
- top-customers.json
- pipeline-2026-q1.xlsx
- customer-renewal-risk.xlsx
- cs-qbr-template.pptx

### Finance
- quarterly-revenue-history-2024-2025.xlsx
- arr-model-2026.xlsx
- board-update-2026-02.pdf
- kpi-definitions.json

### Security/Compliance
- security-policy-access-control.md
- key-rotation-runbook.md
- vendor-risk-register.xlsx

### Meetings + notes
- exec-notes-2026-01-05.md
- product-review-notes-2026-01-19.md
- voice-note-transcript-2026-02-02.md

### Evaluation templates
- incident-postmortem-template.md
- prd-template.md
- customer-intake-form.json

### Word documents (.docx)

- arr-reconciliation-memo-2026-02.docx
- product-milestone-update-2026-02.docx

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

Targets (exact for this case):
- logos: 8
- people: 30
- product-ui: 30 (synthetic UI screenshots/illustrations)
- diagrams: 25 (architecture, dashboards, incident timelines)
- social: 12

## Entity glossary (KG grounding)
- People: Harper Lin, Theo Ramirez, Mina Okafor, Jules Chen, Sana Iqbal, Owen Patel, Riley Morgan
- Teams: Platform Eng, App Eng, Security, Product, CS, Sales
- Products/Modules: Meridian Core, Meridian Automations, Meridian Insights
- Features: Orchid (codename), QueueGuard (internal), Atlas Metrics (dashboard)
- Customers: 10–15 named accounts with aliases
- KPIs: ARR, NRR, MAU, DAU, churn, incident MTTR

### Word documents (.docx)

- arr-reconciliation-memo-2026-02.docx
- product-milestone-update-2026-02.docx

## Timeline of significant events
- 2024-06: Reorg: Platform split into Platform + Data teams.
- 2024-11: Security audit finding: access review cadence unclear.
- 2025-03: Pricing change: usage-based add-on introduced.
- 2025-08: Major customer churn event (NRR narrative shifts).
- 2025-12: Feature Orchid scope expanded.
- 2026-01-17: Queue backlog incident; competing narratives.
- 2026-02-01: Board update prepared; ARR discrepancy appears.

## Benchmark goals (IKAM)
- Queries:
  1) “What is the real ARR and why do numbers differ across docs?”
  2) “What is Feature Orchid status and shipping timeline?”
  3) “What caused the 2026-01-17 incident?”
  4) “What is the key rotation policy vs what is practiced?”
- Ambiguities to resolve/flag:
  - PRD version conflicts
  - KPI definition mismatch
  - policy vs runbook drift
