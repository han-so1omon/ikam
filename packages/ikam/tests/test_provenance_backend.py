"""Unit tests for the DB-agnostic IKAM ProvenanceBackend.

These tests intentionally avoid any database usage. Storage and traversal are
exercised through the `ProvenanceGraphStore` port.
"""

from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timedelta
import uuid

from ikam.almacen.provenance_backend import ProvenanceBackend
from ikam.almacen.provenance_store import GroundedArtifact, ProvenanceGraphStore
from ikam.provenance import DerivationRecord, DerivationType


@dataclass
class _InMemoryStore(ProvenanceGraphStore):
    derivations: list[DerivationRecord]

    def list_grounded_artifacts_for_concept(
        self,
        *,
        project_id: str,
        concept_id: str,
        derivation_type: str,
        max_edges: int = 500,
    ):
        return []

    def record_derivation(self, derivation: DerivationRecord) -> str:
        self.derivations.append(derivation)
        return str(uuid.uuid4())

    def list_derivations(
        self,
        *,
        source_key: str | None = None,
        target_key: str | None = None,
        derivation_type: DerivationType | None = None,
    ):
        out: list[DerivationRecord] = []
        for d in self.derivations:
            if source_key is not None and d.source_key != source_key:
                continue
            if target_key is not None and d.target_key != target_key:
                continue
            if derivation_type is not None and d.derivation_type != derivation_type:
                continue
            out.append(d)
        return out


# === Initialization Tests ===

def test_record_and_query_derivations() -> None:
    store = _InMemoryStore(derivations=[])
    backend = ProvenanceBackend(store)

    t0 = datetime.now()
    backend.record_derivation(
        DerivationRecord(
            source_key="blake3:s1",
            target_key="blake3:t1",
            derivation_type=DerivationType.REUSE,
            created_at=t0,
        )
    )
    backend.record_derivation(
        DerivationRecord(
            source_key="blake3:s2",
            target_key="blake3:t2",
            derivation_type=DerivationType.DELTA,
            created_at=t0 + timedelta(seconds=1),
        )
    )

    assert len(backend.get_derivations()) == 2
    assert [d.target_key for d in backend.get_derivations(source_key="blake3:s1")] == [
        "blake3:t1"
    ]
    assert [d.source_key for d in backend.get_derivations(derivation_type=DerivationType.DELTA)] == [
        "blake3:s2"
    ]


# === Derivation Recording Tests ===

def test_get_derivation_chain_linear() -> None:
    store = _InMemoryStore(derivations=[])
    backend = ProvenanceBackend(store)

    backend.record_derivation(
        DerivationRecord(
            source_key="blake3:A",
            target_key="blake3:B",
            derivation_type=DerivationType.DELTA,
        )
    )
    backend.record_derivation(
        DerivationRecord(
            source_key="blake3:B",
            target_key="blake3:C",
            derivation_type=DerivationType.DELTA,
        )
    )

    chain = backend.get_derivation_chain("blake3:C")
    assert [(d.source_key, d.target_key) for d in chain] == [
        ("blake3:A", "blake3:B"),
        ("blake3:B", "blake3:C"),
    ]


def test_get_provenance_chains_deep_and_branching() -> None:
    store = _InMemoryStore(derivations=[])
    backend = ProvenanceBackend(store)

    # Branching: A -> C, B -> C
    backend.record_derivation(
        DerivationRecord(
            source_key="A",
            target_key="C",
            derivation_type=DerivationType.REUSE,
        )
    )
    backend.record_derivation(
        DerivationRecord(
            source_key="B",
            target_key="C",
            derivation_type=DerivationType.REUSE,
        )
    )

    # Deep linear chain feeding into A: X0 -> X1 -> ... -> X15 -> A
    prev = "X0"
    for i in range(1, 16):
        cur = f"X{i}"
        backend.record_derivation(
            DerivationRecord(
                source_key=prev,
                target_key=cur,
                derivation_type=DerivationType.DELTA,
            )
        )
        prev = cur
    backend.record_derivation(
        DerivationRecord(
            source_key=prev,
            target_key="A",
            derivation_type=DerivationType.DELTA,
        )
    )

    chains = backend.get_provenance_chains("C", max_depth=50)
    assert len(chains) == 2
    assert {tuple(c.artifacts) for c in chains} == {
        tuple(["X0"] + [f"X{i}" for i in range(1, 16)] + ["A", "C"]),
        ("B", "C"),
    }


def test_fisher_info_helpers_sum_and_group() -> None:
    store = _InMemoryStore(derivations=[])
    backend = ProvenanceBackend(store)

    backend.record_derivation(
        DerivationRecord(
            source_key="s1",
            target_key="t1",
            derivation_type=DerivationType.REUSE,
            fisher_info_contribution=2.5,
        )
    )
    backend.record_derivation(
        DerivationRecord(
            source_key="s2",
            target_key="t2",
            derivation_type=DerivationType.DELTA,
            fisher_info_contribution=1.0,
        )
    )
    backend.record_derivation(
        DerivationRecord(
            source_key="s3",
            target_key="t3",
            derivation_type=DerivationType.REUSE,
            fisher_info_contribution=None,
        )
    )

    assert backend.calculate_fisher_info_total() == 3.5
    assert backend.get_fisher_info_breakdown() == {"reuse": 2.5, "delta": 1.0}
