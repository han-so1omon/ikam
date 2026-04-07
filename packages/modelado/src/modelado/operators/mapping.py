from __future__ import annotations
from typing import Any, Dict, cast

from modelado.operators.core import (
    Operator,
    OperatorEnv,
    OperatorParams,
    record_provenance,
    ProvenanceRecord,
)
from modelado.plans.mapping import StructuralMap, compute_map_dna, StructuralMapNode


class MapDNAOperator(Operator):
    """
    Produces a StructuralMap and computes its Map DNA from a raw structural hierarchy.

    Parameters:
        - artifact_id: str - ID of the artifact being mapped.
        - structural_hierarchy: Dict[str, Any] - Raw structural hierarchy.
    """

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Dict[str, Any]:
        artifact_id = params.parameters.get("artifact_id")
        hierarchy_data = params.parameters.get("structural_hierarchy")

        if not artifact_id or not hierarchy_data:
            raise ValueError("MapDNAOperator requires 'artifact_id' and 'structural_hierarchy' parameters")

        # Convert raw hierarchy to StructuralMapNode models
        root_node = StructuralMapNode.model_validate(hierarchy_data)

        smap = StructuralMap(
            artifact_id=cast(str, artifact_id),
            root=root_node,
        )

        # Compute DNA and attach it to the map
        dna = compute_map_dna(smap)
        smap.dna = dna

        # Return the complete map including DNA
        return smap.model_dump(mode="json", by_alias=True)

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)
