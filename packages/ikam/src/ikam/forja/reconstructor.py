"""Forja reconstructor

Reconstruction logic to invert decomposition and guarantee
lossless round-trips via root relation DAG traversal.

V3 Fragment Algebra:
- reconstruct_document() uses root relation fragment for DAG-based reconstruction
- reconstruct_binary() reconstructs binary content from V3 fragments

Version: 2.0.0 (V3 Fragment Algebra - February 2026)
"""

from __future__ import annotations

import base64
from typing import List

from ikam.fragments import (
	Fragment,
	Relation,
	RELATION_MIME,
	is_relation_fragment,
)


class ReconstructionError(Exception):
	"""Error during fragment reconstruction."""


def _find_root_relation(fragments: List[Fragment]) -> Fragment:
	"""Find the root relation fragment in a list of V3 fragments.

	Raises ReconstructionError if no relation fragment found.
	"""
	relation_frags = [f for f in fragments if is_relation_fragment(f)]
	if not relation_frags:
		raise ReconstructionError(
			"No root relation fragment found. V3 reconstruction requires a relation "
			"fragment (MIME application/ikam-relation+json) as DAG entry point."
		)
	return relation_frags[0]


def _extract_canonical_bytes(
	fragments: List[Fragment],
	root_relation: Fragment,
) -> bytes:
	"""Extract canonical bytes from the fragment referenced by root relation's 'canonical' slot."""
	rel = Relation.model_validate(root_relation.value)

	# Find canonical slot
	canonical_id = None
	for bg in rel.binding_groups:
		for sb in bg.slots:
			if sb.slot == "canonical":
				canonical_id = sb.fragment_id
				break
		if canonical_id:
			break

	if canonical_id is None:
		raise ReconstructionError(
			"Root relation has no 'canonical' slot binding. "
			"Cannot determine which fragment holds the original bytes."
		)

	# Find the fragment with that cas_id
	canonical_frag = None
	for f in fragments:
		if f.cas_id == canonical_id:
			canonical_frag = f
			break

	if canonical_frag is None:
		raise ReconstructionError(
			f"Canonical fragment {canonical_id} referenced by root relation "
			f"not found in fragment list."
		)

	# Extract bytes_b64 from value
	if not isinstance(canonical_frag.value, dict) or "bytes_b64" not in canonical_frag.value:
		raise ReconstructionError(
			"Canonical fragment value must be a dict with 'bytes_b64' key."
		)

	try:
		return base64.b64decode(canonical_frag.value["bytes_b64"])
	except Exception as e:
		raise ReconstructionError(f"Failed to decode canonical bytes: {e}") from e


def reconstruct_document(fragments: List[Fragment]) -> str:
	"""Reconstruct document text from V3 fragments via root relation DAG.

	Mathematical Guarantee:
	- Lossless: reconstruct_document(decompose_document(A)) = A

	Args:
		fragments: List of V3 Fragment objects (must include root relation)

	Returns:
		Reconstructed document text

	Raises:
		ReconstructionError: If root relation or canonical fragment missing
	"""
	if not fragments:
		raise ReconstructionError("Cannot reconstruct from empty fragment list")

	root = _find_root_relation(fragments)
	raw_bytes = _extract_canonical_bytes(fragments, root)
	return raw_bytes.decode("utf-8")


def reconstruct_binary(fragments: List[Fragment]) -> bytes:
	"""Reconstruct binary content from V3 fragments via root relation DAG.

	Args:
		fragments: List of V3 Fragment objects (must include root relation)

	Returns:
		Reconstructed binary bytes

	Raises:
		ReconstructionError: If root relation or canonical fragment missing
	"""
	if not fragments:
		raise ReconstructionError("Cannot reconstruct from empty fragment list")

	root = _find_root_relation(fragments)
	return _extract_canonical_bytes(fragments, root)
