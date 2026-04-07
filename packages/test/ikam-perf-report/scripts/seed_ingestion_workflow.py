#!/usr/bin/env python3
"""Seed the canonical 10-step ingestion loop Petri Net as IKAM fragments."""
from __future__ import annotations

import argparse
import logging
import os
import uuid
from typing import List

import psycopg
from modelado.core.execution_context import (
    ExecutionContext,
    ExecutionMode,
    WriteScope,
    execution_context,
)
from modelado.plans.persistence import persist_petri_net
from modelado.plans.ingestion_net import create_ingestion_net_definition
from modelado.plans.schema import PetriNetArc

# Default points at local docker-compose postgres service
DEFAULT_DATABASE_URL = "postgresql://narraciones:narraciones@localhost:5432/narraciones"

LOGGER = logging.getLogger("seed-ingestion-net")

def seed_ingestion_net(database_url: str | None = None) -> None:
    url = database_url or os.getenv("DATABASE_URL") or DEFAULT_DATABASE_URL
    
    # Canonical IDs for the ingestion net
    project_id = str(uuid.uuid5(uuid.NAMESPACE_URL, "modelado/projects/canonical"))
    scope_id = str(uuid.uuid5(uuid.NAMESPACE_URL, "modelado/scopes/canonical-ingestion"))
    
    net_def = create_ingestion_net_definition()
    
    # Generate explicit PetriNetArc objects from the transition inputs/outputs
    # because the envelope refers to them as first-class fragments.
    arcs: List[PetriNetArc] = []
    for transition in net_def["transitions"]:
        for input_arc in transition.inputs:
            arcs.append(PetriNetArc(
                from_kind="place",
                from_id=input_arc.place_id,
                to_kind="transition",
                to_id=transition.transition_id,
                weight=input_arc.weight
            ))
        for output_arc in transition.outputs:
            arcs.append(PetriNetArc(
                from_kind="transition",
                from_id=transition.transition_id,
                to_kind="place",
                to_id=output_arc.place_id,
                weight=output_arc.weight
            ))
            
    # Set up ExecutionContext for writing to IKAM
    ctx = ExecutionContext(
        mode=ExecutionMode.BACKGROUND,
        purpose="Seeding canonical Petri Net",
        write_scope=WriteScope(
            allowed=True,
            project_id=project_id,
            operation="seed_ingestion_net"
        )
    )
    
    with execution_context(ctx):
        try:
            with psycopg.connect(url, autocommit=True) as conn:
                LOGGER.info("Connecting to database: %s", url)
                with conn.cursor() as cur:
                    cur.execute(
                        "DELETE FROM ikam_fragment_store WHERE project_id = %s",
                        (project_id,)
                    )
                    deleted_count = cur.rowcount
                LOGGER.info("Deleted %s existing fragments for project %s", deleted_count, project_id)
                
                plan_ref = persist_petri_net(
                    cx=conn,
                    project_id=project_id,
                    scope_id=scope_id,
                    title="Canonical Ingestion Workflow",
                    goal="Execute the 10-step canonical ingestion loop for any artifact.",
                    places=net_def["places"],
                    transitions=net_def["transitions"],
                    arcs=arcs,
                    marking=net_def["marking"],
                    sections=net_def["sections"]
                )
                LOGGER.info("Successfully persisted fragments to ikam_fragment_store")
                LOGGER.info("Seeding complete.")
                LOGGER.info("Root Fragment ID: %s", plan_ref.fragment_id)
                LOGGER.info("Artifact ID: %s", plan_ref.artifact_id)
        except Exception as e:
            LOGGER.error("Failed to persist fragments: %s", e)
            LOGGER.warning("Database write failed. Ensure postgres is running at %s", url)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--database-url",
        dest="database_url",
        help="PostgreSQL connection string.",
    )
    parser.add_argument(
        "--verbose",
        dest="verbose",
        action="store_true",
        help="Enable debug logging output.",
    )
    args = parser.parse_args()

    level = logging.DEBUG if args.verbose else logging.INFO
    logging.basicConfig(format="[%(levelname)s] %(message)s", level=level)

    seed_ingestion_net(args.database_url)

if __name__ == "__main__":
    main()
