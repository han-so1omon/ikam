from __future__ import annotations
import asyncio
import difflib
import json
from typing import Any, Dict, List, cast, Optional

from modelado.operators.core import (
    Operator,
    OperatorEnv,
    OperatorParams,
    record_provenance,
    ProvenanceRecord,
    MIME_PROPOSITION,
    MIME_STRUCTURED_DATA,
    _run_async_safely,
)
from modelado.config.llm_config import LLMConfig, LLMTask, TaskModelSelector
from modelado.oraculo.ai_client import GenerateRequest
from modelado.plans.mapping import StructuralMap, StructuralMapNode
from modelado.profiles import PROSE_BACKBONE_V1, PHRASING_DELTA_V1
from modelado.profiles.prose import ProseBackbone, PhrasingDelta, TextPatchOp

class LiftOperator(Operator):
    """
    Decomposes a source artifact into fragments guided by a StructuralMap.
    Satisfies Architecture Decision D18 for prose-first artifacts.
    Also supports agentic lifting using a whitelisted LLM (Plan C, Task 5).

    Parameters:
        - source_text: str - The original artifact content.
        - structural_map: Dict[str, Any] - The guide for fragmentation.
    """

    def __init__(self, config: LLMConfig | None = None):
        self.config = config or TaskModelSelector.get_config(LLMTask.LIFTING)

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Dict[str, Any]:
        source_text = params.parameters.get("source_text") or (fragment if isinstance(fragment, str) else None)
        smap_data = params.parameters.get("structural_map")

        if source_text is None or smap_data is None:
            raise ValueError("LiftOperator requires 'source_text' and 'structural_map' parameters")

        smap = StructuralMap.model_validate(smap_data)
        
        # We'll return a dictionary of fragments by their intended role
        fragments: Dict[str, Any] = {
            "propositions": [],
            "backbones": [],
            "deltas": [],
            "hierarchy": []
        }

        # Traverse the map and lift fragments
        self._lift_node(smap.root, source_text, fragments, env)

        return fragments

    def _lift_node(self, node: StructuralMapNode, source: str, collection: Dict[str, Any], env: OperatorEnv):
        # 1. If it's a leaf node that represents a prose unit (e.g., paragraph)
        if node.kind in ["paragraph", "prose"]:
            self._lift_prose_unit(node, source, collection, env)
        
        # 2. Recursively lift children
        for child in node.children:
            self._lift_node(child, source, collection, env)
            
    def _lift_prose_unit(self, node: StructuralMapNode, source: str, collection: Dict[str, Any], env: OperatorEnv):
        source_range = node.source_range or {"start": 0, "end": len(source)}
        start = source_range.get("start", 0)
        end = source_range.get("end", len(source))
        unit_text = source[start:end]

        sentences: List[str] = []
        
        # Agentic lifting if LLM is available
        if env.llm:
            try:
                # Synchronously run async generate via bridge safely
                import logging
                response = _run_async_safely(env.llm.generate(GenerateRequest(
                    messages=[
                        {"role": "system", "content": "Decompose the following text into distinct semantic propositions. Return as a JSON object with a single key 'propositions' containing a list of strings."},
                        {"role": "user", "content": unit_text}
                    ],
                    model=self.config.model,
                    temperature=0.0,
                    response_format={"type": "json_object"}
                )))
                data = json.loads(response.text)
                if isinstance(data, dict) and "propositions" in data:
                    sentences = data["propositions"]
                elif isinstance(data, list):
                    sentences = data
            except Exception as e:
                import logging
                logging.warning(f"Agentic lifting failed, falling back to heuristic: {e}")
                # Fallback to heuristic
                sentences = [s.strip() for s in unit_text.split(".") if s.strip()]
        else:
            # Fallback to heuristic
            sentences = [s.strip() for s in unit_text.split(".") if s.strip()]
        
        prop_ids = []
        logical_text = ""
        
        for i, s in enumerate(sentences):
            prop_content = s # This is our "canonical" proposition
            prop_id = f"frag_prop_{node.id}_{i}"
            
            collection["propositions"].append({
                "id": prop_id,
                "mime_type": MIME_PROPOSITION,
                "content": prop_content
            })
            prop_ids.append(prop_id)
            logical_text += prop_content # Logical merge (no spaces/punctuation between them yet)

        # Create Backbone
        backbone_id = f"frag_backbone_{node.id}"
        backbone = ProseBackbone(
            proposition_ids=prop_ids,
            metadata={"source_node_id": node.id, "kind": node.kind}
        )
        collection["backbones"].append({
            "id": backbone_id,
            "mime_type": MIME_STRUCTURED_DATA,
            "schema": PROSE_BACKBONE_V1,
            "content": backbone.model_dump(mode="json", by_alias=True)
        })

        # Create Phrasing Delta to restore original bytes
        delta = self._compute_phrasing_delta(unit_text, logical_text, backbone_id)
        collection["deltas"].append({
            "id": f"frag_delta_{node.id}",
            "mime_type": MIME_STRUCTURED_DATA,
            "schema": PHRASING_DELTA_V1,
            "content": delta.model_dump(mode="json", by_alias=True)
        })

    def _compute_phrasing_delta(self, original: str, logical: str, backbone_id: str) -> PhrasingDelta:
        """
        Computes the phrasing delta between the logical text (concatenated propositions)
        and the original source text. Uses SequenceMatcher for character-level diffing.
        """
        matcher = difflib.SequenceMatcher(None, logical, original)
        ops: List[TextPatchOp] = []
        
        for tag, i1, i2, j1, j2 in matcher.get_opcodes():
            if tag == "insert":
                # Insert from original[j1:j2] into logical at offset i1
                ops.append(TextPatchOp(
                    op="insert",
                    at=i1,
                    text=original[j1:j2],
                    length=0
                ))
            elif tag == "delete":
                # Delete from logical[i1:i2]
                ops.append(TextPatchOp(
                    op="delete",
                    at=i1,
                    length=i2 - i1,
                    text=""
                ))
            elif tag == "replace":
                # Replace logical[i1:i2] with original[j1:j2]
                # Represented as delete then insert
                ops.append(TextPatchOp(
                    op="delete",
                    at=i1,
                    length=i2 - i1,
                    text=""
                ))
                ops.append(TextPatchOp(
                    op="insert",
                    at=i1,
                    text=original[j1:j2],
                    length=0
                ))
        
        return PhrasingDelta(
            target_id=backbone_id,
            ops=ops
        )

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)
