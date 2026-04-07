# packages/ikam/src/ikam/ir/mime_types.py
"""MIME type constants for IKAM fragment types.

Surface fragments, IR fragments, and system fragments all have
explicit MIME types. This module is the single source of truth.
"""

# -- Surface fragment MIME types --
HEADING = "text/ikam-heading"
PARAGRAPH = "text/ikam-paragraph"
CODE_BLOCK = "text/ikam-code-block"
LIST_ITEM = "text/ikam-list-item"
FORMULA_CELL = "application/ikam-formula-cell+json"
VALUE_CELL = "application/ikam-value-cell+json"
TABLE_REGION = "application/ikam-table-region+json"
SLIDE_SHAPE = "application/ikam-slide-shape+json"
PDF_PAGE = "application/ikam-pdf-page+json"

# -- IR fragment MIME types --
PROPOSITION_IR = "application/ikam-proposition+v1+json"
STRUCTURED_DATA_IR = "application/ikam-structured-data+v1+json"
EXPRESSION_IR = "application/ikam-expression+v1+json"
CLAIM_IR = "application/ikam-claim+v1+json"
TABLE_IR = "application/ikam-table+v1+json"
STYLE_IR = "application/ikam-style+v1+json"

# -- System fragment MIME types --
RECONSTRUCTION_PROGRAM = "application/ikam-reconstruction-program+json"
VERIFICATION_RESULT = "application/ikam-verification-result+json"
PREDICATE_VOCABULARY = "application/ikam-predicate-vocabulary+json"
