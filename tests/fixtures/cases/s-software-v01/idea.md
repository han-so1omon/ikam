# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: s-software-v01
- Business name: PocketRelay
- Industry / domain: Software (B2B micro-SaaS for intake + routing)
- Business model: SaaS subscription (SMB)
- Locations / operating region: US-remote
- Size tier: s
- Org maturity: scrappy; simple systems; docs live in a few places

## High-level description
- One-liner: A tiny SaaS with a couple of customers, a handful of metrics, and just enough documentation drift to confuse “truth.”

## Operating model
- Functions:
  - Product/Engineering (same people)
  - Sales
  - Support
  - Finance (part-time)
- Key roles:
  - Riley Novak — Founder/CEO
  - Chen Wu — Founding Engineer
  - Maya Torres — Customer Success/Support
  - Dev Patel — Fractional Finance

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **2**
- Rationale: fewer artifacts, but definitions drift because everything is informal.
- Where chaos lives:
  - [x] metric definition mismatch ("active" and "churn")
  - [x] pricing page vs contract terms drift
  - [x] incident writeups incomplete

## Intentional contradictions
1) Churn:
   - Support calls churn “cancellations”
   - Finance sheet uses churn as net revenue churn (includes downgrades)
2) Pricing:
   - Pricing page says $49/$99
   - A customer contract addendum references legacy $79 tier

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- one-page-strategy-2026.md
- org.json

### Product + docs
- product-overview.md
- mini-roadmap-2026.md
- prd-smart-routing-v1.md
- prd-smart-routing-v2.md
- release-notes-2025.md

### Customer + sales
- pricing-page-copy.md
- msa-sample.pdf
- contract-addendum-legacy-pricing.pdf
- top-customers.json
- support-macros.md

### Engineering
- runbook-oncall.md
- incident-2026-01-17.md
- uptime-kpis.json

### Finance
- revenue-history-2025.xlsx
- churn-and-mrr-2026.xlsx
- kpi-definitions.json

### Meetings + notes
- founder-notes-2026-02-02.md
- customer-call-notes-2026-02-14.md

### Templates
- incident-template.md
- customer-intake-form.json

### Word documents (.docx)

- customer-onboarding-guide-2026.docx
- monthly-metrics-note-2026-01.docx

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
- logos: 4
- people: 10
- product-ui: 10
- diagrams: 8
- social: 4

### Word documents (.docx)

- customer-onboarding-guide-2026.docx
- monthly-metrics-note-2026-01.docx

## Timeline of significant events
- 2025-06: Launched v1.
- 2025-10: Added Smart Routing (early).
- 2026-01: Incident during webhook retries.
- 2026-02: Pricing cleanup planned.

## Benchmark goals (IKAM)
- Queries:
  1) “What is churn and how is it defined?”
  2) “What is the current pricing and why do docs disagree?”
  3) “What happened in the Jan 17 incident and what did we change?”
