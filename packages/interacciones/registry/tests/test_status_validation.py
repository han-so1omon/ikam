from interacciones.registry.store_postgres import PostgresRegistryStore
from interacciones.registry.models import AgentStatus


def test_row_to_agent_invalid_status_maps_to_unknown():
    # Minimal dict-like row; store method uses [] key access only
    row = {
        "agent_id": "demo-legacy",
        "display_name": "Legacy Agent",
        "status": "ready",  # invalid value
        "capabilities": None,
        "url": None,
        "meta": None,
        "registered_at": None,
        "last_heartbeat": None,
        "in_flight": 0,
    }
    agent = PostgresRegistryStore._row_to_agent(row)  # type: ignore[arg-type]
    assert agent is not None
    assert agent.status == AgentStatus.UNKNOWN
