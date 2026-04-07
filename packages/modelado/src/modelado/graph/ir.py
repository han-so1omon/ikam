from __future__ import annotations

from pydantic import BaseModel, ConfigDict, Field

from ikam.ir.core import ExpressionIR, PropositionIR, StructuredDataIR


class LoweredExecutableGraph(BaseModel):
    executable_graph: StructuredDataIR
    operators: list[ExpressionIR] = Field(default_factory=list)
    graph_edges: list[PropositionIR] = Field(default_factory=list)
    source_workflow: StructuredDataIR | None = None
    source_graph_link: PropositionIR | None = None

    model_config = ConfigDict(arbitrary_types_allowed=True)

    @property
    def fragments(self) -> list[StructuredDataIR | ExpressionIR | PropositionIR]:
        fragments: list[StructuredDataIR | ExpressionIR | PropositionIR] = [self.executable_graph, *self.operators, *self.graph_edges]
        if self.source_workflow is not None:
            fragments.append(self.source_workflow)
        if self.source_graph_link is not None:
            fragments.append(self.source_graph_link)
        return fragments
