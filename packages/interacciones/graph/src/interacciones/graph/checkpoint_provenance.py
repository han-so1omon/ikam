"""Provenance-aware checkpoint implementation.

Integrates graph checkpoints with operation_history table for full audit trail.
Each checkpoint save records an operation with type='graph_checkpoint'.
"""
from __future__ import annotations

from typing import Any, Mapping, Optional, Tuple
from uuid import UUID, uuid4
import time

from .checkpoint import Checkpoint, CheckpointState


class ProvenanceCheckpoint(Checkpoint):
    """Checkpoint implementation that records to operation_history.
    
    Requires a database connection function that provides connection_scope context.
    Each checkpoint save creates an operation_history record with:
    - operation_type='graph_checkpoint'
    - payload={'state': {...}, 'next_node': '...'}
    - parent_operation_id linked to previous checkpoint (if resuming)
    
    Example:
        >>> from narraciones_base_api.app.core.database import connection_scope
        >>> cp = ProvenanceCheckpoint(connection_scope)
        >>> await cp.save("exec-123", {"x": 1}, "node2")
    """

    def __init__(
        self,
        connection_scope_fn: Any,
        agent_id: str = "graph_executor",
    ) -> None:
        """Initialize with database connection function and agent ID.
        
        Args:
            connection_scope_fn: Callable that returns a database connection context
            agent_id: Agent identifier for provenance records (default: "graph_executor")
        """
        self._connection_scope = connection_scope_fn
        self._agent_id = agent_id
        # Track execution_id → operation_id mapping for parent linking
        self._execution_ops: dict[str, UUID] = {}

    async def save(
        self,
        execution_id: str,
        state: Mapping[str, Any],
        next_node: str,
    ) -> None:
        """Save checkpoint and record to operation_history.
        
        Args:
            execution_id: Unique execution identifier
            state: Current graph state
            next_node: Node to execute next after resume
        """
        import json
        
        # Generate operation ID
        operation_id = uuid4()
        parent_id = self._execution_ops.get(execution_id)
        
        # Record to operation_history
        with self._connection_scope() as cx:
            cursor = cx.cursor()
            
            # Verify parent exists if provided
            parent_id_value: Optional[str] = None
            if parent_id:
                cursor.execute(
                    "SELECT 1 FROM operation_history WHERE operation_id = %s",
                    (str(parent_id),),
                )
                if cursor.fetchone():
                    parent_id_value = str(parent_id)
            
            cursor.execute(
                """
                INSERT INTO operation_history 
                (operation_id, parent_operation_id, agent_id, operation_type, 
                 artifact_ids, payload, created_at)
                VALUES (%s, %s, %s, %s, %s, %s, %s)
                """,
                (
                    str(operation_id),
                    parent_id_value,
                    self._agent_id,
                    "graph_checkpoint",
                    [],  # No artifacts affected by checkpoint itself
                    json.dumps({
                        "execution_id": execution_id,
                        "state": state,
                        "next_node": next_node,
                    }),
                    int(time.time() * 1000),
                ),
            )
        
        # Track for next checkpoint's parent
        self._execution_ops[execution_id] = operation_id

    async def load(self, execution_id: str) -> Optional[CheckpointState]:
        """Load most recent checkpoint for execution_id from operation_history.
        
        Args:
            execution_id: Unique execution identifier
            
        Returns:
            Tuple of (state, next_node) if found, else None
        """
        import json
        
        with self._connection_scope() as cx:
            cursor = cx.cursor()
            cursor.execute(
                """
                SELECT payload, operation_id
                FROM operation_history
                WHERE operation_type = 'graph_checkpoint'
                  AND payload->>'execution_id' = %s
                ORDER BY created_at DESC
                LIMIT 1
                """,
                (execution_id,),
            )
            row = cursor.fetchone()
            
            if not row:
                return None
            
            # Extract payload
            payload_raw = row[0] if isinstance(row, tuple) else row.get("payload")
            op_id_raw = row[1] if isinstance(row, tuple) else row.get("operation_id")
            
            # Normalize payload (may already be dict from psycopg)
            if isinstance(payload_raw, str):
                payload = json.loads(payload_raw)
            elif isinstance(payload_raw, dict):
                payload = payload_raw
            else:
                return None
            
            # Track operation_id for parent linking on next save
            if op_id_raw:
                self._execution_ops[execution_id] = UUID(str(op_id_raw))
            
            state = payload.get("state", {})
            next_node = payload.get("next_node")
            
            if not next_node:
                return None
            
            return (state, next_node)
