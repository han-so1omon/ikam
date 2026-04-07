from enum import Enum
from typing import Optional, Any
from dataclasses import dataclass

class ProgressEventKind(str, Enum):
    OPERATION_STARTED = "operation_started"
    PROGRESS_UPDATE = "progress_update"
    STEP_OUTPUT = "step_output"
    WARNING = "warning"
    ERROR = "error"
    OPERATION_COMPLETED = "operation_completed"
    CANCELLED = "cancelled"

@dataclass
class ProgressEvent:
    event_id: str
    operation_id: str
    project_id: str
    ts: int
    kind: ProgressEventKind
    message: Optional[str] = None
    progress: Optional[float] = None  # 0.0 - 1.0
    stage: Optional[str] = None
    payload: Optional[Any] = None
    actor: Optional[str] = None
    schema_version: str = "1.0.0"
