"""Core IR Models — Foundational primitives for Computational Structured Graphs.

This module defines the abstract IR primitives (Layer 0) that form the
substrate for all modeling tasks in the Narraciones ecosystem.

Key Guarantees:
- Layer 0/1 Separation: Core IR (ikam) is decoupled from modeling profiles (modelado).
- Byte Fidelity: Every IR instance records its provenance and scope for reconstruction.
- Deterministic Reduction: Canonicalization rules ensure stable graph shapes.

Version: 1.1.0 (Phase 0 Stabilization Pass - February 2026)
"""

from __future__ import annotations

import json
from enum import Enum, StrEnum
from typing import Any, Dict, List, Optional, Union

from pydantic import BaseModel, ConfigDict, Field, field_validator

try:
    from blake3 import blake3 as _blake3

    def _blake3_hexdigest(data: bytes) -> str:
        return _blake3(data).hexdigest()
except ImportError:
    import hashlib

    def _blake3_hexdigest(data: bytes) -> str:
        return hashlib.blake2b(data).hexdigest()


# -- Base Metadata (D8, D11) --

class IRBase(BaseModel):
    """Foundational metadata for all IR types.
    
    Ensures that every fragment in the graph can be traced to its artifact,
    conversation scope, and derivation provenance.
    """
    artifact_id: str
    fragment_id: Optional[str] = Field(default=None, description="The logical UUID inode identifying this specific fragment in the graph.")
    cas_id: Optional[str] = Field(default=None, description="The content-addressable hash pointing to the byte-wise deduplicated storage.")
    scope_id: Optional[str] = None
    provenance_id: Optional[str] = None
    lsn: int = Field(default=0, description="Log Sequence Number for version tracking.")
    
    model_config = ConfigDict(populate_by_name=True)


# -- Expression IR (M16) --

class OpType(str, Enum):
    """Operation types for IR/DSL."""
    # Data loading
    LOAD = "LOAD"
    REF = "REF"
    
    # Arithmetic
    ADD = "ADD"
    SUB = "SUB"
    MUL = "MUL"
    DIV = "DIV"
    
    # Aggregations
    AGG_SUM = "AGG_SUM"
    AGG_AVG = "AGG_AVG"
    AGG_MAX = "AGG_MAX"
    AGG_MIN = "AGG_MIN"
    AGG_COUNT = "AGG_COUNT"
    
    # Data operations
    JOIN = "JOIN"
    MAP = "MAP"
    FILTER = "FILTER"
    RANGE = "RANGE"
    
    # Rendering
    CHART = "CHART"
    FORMAT = "FORMAT"
    
    # Control flow
    LET = "LET"
    LAMBDA = "LAMBDA"


class VarType(str, Enum):
    """Variable types for symbol table."""
    SCALAR = "scalar"
    VECTOR = "vector"
    MATRIX = "matrix"
    TABLE = "table"


class Symbol(BaseModel):
    """Symbol table entry for variables."""
    symbol_id: str
    name: str
    type: VarType
    units: Optional[str] = None
    metadata: Dict[str, Any] = Field(default_factory=dict)
    
    model_config = ConfigDict(populate_by_name=True)


class OpAST(BaseModel):
    """Abstract Syntax Tree node for operations."""
    op_type: OpType
    operands: List[Union[OpAST, str, int, float]] = Field(default_factory=list)
    params: Dict[str, Any] = Field(default_factory=dict)
    
    model_config = ConfigDict(populate_by_name=True, arbitrary_types_allowed=True)


class ExpressionIR(IRBase):
    """Symbolic and numeric expressions for logic and derivation.
    
    Stored with MIME application/ikam-expression+v1+json.
    """
    ast: OpAST
    output_type: Optional[VarType] = None
    output_units: Optional[str] = None
    
    # Support for OpShape/OpInstance legacy fields if needed
    shape_id: Optional[str] = None
    instance_id: Optional[str] = None


class OpShape(BaseModel):
    """Canonical operation shape for deduplication."""
    shape_id: str
    shape_hash: str  # blake3 hex of canonical AST
    ast: OpAST
    arity: int
    op_type: OpType
    
    model_config = ConfigDict(populate_by_name=True)


class OpInstance(BaseModel):
    """Operation instance with bound parameters."""
    instance_id: str
    shape_id: str
    artifact_id: Optional[str] = None
    params: Dict[str, Any] = Field(default_factory=dict)
    scope: Optional[str] = None
    
    model_config = ConfigDict(populate_by_name=True)


# -- Proposition IR (M2, M3, M12) --

class Modality(StrEnum):
    FACTUAL = "factual"
    FORECAST = "forecast"
    HYPOTHETICAL = "hypothetical"
    NORMATIVE = "normative"
    CAUSAL = "causal"
    DIRECTIVE = "directive"


class EvidenceRole(StrEnum):
    PRIMARY = "primary"
    CONTEXTUAL = "contextual"
    REBUTTAL = "rebuttal"
    UNDERCUT = "undercut"


class EvidenceRef(BaseModel):
    """Reference to any fragment serving as evidence."""
    fragment_id: str
    role: EvidenceRole = EvidenceRole.PRIMARY
    weight: float = Field(default=1.0, ge=0.0, le=1.0)
    meta: Dict[str, Any] = Field(default_factory=dict)


class ArgumentRelation(StrEnum):
    SUPPORTS = "supports"
    REBUTS = "rebuts"
    UNDERCUTS = "undercuts"


class ArgumentEdge(BaseModel):
    """Edge in the proposition's internal argument graph."""
    from_id: str  # Can be an evidence_ref.fragment_id or another proposition_id
    to_id: str    # Can be "self" (the claim), another evidence_id, or a connection/edge ID
    relation: ArgumentRelation
    confidence: float = Field(default=1.0, ge=0.0, le=1.0)
    method: Optional[str] = None  # Evaluator/Agent ID


class PropositionIR(IRBase):
    """An atomic proposition with mandatory evidence and Pollock argument structure.
    
    Stored with MIME application/ikam-proposition+v1+json.
    """
    profile: str = Field(..., description="The profile defining the statement schema (e.g. modelado/reasoning@1)")
    statement: Dict[str, Any] = Field(
        ..., 
        description="The normalized assertion payload (e.g. subject/predicate/object)."
    )
    modality: Modality = Modality.FACTUAL
    evidence_refs: List[EvidenceRef] = Field(
        ..., 
        min_length=1, 
        description="Mandatory non-empty evidence references."
    )
    argument_edges: List[ArgumentEdge] = Field(default_factory=list)
    
    confidence: Optional[float] = Field(default=None, ge=0.0, le=1.0)
    evaluation_status: Optional[str] = None  # "supported", "contradicted", "undecidable"
    
    @field_validator("evidence_refs")
    @classmethod
    def validate_non_empty_evidence(cls, v: List[EvidenceRef]) -> List[EvidenceRef]:
        if not v:
            raise ValueError("PropositionIR must have at least one evidence reference.")
        return v


# -- Structured Data IR (D18, M16) --

class DataProfile(StrEnum):
    TABULAR = "tabular"
    TENSOR = "tensor"
    PROSE = "prose"
    STYLE = "style"
    PATCH = "patch"
    ENTITY = "entity"


class AxisRole(StrEnum):
    INDEX = "index"
    COLUMNS = "columns"
    SEQUENCE = "sequence"
    TIME = "time"
    ENTITY = "entity"
    METRIC = "metric"


class Axis(BaseModel):
    """Definition of a dimension/axis in the structured data."""
    name: str
    role: AxisRole
    units: Optional[str] = None
    labels: Optional[List[str]] = None  # Explicit labels if not inferred


class StructuredDataIR(IRBase):
    """Multidimensional or relational data with strict profiles.
    
    Stored with MIME application/ikam-structured-data+v1+json.
    Includes support for tables, tensors, prose backbones, and style sheets.
    """
    profile: Union[DataProfile, str]  # Allow custom profile strings from Layer 1
    shape: List[int] = Field(default_factory=list, description="Dimensionality of the data.")
    axes: List[Axis] = Field(default_factory=list)
    
    data: Any = Field(
        ..., 
        description="The actual data payload (list of lists, dicts, or sequence of IDs)."
    )
    
    dtype_map: Dict[str, str] = Field(default_factory=dict)
    layout: Dict[str, Any] = Field(
        default_factory=dict, 
        description="Layout metadata (e.g. dense/sparse, placement hints)."
    )
    
    metadata: Dict[str, Any] = Field(default_factory=dict)
    provenance_refs: List[str] = Field(default_factory=list)


# -- Legacy Compat IR (MIME versioned replacements) --

class ClaimIR(IRBase):
    """An extracted factual claim expressed as an SPO triple.
    
    Stored with MIME application/ikam-claim+v1+json.
    """
    subject: str
    predicate: str
    object: str
    confidence: float = Field(default=1.0, ge=0.0, le=1.0)
    qualifiers: Dict[str, Any] = Field(default_factory=dict)


class ColumnDef(BaseModel):
    """Column definition for a TableIR."""
    name: str
    dtype: str  # "text", "numeric", "date", "boolean"
    unit: Optional[str] = None
    format_spec: Optional[str] = None  # e.g., "currency:usd:2dp"


class TableIR(IRBase):
    """A table expressed as schema (columns) + rows.
    
    Stored with MIME application/ikam-table+v1+json.
    Rows are dicts keyed by column name.
    """
    columns: List[ColumnDef] = Field(default_factory=list)
    rows: List[Dict[str, Any]] = Field(default_factory=list)


# -- Expression IR Utilities --

class SymbolTable:
    """Symbol table for variable interning."""
    
    def __init__(self):
        self._symbols: Dict[str, Symbol] = {}
        self._name_to_id: Dict[str, str] = {}
    
    def intern(self, name: str, var_type: VarType, units: Optional[str] = None, 
               metadata: Optional[Dict[str, Any]] = None) -> str:
        """Intern a symbol and return its ID (idempotent)."""
        if name in self._name_to_id:
            return self._name_to_id[name]
        
        # Generate deterministic symbol ID
        symbol_id = f"sym:{_blake3_hexdigest(name.encode('utf-8'))[:16]}"
        
        symbol = Symbol(
            symbol_id=symbol_id,
            name=name,
            type=var_type,
            units=units,
            metadata=metadata or {}
        )
        
        self._symbols[symbol_id] = symbol
        self._name_to_id[name] = symbol_id
        return symbol_id
    
    def get(self, symbol_id: str) -> Optional[Symbol]:
        """Retrieve symbol by ID."""
        return self._symbols.get(symbol_id)
    
    def get_by_name(self, name: str) -> Optional[Symbol]:
        """Retrieve symbol by name."""
        symbol_id = self._name_to_id.get(name)
        return self._symbols.get(symbol_id) if symbol_id else None
    
    def all_symbols(self) -> List[Symbol]:
        """Return all symbols."""
        return list(self._symbols.values())


def canonicalize_ast(ast: OpAST) -> OpAST:
    """Canonicalize AST for deterministic hashing.

    Returns a new OpAST — never mutates the input.
    """
    commutative_ops = {OpType.ADD, OpType.MUL, OpType.AGG_SUM}

    operands = list(ast.operands)

    if ast.op_type in commutative_ops and operands:
        operands = sorted(operands, key=lambda x: json.dumps(x, sort_keys=True, default=str))

    canonicalized_operands: List[Any] = []
    for operand in operands:
        if isinstance(operand, OpAST):
            canonicalized_operands.append(canonicalize_ast(operand))
        else:
            canonicalized_operands.append(operand)

    canonical_params = dict(sorted(ast.params.items())) if ast.params else {}

    return OpAST(
        op_type=ast.op_type,
        operands=canonicalized_operands,
        params=canonical_params,
    )


def compute_shape_hash(ast: OpAST) -> str:
    """Compute deterministic hash of canonical AST."""
    canonical = canonicalize_ast(ast)
    ast_json = canonical.model_dump_json(exclude_none=True)
    hash_bytes = _blake3_hexdigest(ast_json.encode("utf-8"))
    return hash_bytes


def create_op_shape(ast: OpAST) -> OpShape:
    """Create OpShape from AST with deterministic hash."""
    canonical_ast = canonicalize_ast(ast)
    shape_hash = compute_shape_hash(canonical_ast)
    shape_id = f"shape:{shape_hash[:16]}"
    arity = len(canonical_ast.operands)

    return OpShape(
        shape_id=shape_id,
        shape_hash=shape_hash,
        ast=canonical_ast,
        arity=arity,
        op_type=canonical_ast.op_type
    )


def fold_constants(ast: OpAST) -> OpAST:
    """Fold constant expressions where possible.

    Returns a new OpAST — never mutates the input.
    """
    if ast.op_type in {OpType.ADD, OpType.SUB, OpType.MUL, OpType.DIV}:
        if all(isinstance(op, (int, float)) for op in ast.operands):
            if ast.op_type == OpType.ADD:
                result: Any = sum(ast.operands)  # type: ignore
            elif ast.op_type == OpType.MUL:
                result = 1
                for op in ast.operands:
                    result *= op  # type: ignore
            elif ast.op_type == OpType.SUB and len(ast.operands) == 2:
                result = ast.operands[0] - ast.operands[1]  # type: ignore
            elif ast.op_type == OpType.DIV and len(ast.operands) == 2:
                result = ast.operands[0] / ast.operands[1]  # type: ignore
            else:
                return ast

            return OpAST(op_type=OpType.LOAD, operands=[result])

    folded_operands: List[Any] = []
    for operand in ast.operands:
        if isinstance(operand, OpAST):
            folded_operands.append(fold_constants(operand))
        else:
            folded_operands.append(operand)

    return OpAST(
        op_type=ast.op_type,
        operands=folded_operands,
        params=dict(ast.params),
    )
