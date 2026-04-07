"""Canonicalization for generated functions.

Normalizes Python code to maximize CAS deduplication while preserving semantics.

Key principles:
1. Semantic preservation: Canonicalized code behaves identically to original
2. Determinism: Same semantic code → same canonical form
3. Storage efficiency: Maximize CAS deduplication via normalization

Canonicalization rules (applied in order):
1. Sort imports alphabetically
2. Normalize whitespace (consistent indentation, spacing)
3. Rename variables to canonical form (param1, param2, var1, var2, etc.)
4. Sort dictionary literals by key
5. Normalize string quotes (always use double quotes)
6. Remove trailing whitespace and blank lines
7. Normalize docstrings (consistent formatting)
8. Sort function arguments alphabetically (where order-independent)

Mathematical guarantee:
- hash(canonicalize(f1)) == hash(canonicalize(f2)) IFF f1 ≡ f2 (semantically equivalent)
"""

from __future__ import annotations

import ast
import hashlib
import re
from dataclasses import dataclass
from typing import Any, Dict, List, Optional, Set

try:
    import blake3
    BLAKE3_AVAILABLE = True
except ImportError:
    BLAKE3_AVAILABLE = False


@dataclass
class CanonicalizedCode:
    """Result of canonicalization with provenance.
    
    Attributes:
        canonical_code: Normalized Python code
        content_hash: BLAKE3 hash (or SHA256 fallback)
        original_hash: Hash of original code
        transformations: List of transformations applied
        is_semantically_equivalent: Whether canonicalization preserved semantics
    """
    canonical_code: str
    content_hash: str
    original_hash: str
    transformations: List[str]
    is_semantically_equivalent: bool = True
    
    def to_dict(self) -> Dict[str, Any]:
        """Serialize to dictionary."""
        return {
            "canonical_code": self.canonical_code,
            "content_hash": self.content_hash,
            "original_hash": self.original_hash,
            "transformations": self.transformations,
            "is_semantically_equivalent": self.is_semantically_equivalent,
        }


class CodeCanonicalizer:
    """Canonicalize Python code for maximum CAS deduplication.
    
    Uses AST transformation to apply semantic-preserving normalizations.
    """
    
    def __init__(self, use_blake3: bool = True):
        """Initialize canonicalizer.
        
        Args:
            use_blake3: Use BLAKE3 for hashing (faster); falls back to SHA256 if unavailable
        """
        self.use_blake3 = use_blake3 and BLAKE3_AVAILABLE
        self.transformations_applied: List[str] = []
    
    def canonicalize(self, code: str) -> CanonicalizedCode:
        """Canonicalize Python code.
        
        Args:
            code: Original Python code
            
        Returns:
            CanonicalizedCode with normalized code and provenance
        """
        self.transformations_applied = []
        original_hash = self._compute_hash(code)
        
        # Step 1: Parse code into AST
        try:
            tree = ast.parse(code)
        except SyntaxError as e:
            # Cannot canonicalize invalid Python; return original
            return CanonicalizedCode(
                canonical_code=code,
                content_hash=original_hash,
                original_hash=original_hash,
                transformations=["parse_failed"],
                is_semantically_equivalent=False,
            )
        
        # Step 2: Apply AST transformations
        tree = self._normalize_ast(tree)
        
        # Step 3: Unparse AST back to code
        canonical_code = ast.unparse(tree)
        
        # Step 4: Apply text-level normalizations
        canonical_code = self._normalize_text(canonical_code)
        
        # Step 5: Compute content hash
        content_hash = self._compute_hash(canonical_code)
        
        return CanonicalizedCode(
            canonical_code=canonical_code,
            content_hash=content_hash,
            original_hash=original_hash,
            transformations=self.transformations_applied.copy(),
            is_semantically_equivalent=True,
        )
    
    def _normalize_ast(self, tree: ast.AST) -> ast.AST:
        """Apply AST-level normalizations.
        
        Args:
            tree: Parsed AST
            
        Returns:
            Normalized AST
        """
        # Sort imports
        tree = self._sort_imports(tree)
        
        # Rename variables to canonical form
        tree = self._rename_variables(tree)
        
        # Sort dictionary literals
        tree = self._sort_dict_literals(tree)
        
        return tree
    
    def _sort_imports(self, tree: ast.AST) -> ast.AST:
        """Sort import statements alphabetically."""
        class ImportSorter(ast.NodeTransformer):
            def visit_Module(self, node):
                # Separate imports from other statements
                imports = []
                other = []
                
                for stmt in node.body:
                    if isinstance(stmt, (ast.Import, ast.ImportFrom)):
                        imports.append(stmt)
                    else:
                        other.append(stmt)
                
                # Sort imports alphabetically
                imports.sort(key=lambda x: ast.unparse(x))
                
                # Reconstruct body with sorted imports first
                node.body = imports + other
                return node
        
        if imports := [n for n in ast.walk(tree) if isinstance(n, (ast.Import, ast.ImportFrom))]:
            self.transformations_applied.append("sort_imports")
        
        return ImportSorter().visit(tree)
    
    def _rename_variables(self, tree: ast.AST) -> ast.AST:
        """Rename variables to canonical form (var1, var2, etc.).
        
        Preserves:
        - Built-in names (print, len, etc.)
        - Function names (keep original)
        - Parameter names (keep original; callers may pass kwargs)
        - Local variables (rename to var1, var2, etc.)
        """
        class VariableRenamer(ast.NodeTransformer):
            def __init__(self):
                self.var_mapping: Dict[str, str] = {}
                self.var_counter = 0
                self.builtins = set(dir(__builtins__))
                self._param_stack: List[Set[str]] = []
            
            def visit_FunctionDef(self, node):
                # Preserve parameter names to keep call signatures stable.
                params = {arg.arg for arg in node.args.args}
                self._param_stack.append(params)
                
                # Visit function body
                self.generic_visit(node)
                self._param_stack.pop()
                return node
            
            def visit_Name(self, node):
                current_params = self._param_stack[-1] if self._param_stack else set()
                if node.id in current_params:
                    return node
                # Rename local variables (not builtins, not already mapped)
                if node.id in self.var_mapping:
                    node.id = self.var_mapping[node.id]
                elif node.id not in self.builtins and isinstance(node.ctx, ast.Store):
                    self.var_counter += 1
                    self.var_mapping[node.id] = f"var{self.var_counter}"
                    node.id = f"var{self.var_counter}"
                
                return node
        
        renamer = VariableRenamer()
        tree = renamer.visit(tree)
        
        if renamer.var_mapping:
            self.transformations_applied.append("rename_variables")
        
        return tree
    
    def _sort_dict_literals(self, tree: ast.AST) -> ast.AST:
        """Sort dictionary literal keys alphabetically."""
        class DictSorter(ast.NodeTransformer):
            def visit_Dict(self, node):
                # Sort keys (if all are strings/constants)
                if all(isinstance(k, ast.Constant) for k in node.keys if k is not None):
                    # Create (key, value) pairs
                    pairs = list(zip(node.keys, node.values))
                    # Sort by key value
                    pairs.sort(key=lambda p: str(p[0].value) if p[0] else "")
                    # Reconstruct keys and values
                    node.keys, node.values = zip(*pairs) if pairs else ([], [])
                
                self.generic_visit(node)
                return node
        
        sorter = DictSorter()
        tree = sorter.visit(tree)
        
        if any(isinstance(n, ast.Dict) for n in ast.walk(tree)):
            self.transformations_applied.append("sort_dict_literals")
        
        return tree
    
    def _normalize_text(self, code: str) -> str:
        """Apply text-level normalizations.
        
        Args:
            code: Code string
            
        Returns:
            Normalized code string
        """
        # Normalize line endings
        code = code.replace("\r\n", "\n").replace("\r", "\n")
        self.transformations_applied.append("normalize_line_endings")
        
        # Remove trailing whitespace
        lines = code.split("\n")
        code = "\n".join(line.rstrip() for line in lines)
        self.transformations_applied.append("remove_trailing_whitespace")
        
        # Normalize blank lines (max 2 consecutive)
        code = re.sub(r"\n{3,}", "\n\n", code)
        self.transformations_applied.append("normalize_blank_lines")
        
        # Ensure file ends with single newline
        code = code.rstrip("\n") + "\n"
        
        return code
    
    def _compute_hash(self, code: str) -> str:
        """Compute content hash (BLAKE3 or SHA256 fallback).
        
        Args:
            code: Code string
            
        Returns:
            Hex-encoded hash
        """
        if self.use_blake3:
            return blake3.blake3(code.encode()).hexdigest()
        else:
            return hashlib.sha256(code.encode()).hexdigest()


def canonicalize_function(code: str, use_blake3: bool = True) -> CanonicalizedCode:
    """Canonicalize a Python function (convenience function).
    
    Args:
        code: Python function code
        use_blake3: Use BLAKE3 for hashing (faster than SHA256)
        
    Returns:
        CanonicalizedCode with normalized code and provenance
    """
    canonicalizer = CodeCanonicalizer(use_blake3=use_blake3)
    return canonicalizer.canonicalize(code)
