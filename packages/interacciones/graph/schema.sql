-- Graph checkpoints table for PostgresCheckpoint
-- Stores execution state and next node for pause/resume functionality

CREATE TABLE IF NOT EXISTS graph_checkpoints (
    execution_id TEXT PRIMARY KEY,
    state JSONB NOT NULL,
    next_node TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for querying by execution_id (already covered by PK)
-- Index for cleanup queries by timestamp
CREATE INDEX IF NOT EXISTS idx_graph_checkpoints_updated_at ON graph_checkpoints(updated_at);

-- Optional: Add a comment for documentation
COMMENT ON TABLE graph_checkpoints IS 'Stores graph execution checkpoints for pause/resume with human-in-the-loop';
COMMENT ON COLUMN graph_checkpoints.execution_id IS 'Unique identifier for the graph execution';
COMMENT ON COLUMN graph_checkpoints.state IS 'Current graph state as JSONB';
COMMENT ON COLUMN graph_checkpoints.next_node IS 'Node ID to resume execution from';
