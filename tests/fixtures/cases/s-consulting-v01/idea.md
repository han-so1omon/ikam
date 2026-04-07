# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: s-consulting-v01
- Business name: Birchline Insights
- Industry / domain: Consulting (ops + analytics boutique)
- Business model: project-based advisory + light implementation
- Locations / operating region: US (remote)
- Size tier: s
- Org maturity: small team; docs split across shared drive + email exports

## High-level description
- One-liner: A tiny consulting firm delivering a few client projects, with just enough scope/definition drift to break naive artifact parsers.

## Operating model
- Functions:
  - Delivery
  - Sales
  - Finance (fractional)
- Key roles:
  - Avery Hart — Principal
  - Jordan Ng — Consultant
  - Casey Moore — Analyst
  - Priya Desai — Fractional Finance

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **3**
- Rationale: low volume but high ambiguity; artifacts inconsistent.
- Where chaos lives:
  - [x] statement of work vs change requests drift
  - [x] time tracking vs invoice summaries mismatch
  - [x] deliverable version sprawl (v1/v2 “final”)

## Intentional contradictions
1) Scope:
   - SOW says 6-week discovery only
   - Change request implies implementation included
2) Hours:
   - Timesheet totals 148 hours
   - Invoice summary bills 132 hours (write-offs)
3) Deliverable status:
   - Deck labeled FINAL
   - Email thread calls it “draft pending client data”

## Artifact set

### Firm identity
- mission-vision-values.md
- brand-guide.md
- services-one-pager.pdf
- org.json

### Client project (primary)
Client: Redwood Freight Co.
Project: Network optimization discovery
- sow-redwood-freight.pdf
- change-request-001.md
- project-plan.md
- weekly-status-2026-01-12.md
- weekly-status-2026-01-19.md
- weekly-status-2026-01-26.md
- client-email-thread-2026-01-27.md
- deliverable-deck-final.pptx
- deliverable-deck-v2.pptx

### Finance + delivery ops
- timesheet-2026-01.xlsx
- invoice-summary-2026-01.pdf
- pipeline.xlsx
- revenue-history-2025.xlsx
- kpi-definitions.json

### Templates
- sow-template.md
- invoice-template.md
- meeting-notes-template.md

### Word documents (.docx)

- engagement-scoping-memo-2026-01-06.docx
- hours-reconciliation-2026-02-03.docx

### Images (mixed; generated)
Documentation:
- assets/images/README.md

Folders:
- assets/images/logos/
- assets/images/people/
- assets/images/office/
- assets/images/diagrams/
- assets/images/social/

Metadata:
- assets/images/prompts.jsonl

Targets (exact):
- logos: 4
- people: 10
- office: 8
- diagrams: 8
- social: 4

### Word documents (.docx)

- engagement-scoping-memo-2026-01-06.docx
- hours-reconciliation-2026-02-03.docx

## Timeline of significant events
- 2025-09: Firm founded.
- 2026-01: Redwood Freight discovery kicks off.
- 2026-01-27: Client requests implementation add-on.

## Benchmark goals (IKAM)
- Queries:
  1) “What is the agreed scope and is implementation included?”
  2) “Why do billed hours differ from timesheets?”
  3) “Which deliverable version is actually final?”
