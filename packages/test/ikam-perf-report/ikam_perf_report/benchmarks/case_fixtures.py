from __future__ import annotations

import json
import mimetypes
import os
from dataclasses import dataclass
from fnmatch import fnmatch
from pathlib import Path
from typing import Any


_DEFAULT_CASES_ROOT = "tests/fixtures/cases"
_REGISTRY_FILE = "_case-registry.json"
_IKAMIGNORE_FILE = ".ikamignore"
_EXTENSION_MIME_FALLBACKS = {
    ".md": "text/markdown",
    ".txt": "text/plain",
    ".json": "application/json",
    ".jsonl": "application/jsonl",
    ".csv": "text/csv",
    ".tsv": "text/tab-separated-values",
    ".xlsx": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    ".pptx": "application/vnd.openxmlformats-officedocument.presentationml.presentation",
    ".pdf": "application/pdf",
    ".png": "image/png",
    ".jpg": "image/jpeg",
    ".jpeg": "image/jpeg",
    ".svg": "image/svg+xml",
    ".py": "text/x-python",
}


@dataclass(frozen=True)
class CaseAsset:
    file_name: str
    mime_type: str
    payload: bytes


@dataclass(frozen=True)
class CaseFixture:
    case_id: str
    domain: str
    size_tier: str
    assets: list[CaseAsset]


@dataclass(frozen=True)
class _IgnoreRule:
    pattern: str
    negated: bool
    anchored: bool
    directory_only: bool


def _cases_root() -> Path:
    configured = os.getenv("IKAM_CASES_ROOT", _DEFAULT_CASES_ROOT).strip() or _DEFAULT_CASES_ROOT
    return Path(configured)


def case_fixture_dir(case_id: str) -> Path:
    return _cases_root() / case_id


def load_registry() -> dict[str, Any]:
    registry_path = _cases_root() / _REGISTRY_FILE
    if not registry_path.exists():
        raise FileNotFoundError(f"Case registry not found: {registry_path}")
    return json.loads(registry_path.read_text(encoding="utf-8"))


def available_case_ids() -> list[str]:
    registry = load_registry()
    return [entry["case_id"] for entry in registry.get("cases", [])]


def available_cases() -> list[dict[str, Any]]:
    registry = load_registry()
    cases: list[dict[str, Any]] = []
    for entry in registry.get("cases", []):
        case_id = str(entry.get("case_id", "")).strip()
        if not case_id:
            continue
        image_targets = entry.get("image_targets")
        cases.append({
            "case_id": case_id,
            "domain": str(entry.get("domain", "unknown")),
            "size_tier": str(entry.get("size_tier", "unknown")),
            "chaos_level": int(entry.get("chaos_level", 0) or 0),
            "deliberate_contradictions": bool(entry.get("deliberate_contradictions", False)),
            "idea_file": str(entry.get("idea_file", "idea.md")),
            "image_targets": image_targets if isinstance(image_targets, dict) else {},
        })
    return cases


def parse_case_ids(case_ids_raw: str | None) -> list[str]:
    if not case_ids_raw:
        return available_case_ids()
    return [item.strip() for item in case_ids_raw.split(",") if item.strip()]


def validate_case_ids(case_ids: list[str]) -> tuple[list[str], list[str]]:
    known = set(available_case_ids())
    valid = [case_id for case_id in case_ids if case_id in known]
    missing = [case_id for case_id in case_ids if case_id not in known]
    return valid, missing


def _detect_mime_type(path: Path) -> str:
    guessed, _ = mimetypes.guess_type(str(path))
    if guessed:
        return guessed
    return _EXTENSION_MIME_FALLBACKS.get(path.suffix.lower(), "application/octet-stream")


def _parse_ignore_file(path: Path) -> list[_IgnoreRule]:
    if not path.exists() or not path.is_file():
        return []
    rules: list[_IgnoreRule] = []
    for raw_line in path.read_text(encoding="utf-8").splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        negated = line.startswith("!")
        if negated:
            line = line[1:].strip()
            if not line:
                continue
        anchored = line.startswith("/")
        if anchored:
            line = line[1:]
        normalized = line.replace("\\", "/")
        directory_only = normalized.endswith("/")
        if directory_only:
            normalized = normalized.rstrip("/")
        if not normalized:
            continue
        rules.append(
            _IgnoreRule(
                pattern=normalized,
                negated=negated,
                anchored=anchored,
                directory_only=directory_only,
            )
        )
    return rules


def _match_rule(path_str: str, rule: _IgnoreRule) -> bool:
    if rule.directory_only:
        if rule.anchored:
            return path_str == rule.pattern or path_str.startswith(f"{rule.pattern}/")
        parts = path_str.split("/")
        for index, part in enumerate(parts):
            if part != rule.pattern:
                continue
            candidate = "/".join(parts[: index + 1])
            if path_str == candidate or path_str.startswith(f"{candidate}/"):
                return True
        return False

    pattern = rule.pattern
    if rule.anchored:
        return fnmatch(path_str, pattern)
    if "/" in pattern:
        return fnmatch(path_str, pattern) or fnmatch(path_str, f"**/{pattern}")
    return fnmatch(path_str, pattern) or fnmatch(path_str, f"**/{pattern}")


def _is_ignored(path_str: str, rules: list[_IgnoreRule]) -> bool:
    ignored = False
    for rule in rules:
        if _match_rule(path_str, rule):
            ignored = not rule.negated
    return ignored


def load_case_fixture(case_id: str) -> CaseFixture:
    registry = load_registry()
    cases = {entry["case_id"]: entry for entry in registry.get("cases", [])}
    if case_id not in cases:
        raise ValueError(f"Unknown case_id: {case_id}")
    case_meta = cases[case_id]
    case_dir = case_fixture_dir(case_id)
    if not case_dir.exists():
        raise FileNotFoundError(f"Case directory missing for {case_id}: {case_dir}")

    root_rules = _parse_ignore_file(_cases_root() / _IKAMIGNORE_FILE)
    case_rules = _parse_ignore_file(case_dir / _IKAMIGNORE_FILE)
    rules = [*root_rules, *case_rules]

    assets: list[CaseAsset] = []
    for path in sorted(case_dir.rglob("*")):
        relative = path.relative_to(case_dir)
        relative_str = str(relative).replace("\\", "/")
        if not path.is_file():
            continue
        if relative_str == _IKAMIGNORE_FILE:
            continue
        if _is_ignored(relative_str, rules):
            continue
        mime_type = _detect_mime_type(path)
        assets.append(
            CaseAsset(
                file_name=str(relative),
                mime_type=mime_type,
                payload=path.read_bytes(),
            )
        )

    return CaseFixture(
        case_id=case_id,
        domain=case_meta.get("domain", "unknown"),
        size_tier=case_meta.get("size_tier", "unknown"),
        assets=assets,
    )
