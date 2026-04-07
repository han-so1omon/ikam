"""
Guard test: modelado source files must NOT import deleted adapter functions
directly from ikam.adapters.

These functions were removed from ikam.adapters in Task 2 (commit 258c36c).

Banned patterns:
  - from ikam.adapters import <deleted_function>
  - ikam_adapters.<deleted_function>()
"""

from __future__ import annotations

import re
from pathlib import Path

# Functions deleted from ikam.adapters in Task 2
DELETED_FUNCTIONS = {
    "build_domain_id_to_cas_id_map",
    "cas_id_for_domain_fragment",
    "generate_cas_bytes",
    "domain_to_storage",
    "domain_fragment_from_cas_bytes",
    "storage_to_domain",
    "serialize_content",
    "deserialize_content",
}

MODELADO_SRC = Path(__file__).resolve().parent.parent / "src" / "modelado"

# Patterns that indicate importing a deleted function from ikam.adapters
IMPORT_RE = re.compile(
    r"from\s+ikam\.adapters\s+import\s+.*\b("
    + "|".join(DELETED_FUNCTIONS)
    + r")\b"
)

# Patterns for attribute access via ikam_adapters namespace
ATTR_RE = re.compile(
    r"ikam_adapters\.(" + "|".join(DELETED_FUNCTIONS) + r")\b"
)

# Also catch `from ikam.forja import reconstruct_document_v3` (renamed)
V3_SUFFIX_RE = re.compile(
    r"from\s+ikam\.forja\s+import\s+.*\b(reconstruct_document_v3|reconstruct_binary_v3|decompose_document_v3|decompose_binary_v3)\b"
)


def _scan_file(path: Path) -> list[str]:
    """Return list of violation descriptions found in the file."""
    text = path.read_text()
    violations = []
    in_ikam_adapters_import = False
    for i, line in enumerate(text.splitlines(), 1):
        stripped = line.strip()
        if stripped.startswith("#"):
            continue

        # Track multi-line `from ikam.adapters import (` blocks
        if re.search(r"from\s+ikam\.adapters\s+import\s*\(", line):
            in_ikam_adapters_import = True
        if in_ikam_adapters_import:
            for fn in DELETED_FUNCTIONS:
                if re.search(r"\b" + fn + r"\b", line):
                    violations.append(
                        f"{path.name}:{i} imports deleted '{fn}' from ikam.adapters"
                    )
            if ")" in line:
                in_ikam_adapters_import = False
            continue

        # Single-line from ikam.adapters import ...
        m = IMPORT_RE.search(line)
        if m:
            violations.append(
                f"{path.name}:{i} imports deleted '{m.group(1)}' from ikam.adapters"
            )
        m = ATTR_RE.search(line)
        if m:
            violations.append(
                f"{path.name}:{i} uses deleted 'ikam_adapters.{m.group(1)}'"
            )
        m = V3_SUFFIX_RE.search(line)
        if m:
            violations.append(
                f"{path.name}:{i} imports renamed '{m.group(1)}' (drop _v3 suffix)"
            )
    return violations


def test_modelado_does_not_import_deleted_adapter_functions():
    """No modelado source file may import deleted legacy adapter functions
    directly from ikam.adapters."""
    assert MODELADO_SRC.is_dir(), f"Source dir not found: {MODELADO_SRC}"

    all_violations: list[str] = []
    for py in sorted(MODELADO_SRC.rglob("*.py")):
        all_violations.extend(_scan_file(py))

    assert all_violations == [], (
        "Modelado files still reference deleted ikam.adapters functions:\n"
        + "\n".join(f"  • {v}" for v in all_violations)
    )


def test_modelado_has_no_legacy_adapter_compat_imports():
    """No modelado source file may import from _legacy_adapter_compat."""
    assert MODELADO_SRC.is_dir(), f"Source dir not found: {MODELADO_SRC}"

    violations: list[str] = []
    for py in sorted(MODELADO_SRC.rglob("*.py")):
        text = py.read_text()
        if "_legacy_adapter_compat" in text:
            violations.append(py.name)

    assert violations == [], (
        "Modelado source still imports _legacy_adapter_compat:\n"
        + "\n".join(f"  • {name}" for name in violations)
    )
