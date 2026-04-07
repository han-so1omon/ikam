from typing import Protocol, List, Any

class ExecutionScope(Protocol):
    def get_dynamic_execution_steps(self) -> List[str]:
        ...

    def get_step_execution_metadata(self, step_name: str) -> dict[str, Any]:
        ...
