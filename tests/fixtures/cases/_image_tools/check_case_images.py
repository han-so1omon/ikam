from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path


def main():
    ap = argparse.ArgumentParser(
        description="Check image completeness for a case by comparing prompts.jsonl to files present in assets/images."  # noqa: E501
    )
    ap.add_argument("--case-dir", required=True)
    args = ap.parse_args()

    case_dir = Path(args.case_dir).expanduser().resolve()
    prompts_path = case_dir / "assets/images/prompts.jsonl"
    if not prompts_path.exists():
        raise SystemExit(f"Missing prompts.jsonl: {prompts_path}")

    lines = [l for l in prompts_path.read_text(encoding="utf-8").splitlines() if l.strip()]
    specs = [json.loads(l) for l in lines]

    expected_by_kind = Counter(s.get("kind", "unknown") for s in specs)

    # actual by scanning for each expected file path
    present_by_kind = Counter()
    missing_by_kind = Counter()

    for s in specs:
        kind = s.get("kind", "unknown")
        rel = Path(s["out_path"])
        if (case_dir / rel).exists():
            present_by_kind[kind] += 1
        else:
            missing_by_kind[kind] += 1

    total_expected = sum(expected_by_kind.values())
    total_present = sum(present_by_kind.values())

    print(f"CASE {case_dir.name}")
    print(f"TOTAL expected={total_expected} present={total_present} missing={total_expected-total_present}")
    for kind in sorted(expected_by_kind.keys()):
        e = expected_by_kind[kind]
        p = present_by_kind.get(kind, 0)
        m = e - p
        print(f"- {kind}: {p}/{e} present (missing {m})")


if __name__ == "__main__":
    main()
