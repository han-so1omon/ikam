# Architecture Overview (high-level)

Key components:
- Ingestion API
- Queue (events)
- Normalizer service
- Workflow engine
- Metrics pipeline

Known risk: queue backlog under burst traffic.
