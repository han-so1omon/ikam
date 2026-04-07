CREATE TABLE IF NOT EXISTS workflow_trace_events (
    trace_id TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    run_id TEXT,
    step_id TEXT,
    event_type TEXT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    transition_id TEXT,
    marking_before_ref TEXT,
    marking_after_ref TEXT,
    enabled_transition_ids JSONB,
    request_id TEXT,
    executor_id TEXT,
    approval_id TEXT,
    lease_owner TEXT,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_workflow_trace_events_workflow_id ON workflow_trace_events(workflow_id);
CREATE INDEX IF NOT EXISTS idx_workflow_trace_events_run_id ON workflow_trace_events(run_id);
CREATE INDEX IF NOT EXISTS idx_workflow_trace_events_occurred_at ON workflow_trace_events(occurred_at);
