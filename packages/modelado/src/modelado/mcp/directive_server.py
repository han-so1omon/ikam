from __future__ import annotations
import hashlib
import json
import sys
import time
import asyncio
import logging
from typing import Dict, Any, List, Optional

from modelado.db import get_connection
from modelado.oraculo.factory import create_ai_client_from_env
from modelado.profiles import PROSE_BACKBONE_V1, PHRASING_DELTA_V1
from modelado.plans.mapping import STRUCTURAL_MAP_SCHEMA_ID
from modelado.reasoning.query import SemanticQuery
from modelado.reasoning.explorer import GraphExplorer
from modelado.reasoning.synthesizer import SynthesizerService

# Exported constants for tests and clients
PROSE_BACKBONE_SCHEMA_ID = PROSE_BACKBONE_V1
PHRASING_DELTA_SCHEMA_ID = PHRASING_DELTA_V1

logger = logging.getLogger(__name__)

def _read_request() -> Dict[str, Any]:
    """Read JSON-RPC 2.0 request from stdin, block/wait if empty."""
    while True:
        line = sys.stdin.readline()
        if not line:
            time.sleep(0.1)
            continue
        try:
            return json.loads(line)
        except json.JSONDecodeError:
            logger.warning("Received invalid or empty JSON input; waiting for next line.")
            time.sleep(0.1)

def _write_response(id_val: Optional[int], result: Any = None, error: Any = None) -> None:
    """Write JSON-RPC 2.0 response to stdout."""
    resp = {"jsonrpc": "2.0", "id": id_val}
    if error is not None:
        resp["error"] = error
    else:
        resp["result"] = result
    sys.stdout.write(json.dumps(resp) + "\n")
    sys.stdout.flush()

def handle_get_ir_directives(params: Dict[str, Any]) -> Any:
    kind = params.get("kind")
    
    directives = {
        "prose": {
            "schema": PROSE_BACKBONE_V1,
            "invariants": [
                "ProseBackbone must sequence Proposition IDs only",
                "PhrasingDelta must target a valid Backbone",
                "Exact byte-identity required for reconstruction"
            ],
            "examples": {
                "backbone": {
                    "schema": PROSE_BACKBONE_V1,
                    "proposition_ids": ["frag_prop_1", "frag_prop_2"]
                }
            }
        },
        "structural-map": {
            "schema": STRUCTURAL_MAP_SCHEMA_ID,
            "invariants": [
                "Hierarchy must be acyclic",
                "Root node must have level 0",
                "Map DNA must be computed via BLAKE3"
            ]
        },
        "proposition": {
            "schema": "application/ikam-proposition+json",
            "invariants": [
                "Proposition must represent a single canonical fact",
                "No artifact-specific phrasing allowed"
            ]
        }
    }
    
    return directives.get(str(kind), {"error": f"Unknown IR kind: {kind}"})

def handle_match_proven_patterns(params: Dict[str, Any]) -> Any:
    dna = params.get("dna", {})
    fingerprint = dna.get("fingerprint")
    
    # Mock lookup for proven patterns based on DNA fingerprint
    # In a production system, this would query a historical database (Layer 9)
    patterns = [
        {
            "pattern_id": "pattern_std_paragraph_v1",
            "confidence": 0.95,
            "program_template": {
                "ops": ["RESOLVE", "MAP", "JOIN", "APPLY", "VERIFY"],
                "config": {"verify_strategy": "hard-gate"}
            },
            "description": "Standard prose reconstruction for multi-sentence paragraphs"
        }
    ]
    
    # If the fingerprint matches a known "demo" case, we can return more specific patterns
    if fingerprint == "demo-fingerprint":
        patterns.append({
            "pattern_id": "pattern_complex_table_v2",
            "confidence": 0.88,
            "program_template": {"ops": ["RESOLVE", "TABULATE", "VERIFY"]}
        })
        
    return {"matches": patterns}


def propose_rerender_subgraph(dna: Dict[str, Any]) -> Dict[str, Any]:
    """
    Given a StructuralDNA fingerprint, return an executable SubgraphProposal JSON
    blueprint describing the required IR fragments, slot bindings, and relational
    metadata needed to satisfy the Render Verification Contract (D18).

    SubgraphProposal schema:
    {
      "proposal_id": str,              # Stable ID for this blueprint
      "dna_fingerprint": str,          # Input DNA fingerprint
      "fragment_templates": [          # Ordered list of IR fragment stubs
        {
          "slot": str,                 # e.g. "backbone", "delta_0", "prop_0"
          "schema": str,               # MIME / profile ID
          "binding": str               # Placeholder e.g. "{{prop_0}}"
        }
      ],
      "relational_metadata": {         # How fragments must connect
        "backbone_slots_propositions": [str],  # backbone.data.proposition_ids[*] -> prop slots
        "delta_targets_backbone": str,         # delta.targets -> backbone slot
        "verify_strategy": str                 # hard-gate | soft-gate
      },
      "reconstruction_ops": [str],     # Ordered operator sequence for re-render
      "confidence": float
    }
    """
    if not isinstance(dna, dict):
        raise ValueError("propose_rerender_subgraph requires a dict 'dna' argument")

    fingerprint = dna.get("fingerprint")
    if not isinstance(fingerprint, str) or not fingerprint.strip():
        raise ValueError(
            "propose_rerender_subgraph requires a non-empty string 'fingerprint' key in dna"
        )
    fingerprint = fingerprint.strip()

    # Determine complexity heuristic from fingerprint
    is_complex = "complex" in fingerprint or "table" in fingerprint

    num_props = 3 if is_complex else 2
    prop_slots = [
        {"slot": f"prop_{i}", "schema": "application/ikam-proposition+v1+json", "binding": f"{{{{prop_{i}}}}}"}
        for i in range(num_props)
    ]

    fragment_templates: List[Dict[str, Any]] = [
        {
            "slot": "backbone",
            "schema": PROSE_BACKBONE_V1,
            "binding": "{{backbone}}",
        },
        *prop_slots,
        {
            "slot": "delta_0",
            "schema": PHRASING_DELTA_V1,
            "binding": "{{delta_0}}",
        },
    ]

    reconstruction_ops = ["RESOLVE", "MAP", "LIFT", "VERIFY", "APPLY", "COMMIT"]
    if is_complex:
        reconstruction_ops.insert(3, "TABULATE")

    proposal_id = "subgraph:" + hashlib.blake2b(
        fingerprint.encode("utf-8"), digest_size=8
    ).hexdigest()

    return {
        "proposal_id": proposal_id,
        "dna_fingerprint": fingerprint,
        "fragment_templates": fragment_templates,
        "relational_metadata": {
            "backbone_slots_propositions": [f"prop_{i}" for i in range(num_props)],
            "delta_targets_backbone": "backbone",
            "verify_strategy": "hard-gate",
        },
        "reconstruction_ops": reconstruction_ops,
        "confidence": 0.88 if is_complex else 0.95,
    }

async def handle_semantic_query(params: Dict[str, Any], synthesizer: SynthesizerService) -> Any:
    """
    Execute a natural language query against the computational graph.
    
    This handles the full SQI pipeline: Discovery -> Interpretation.
    
    Args:
        params: The JSON parameters matching SemanticQuery schema.
        synthesizer: Initialized SynthesizerService instance.
    
    Returns:
        JSON response with 'interpretation' and 'attribution'.
    """
    try:
        # 1. Parse query
        query = SemanticQuery(**params)
        
        # 2. Discovery Phase (Synchronous via GraphExplorer)
        with get_connection() as cx:
            # We use the raw psycopg connection from the wrapper
            subgraph = GraphExplorer.discover(cx._conn, query)
            
        # 3. Interpretation Phase (Asynchronous via SynthesizerService)
        # This satisfies D18 (Byte Fidelity) and D19 (Isolation).
        result = await synthesizer.synthesize(query, subgraph)
        
        return result
    except Exception as e:
        logger.exception("Failed executing semantic_query tool")
        return {"error": str(e)}

async def async_main() -> None:
    logging.basicConfig(level=logging.INFO)
    logger.info("Starting Directive MCP Server")
    
    # Initialize long-lived services
    ai_client = create_ai_client_from_env()
    synthesizer = SynthesizerService(ai_client)
    
    loop = asyncio.get_running_loop()
    
    try:
        while True:
            # Read line in a separate thread to avoid blocking the event loop
            line = await loop.run_in_executor(None, sys.stdin.readline)
            if not line:
                await asyncio.sleep(0.1)
                continue
            
            try:
                req = json.loads(line)
            except json.JSONDecodeError:
                logger.warning("Received invalid or empty JSON input; waiting for next line.")
                continue

            method = req.get("method", "")
            params = req.get("params", {})
            id_val = req.get("id")
            
            if method == "get_ir_directives":
                result = handle_get_ir_directives(params)
            elif method == "match_proven_patterns":
                result = handle_match_proven_patterns(params)
            elif method == "propose_rerender_subgraph":
                result = propose_rerender_subgraph(params.get("dna", {}))
            elif method == "semantic_query":
                result = await handle_semantic_query(params, synthesizer)
            elif method == "list_tools":
                result = [
                    {"name": "get_ir_directives", "description": "Returns IR schema and invariants"},
                    {"name": "match_proven_patterns", "description": "Matches DNA to proven programs"},
                    {"name": "propose_rerender_subgraph", "description": "Returns executable SubgraphProposal blueprint for IR re-render"},
                    {"name": "semantic_query", "description": "Execute a natural language query against the computational graph with byte-fidelity attribution."},
                ]
            else:
                result = {"error": f"Unknown method: {method}"}
                
            _write_response(id_val, result=result)
            
    except KeyboardInterrupt:
        logger.info("Directive MCP Server stopped")
    except Exception as e:
        logger.exception("Directive MCP Server error")
        _write_response(None, error={"message": str(e)})

def main() -> None:
    asyncio.run(async_main())

if __name__ == "__main__":

    main()
