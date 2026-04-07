CREATE TABLE IF NOT EXISTS workflow_trace_promotion_outbox (
    outbox_id BIGSERIAL PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    run_id TEXT,
    payload JSONB NOT NULL,
    committed_trace_fragment_id TEXT,
    lease_owner TEXT,
    lease_expires_at TIMESTAMPTZ,
    processed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_trace_promotion_outbox_workflow_id ON workflow_trace_promotion_outbox(workflow_id);
CREATE INDEX IF NOT EXISTS idx_trace_promotion_outbox_run_id ON workflow_trace_promotion_outbox(run_id);
