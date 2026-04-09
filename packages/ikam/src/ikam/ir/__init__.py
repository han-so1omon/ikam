"""IKAM IR package — Intermediate Representation models and protocols."""
from ikam.ir.mime_types import (
    HEADING, PARAGRAPH, CODE_BLOCK, LIST_ITEM,
    FORMULA_CELL, VALUE_CELL, TABLE_REGION, SLIDE_SHAPE, PDF_PAGE,
    EXPRESSION_IR, CLAIM_IR, TABLE_IR, STYLE_IR,
    RECONSTRUCTION_PROGRAM, VERIFICATION_RESULT,
    GRAPH_SLICE_MIME, GRAPH_DELTA_MIME,
)
from ikam.ir.graph_native import (
    ATOMIC_GRAPH_APPLY_MODE,
    GraphAnchor,
    GraphRegion,
    IKAMGraphDelta,
    IKAMGraphDeltaOp,
    IKAMGraphSlice,
    RemoveGraphDeltaOp,
    UpsertGraphDeltaOp,
)
from ikam.ir.protocols import Lifter, Renderer, Canonicalizer, FragmentEmbedder
from ikam.ir.core import (
    IRBase,
    PropositionIR,
    EvidenceRef,
    EvidenceRole,
    Modality,
    StructuredDataIR,
    DataProfile,
    Axis,
    AxisRole,
    ExpressionIR,
    OpType,
    VarType,
    Symbol,
    OpAST,
    OpShape,
    OpInstance,
    ClaimIR,
    TableIR,
    ColumnDef,
    SymbolTable,
    canonicalize_ast,
    compute_shape_hash,
    create_op_shape,
    fold_constants,
)
from ikam.ir.text_conversion import fragment_to_text
from ikam.ir.reconstruction import ReconstructionProgram, CompositionStep, program_to_fragment

__all__ = [
    # Core IR Primitives
    "IRBase", "PropositionIR", "EvidenceRef", "EvidenceRole", "Modality", "StructuredDataIR", "DataProfile", "Axis", "AxisRole",
    "ExpressionIR",
    # Expression IR components
    "OpType", "VarType", "Symbol", "OpAST", "OpShape", "OpInstance", "SymbolTable",
    "canonicalize_ast", "compute_shape_hash", "create_op_shape", "fold_constants",
    # Legacy & Profile Models
    "ClaimIR", "TableIR", "ColumnDef",
    # MIME types
    "HEADING", "PARAGRAPH", "CODE_BLOCK", "LIST_ITEM",
    "FORMULA_CELL", "VALUE_CELL", "TABLE_REGION", "SLIDE_SHAPE", "PDF_PAGE",
    "EXPRESSION_IR", "CLAIM_IR", "TABLE_IR", "STYLE_IR",
    "RECONSTRUCTION_PROGRAM", "VERIFICATION_RESULT",
    "GRAPH_SLICE_MIME", "GRAPH_DELTA_MIME",
    # Graph-native contracts
    "ATOMIC_GRAPH_APPLY_MODE",
    "GraphAnchor", "GraphRegion",
    "UpsertGraphDeltaOp", "RemoveGraphDeltaOp", "IKAMGraphDeltaOp",
    "IKAMGraphSlice", "IKAMGraphDelta",
    # Protocols
    "Lifter", "Renderer", "Canonicalizer", "FragmentEmbedder",
    # Utilities
    "fragment_to_text",
    # Reconstruction
    "ReconstructionProgram", "CompositionStep", "program_to_fragment",
]
