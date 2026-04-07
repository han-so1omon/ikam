# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: m-consulting-v01
- Business name: Northlake Advisory Group (NAG)
- Industry / domain: Consulting (operations + finance transformation for mid-market firms)
- Business model: Services (retainers + fixed-scope projects)
- Locations / operating region: Seattle, WA + remote (US)
- Size tier: m
- Org maturity: process-emerging (some templates, uneven adoption)

## High-level description
- One-liner: A mid-sized consulting firm helping mid-market companies fix operational bottlenecks, reporting, and cost structure.
- What they deliver:
  - Diagnostic engagements (2–4 weeks)
  - 90-day transformation sprints
  - Ongoing advisory retainers
- Target customers:
  - Manufacturing, logistics, healthcare services (mid-market)
- Differentiators:
  - Practical, spreadsheet-driven, “get it done” approach
  - Strong operator bench (ex-industry)

## Operating model
- Teams / functions:
  - Partners
  - Delivery (consultants)
  - Analytics (light)
  - Sales/BD
  - Ops/Admin
- Key roles (named people + titles):
  - Elena Park — Managing Partner
  - Ravi Desai — Partner, Growth
  - Casey Nguyen — Engagement Manager
  - Morgan Lee — Senior Consultant
  - Priya Shah — Analyst
  - Jo Alvarez — Ops Manager
- Tools (realism anchors):
  - Docs: Google Drive + exported PDFs
  - Spreadsheets: Excel
  - CRM-ish: HubSpot (partial)
  - Project tracking: mixed (some Jira, some Monday, lots of ad-hoc)
  - Meetings: Zoom + notes in docs

## Chaos profile (different from s-local-retail-v01)
- Overall chaos level (1–5): **4**
- Why: many parallel clients + reuse of templates + time pressure → version sprawl.
- Where chaos lives:
  - [x] naming conventions (clients have codenames + legal names)
  - [x] versioning / duplicates (decks reused, “copy of copy”)
  - [x] conflicting metrics (client-provided numbers vs consultant-derived)
  - [x] stale strategy/roadmap (internal GTM doc trails reality)
  - [x] missing documents (some engagements missing closeout)
  - [x] shadow spreadsheets (personal files outside shared drive)
  - [x] messy meeting notes (action items lost, owners unclear)
  - [x] unclear ownership/RACI (partners override process)

## Intentional contradictions to include
1) Revenue recognition vs pipeline:
   - Finance summary recognizes a project in Q3
   - Pipeline report still lists it as “At risk / not closed”
2) Client naming:
   - “Evergreen Transport” vs “Evergreen Logistics LLC” vs codename “PINECONE”
3) Delivery methodology:
   - Ops handbook says “two-week sprints”
   - Actual engagement artifacts show irregular cadence and ad-hoc milestones
4) Headcount/utilization:
   - Staffing plan counts 18 consultants
   - HR roster shows 16 FTE + 4 contractors (not all billable)

## Financial shape (benchmark intent)
- Currency: USD
- Revenue streams:
  - Fixed-scope project fees
  - Retainers
  - Training workshops
- Cost centers:
  - Labor
  - Travel (declining)
  - Subcontractors
  - Sales/marketing
- Reporting cadence:
  - Monthly internal
  - Quarterly board-style update

## Artifact set (what documents should exist)
Include a mix of Markdown, JSON, XLSX, PPTX, and PDF.

### Strategy + identity
- mission-vision-values.md
- brand-guide.md (palette + tone + slide style guidance)
- high-level-strategy-2026.md
- service-lines.md

### Marketing + sales
- marketing-pitch-deck.pptx
- case-study-one-pager.pdf
- proposal-template.docx (or md if docx not used)
- messaging-pillars.md
- lead-list.json

### Finance + pipeline
- projected-revenue-2026.xlsx
- quarterly-revenue-history-2024-2025.xlsx
- pipeline-report-2025-q3.xlsx
- utilization-tracker-2025-h2.xlsx
- kpi-definitions.json

### Delivery / project ops
- engagement-playbook.md
- sample-engagement-PINECONE/ (folder)
  - statement-of-work.pdf
  - kickoff-notes.md
  - weekly-status-2025-08-15.md
  - weekly-status-2025-08-29.md
  - deliverable-cost-model.xlsx
  - deliverable-deck-v1.pptx
  - deliverable-deck-v3-FINAL.pptx
  - closeout-summary.md (intentionally missing in some cases)

### Meetings + notes
- all-hands-notes-2025-09-03.md
- partner-meeting-notes-2025-10-10.md
- voice-note-transcript-2025-10-21.md

### Evaluation templates
- client-intake-form.json
- engagement-retro-template.md
- consultant-performance-rubric.md

### Word documents (.docx)

- engagement-status-2026-02-10.docx

### Images (mixed; generated)
Documentation:
- assets/images/README.md

Folders:
- assets/images/logos/
- assets/images/people/
- assets/images/social/
- assets/images/diagrams/

Metadata:
- assets/images/prompts.jsonl

Targets (exact for this case):
- logos: 6
- people: 18
- social: 24
- diagrams: 12 (consulting-style frameworks, charts, swimlanes; illustrative)

## Entity glossary (for knowledge graph grounding)
- People: Elena Park, Ravi Desai, Casey Nguyen, Morgan Lee, Priya Shah, Jo Alvarez
- Teams: Partners, Delivery, Analytics, Ops
- Clients (canonical + aliases):
  - Evergreen Transport / Evergreen Logistics LLC / PINECONE
  - 3–5 additional clients with aliases
- Services:
  - Ops diagnostic
  - Finance transformation
  - Cost model + pricing
- Projects/initiatives:
  - 2026 GTM refresh
  - Template cleanup (attempt)
- KPIs:
  - Booked revenue, recognized revenue
  - Pipeline by stage
  - Utilization %
  - Gross margin

### Word documents (.docx)

- engagement-status-2026-02-10.docx

## Timeline of significant events
- 2024-02: Elena becomes Managing Partner (promotion).
- 2024-09: Launch “90-day sprint” offer (marketing says standardized; delivery varies).
- 2025-03: Layoffs (2) + hiring freeze; later quietly adds contractors.
- 2025-07: Wins Evergreen (PINECONE) engagement; client naming sprawl begins.
- 2025-08: Internal template overhaul attempt; produces duplicates.
- 2025-10: Partner conflict on pricing; proposal template diverges.
- 2025-12: Big year-end push; utilization tracker becomes unreliable.
- 2026-01: GTM refresh draft circulated but not adopted.

## Benchmark goals (IKAM)
- Queries:
  1) “What is the true status of PINECONE: closed/won vs at-risk?”
  2) “Reconcile recognized revenue vs pipeline for Q3 2025.”
  3) “What delivery methodology is actually used?”
  4) “Who are the key people and what roles changed over time?”
- Ambiguities IKAM must resolve/flag:
  - multiple client names/codenames
  - deck version conflicts
  - missing closeout artifact
