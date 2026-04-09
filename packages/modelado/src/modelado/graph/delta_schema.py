from __future__ import annotations

from typing import Annotated, Any, Literal

from pydantic import BaseModel, ConfigDict, Field

from ikam.ir import (
    GRAPH_DELTA_MIME,
    GraphAnchor,
    GraphRegion,
    IKAMGraphDelta,
    RemoveGraphDeltaOp,
    UpsertGraphDeltaOp,
)


GRAPH_DELTA_ENVELOPE_SCHEMA_ID = "modelado/ikam-graph-delta-envelope@1"


class GraphDeltaAnchor(GraphAnchor):
    model_config = ConfigDict(extra="forbid")


class GraphDeltaRegion(GraphRegion):
    anchor: GraphDeltaAnchor

    model_config = ConfigDict(extra="forbid")


class UpsertGraphDeltaSchemaOp(UpsertGraphDeltaOp):
    anchor: GraphDeltaAnchor
    value: Any

    model_config = ConfigDict(extra="forbid")


class RemoveGraphDeltaSchemaOp(RemoveGraphDeltaOp):
    region: GraphDeltaRegion

    model_config = ConfigDict(extra="forbid")


GraphDeltaSchemaOp = Annotated[
    UpsertGraphDeltaSchemaOp | RemoveGraphDeltaSchemaOp,
    Field(discriminator="op"),
]


class IKAMGraphDeltaSchema(IKAMGraphDelta):
    ops: list[GraphDeltaSchemaOp]
    apply_mode: Literal["atomic"] = "atomic"

    model_config = ConfigDict(extra="forbid")


class IKAMGraphDeltaEnvelope(BaseModel):
    schema_id: Literal["modelado/ikam-graph-delta-envelope@1"] = Field(
        default=GRAPH_DELTA_ENVELOPE_SCHEMA_ID,
        alias="schema",
    )
    mime_type: Literal["application/ikam-graph-delta+v1+json"] = GRAPH_DELTA_MIME
    delta: IKAMGraphDeltaSchema

    model_config = ConfigDict(extra="forbid")
