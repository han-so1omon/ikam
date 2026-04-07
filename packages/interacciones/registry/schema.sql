-- PostgreSQL schema for the agent registry

CREATE TABLE IF NOT EXISTS agent_registry (
    agent_id TEXT PRIMARY KEY,
    display_name TEXT,
    status TEXT NOT NULL DEFAULT 'unknown',
    capabilities JSONB NOT NULL,
    url TEXT,
    meta JSONB NOT NULL DEFAULT '{}'::jsonb,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat TIMESTAMPTZ,
    in_flight INTEGER NOT NULL DEFAULT 0
);

COMMENT ON TABLE agent_registry IS 'Registered agents for Interacciones registry';
COMMENT ON COLUMN agent_registry.agent_id IS 'Unique agent identifier';
COMMENT ON COLUMN agent_registry.status IS 'AgentStatus enum as text';
COMMENT ON COLUMN agent_registry.capabilities IS 'JSONB: AgentCapability payload';

CREATE INDEX IF NOT EXISTS idx_agent_registry_status ON agent_registry(status);
CREATE INDEX IF NOT EXISTS idx_agent_registry_last_heartbeat ON agent_registry(last_heartbeat);
