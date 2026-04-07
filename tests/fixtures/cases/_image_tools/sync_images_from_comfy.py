from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path

DEFAULT_COMFY_OUTPUT_DIR = Path(
    "/Users/Shared/p/ingenierias-lentas/narraciones-de-economicos/tools/ComfyUI/output"
)


def main():
    ap = argparse.ArgumentParser(
        description="Copy rendered images from ComfyUI/output into a case folder using prompts.jsonl manifest."
    )
    ap.add_argument("--case-dir", required=True)
    ap.add_argument(
        "--comfy-output",
        default=str(DEFAULT_COMFY_OUTPUT_DIR),
        help="Path to ComfyUI/output",
    )
    ap.add_argument(
        "--dry-run",
        action="store_true",
        help="Print actions but do not copy files",
    )
    ap.add_argument(
        "--overwrite",
        action="store_true",
        help="Overwrite case images if they already exist",
    )

    args = ap.parse_args()

    case_dir = Path(args.case_dir).expanduser().resolve()
    prompts_path = case_dir / "assets/images/prompts.jsonl"
    if not prompts_path.exists():
        raise SystemExit(f"Missing prompts.jsonl: {prompts_path}")

    comfy_output = Path(args.comfy_output).expanduser().resolve()
    if not comfy_output.exists():
        raise SystemExit(f"Missing Comfy output dir: {comfy_output}")

    lines = [l for l in prompts_path.read_text(encoding="utf-8").splitlines() if l.strip()]

    copied = 0
    missing = 0
    skipped = 0

    for line in lines:
        spec = json.loads(line)
        rel = Path(spec["out_path"])

        # Comfy writes PNGs with suffix numbering; our filename_prefix uses rel.with_suffix('')
        prefix = str(rel.with_suffix(""))
        # We accept either exact out_path if it exists, or the comfy numbered output
        exact_src = comfy_output / rel

        candidates = []
        if exact_src.exists():
            candidates.append(exact_src)
        else:
            # find first matching numbered file
            globbed = sorted(comfy_output.glob(prefix + "_*.png"))
            candidates.extend(globbed[:1])

        if not candidates:
            missing += 1
            continue

        src = candidates[0]
        dst = case_dir / rel
        dst.parent.mkdir(parents=True, exist_ok=True)

        if dst.exists() and not args.overwrite:
            skipped += 1
            continue

        if args.dry_run:
            print(f"COPY {src} -> {dst}")
        else:
            shutil.copy2(src, dst)
        copied += 1

    print(
        f"Sync complete. copied={copied} skipped={skipped} missing={missing} case={case_dir.name}"
    )


if __name__ == "__main__":
    main()
