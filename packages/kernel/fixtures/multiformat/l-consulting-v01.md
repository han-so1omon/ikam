# IKAM Benchmark Case Spec (idea.md)

## Identity
- Case ID: l-consulting-v01
- Business name: Greybridge Partners
- Industry / domain: Consulting (strategy + operations + turnaround)
- Business model: Services (retainers + success fees + fixed-scope diagnostics)
- Locations / operating region: Chicago, IL + travel (US)
- Size tier: l
- Org maturity: process-heavy externally, internally fragmented

## High-level description
- One-liner: A larger consulting firm with multiple practices and a lot of internal artifacts; client work is structured, internal ops are messy.
- Practices:
  - Turnaround / restructuring
  - Ops excellence
  - Pricing + cost transformation
  - PMO

## Operating model
- Functions:
  - Partners
  - Practice leads
  - Delivery teams
  - Research
  - Finance
  - Talent/HR
- Key roles:
  - Marisol Vega — Managing Partner
  - Grant Ellison — Partner, Turnaround
  - Ayesha Rahman — Partner, Ops
  - Tomás Silva — Director, PMO
  - Nina Cho — Finance Director
  - Evan Price — Talent Lead

## Chaos profile (new pick; tracked)
- Overall chaos level (1–5): **4**
- Rationale: tons of artifacts + template reuse + multiple practices → contradiction-prone.
- Where chaos lives:
  - [x] versioning and duplicates (decks, SOWs, case studies)
  - [x] conflicting metrics (utilization, margin, success fees)
  - [x] naming drift (client aliases, practice names)
  - [x] stale strategy (GTM doc behind reality)
  - [x] shadow spreadsheets (utilization “truth” kept by finance)
  - [x] messy meeting notes (partners + practice leads)

## Intentional contradictions
1) Utilization:
   - Practice dashboard says 78%
   - Finance tracker says 71% (excludes non-billable "internal" work)
2) Pipeline:
   - CRM export shows deal as "Proposal"
   - Partner meeting notes say "verbally won"
3) Success fee accounting:
   - Case study claims $1.2M success fee
   - Finance summary books $0.9M (timing + contingencies)

## Artifact set

### Strategy + identity
- mission-vision-values.md
- brand-guide.md
- high-level-strategy-2026.md
- practices-overview.md

### Marketing + sales
- marketing-pitch-deck.pptx
- case-studies-portfolio.pdf
- proposal-template.md
- lead-list.json
- messaging-pillars.md

### Finance + pipeline
- quarterly-revenue-history-2024-2025.xlsx
- projected-revenue-2026.xlsx
- pipeline-report-2026-q1.xlsx
- utilization-tracker-2025.xlsx
- kpi-definitions.json

### Delivery (sample engagement)
- sample-engagement-IRONCLAD/ (folder)
  - statement-of-work.pdf
  - kickoff-notes.md
  - weekly-status-2026-01-12.md
  - weekly-status-2026-01-26.md
  - deliverable-turnaround-plan-v1.pptx
  - deliverable-turnaround-plan-v4-FINAL.pptx
  - benefits-tracker.xlsx
  - closeout-summary.md (may be incomplete)

### Meetings + notes
- partner-meeting-notes-2026-02-03.md
- practice-leads-notes-2026-02-10.md
- voice-note-transcript-2026-02-18.md

### Evaluation templates
- client-intake-form.json
- engagement-retro-template.md
- consultant-performance-rubric.md

### Word documents (.docx)

- practice-performance-2026-01.docx

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

Targets (exact):
- logos: 6
- people: 22
- social: 18
- diagrams: 18

### Word documents (.docx)

- practice-performance-2026-01.docx

## Timeline of significant events
- 2024-01: Managing Partner change (Marisol).
- 2024-07: New Turnaround practice launched; overlaps with Ops.
- 2025-03: Finance rolls out new utilization definition (not adopted).
- 2025-11: Big success-fee case closes; marketing claims larger number.
- 2026-02: Practice conflict on pipeline truth.

## Benchmark goals (IKAM)
- Queries:
  1) “What is real utilization and why do numbers differ?”
  2) “Which deals are actually won vs just in CRM?”
  3) “What success fees exist and how are they recognized?”
  4) “Find the latest FINAL deliverable deck for IRONCLAD.”
