CREATE TABLE IF NOT EXISTS workflow_state (
    workflow_id TEXT PRIMARY KEY,
    current_step TEXT NOT NULL,
    status TEXT NOT NULL,
    executor_id TEXT,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    next_run_at TIMESTAMPTZ,
    lease_owner TEXT,
    lease_expires_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_workflow_state_status ON workflow_state(status);
CREATE INDEX IF NOT EXISTS idx_workflow_state_next_run_at ON workflow_state(next_run_at);
