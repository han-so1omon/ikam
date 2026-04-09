__all__ = [
    "GraphCompiler",
    "GRAPH_DELTA_ENVELOPE_SCHEMA_ID",
    "IKAMGraphDeltaEnvelope",
    "LoweredGraphDelta",
    "LoweredExecutableGraph",
    "lower_graph_delta_envelope",
    "lower_rich_petri_transition",
]


def __getattr__(name: str):
    if name == "GraphCompiler":
        from .compiler import GraphCompiler

        return GraphCompiler
    if name in {"GRAPH_DELTA_ENVELOPE_SCHEMA_ID", "IKAMGraphDeltaEnvelope"}:
        from .delta_schema import GRAPH_DELTA_ENVELOPE_SCHEMA_ID, IKAMGraphDeltaEnvelope

        return {
            "GRAPH_DELTA_ENVELOPE_SCHEMA_ID": GRAPH_DELTA_ENVELOPE_SCHEMA_ID,
            "IKAMGraphDeltaEnvelope": IKAMGraphDeltaEnvelope,
        }[name]
    if name in {"LoweredGraphDelta", "lower_graph_delta_envelope"}:
        from .delta_lowering import LoweredGraphDelta, lower_graph_delta_envelope

        return {
            "LoweredGraphDelta": LoweredGraphDelta,
            "lower_graph_delta_envelope": lower_graph_delta_envelope,
        }[name]
    if name == "LoweredExecutableGraph":
        from .ir import LoweredExecutableGraph

        return LoweredExecutableGraph
    if name == "lower_rich_petri_transition":
        from .lowering_petri import lower_rich_petri_transition

        return lower_rich_petri_transition
    raise AttributeError(name)
