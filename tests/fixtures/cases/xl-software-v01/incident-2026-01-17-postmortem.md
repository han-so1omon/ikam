# Incident Postmortem — 2026-01-17 (Queue backlog)

Summary: elevated ingestion latency and delayed workflow triggers.

## Narrative
Primary factor: upstream vendor latency amplified our queue.

## Contributing factors
- Consumer autoscaling lag
- Missing alert on queue depth

## Action items
- Add queue-depth SLO
- Add backpressure controls
