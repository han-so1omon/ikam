__all__ = ["ExecutionDispatcher", "ExecutionQueueBus", "attach_execution_queue_bus"]


def __getattr__(name: str):
    if name == "ExecutionDispatcher":
        from .compiler import ExecutionDispatcher

        return ExecutionDispatcher
    if name == "ExecutionQueueBus":
        from .bus import ExecutionQueueBus

        return ExecutionQueueBus
    if name == "attach_execution_queue_bus":
        from .bus import attach_execution_queue_bus

        return attach_execution_queue_bus
    raise AttributeError(name)
