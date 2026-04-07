#!/usr/bin/env bash
set -euo pipefail

ROOT="/Users/Shared/p/ingenierias-lentas/narraciones-de-economicos/tests/fixtures/cases"
TOOLS="$ROOT/_image_tools"
LOGDIR="$TOOLS/logs"

mkdir -p "$LOGDIR"

# Build list: (count case)
CASE_LIST_FILE="$LOGDIR/_case_list.txt"
python3 - <<'PY' > "$CASE_LIST_FILE"
from pathlib import Path
root=Path('/Users/Shared/p/ingenierias-lentas/narraciones-de-economicos/tests/fixtures/cases')
rows=[]
for case_dir in sorted([p for p in root.iterdir() if p.is_dir() and not p.name.startswith('_')]):
    pp=case_dir/'assets/images/prompts.jsonl'
    if pp.exists():
        n=sum(1 for _ in pp.open('r',encoding='utf-8'))
        rows.append((n, case_dir.name))
rows.sort()
for n,name in rows:
    print(f"{n} {name}")
print(f"TOTAL {len(rows)}")
PY

TOTAL_CASES=$(tail -n 1 "$CASE_LIST_FILE" | awk '{print $2}')

echo "[run_all] $(date) cases=$TOTAL_CASES"

# iterate all but last TOTAL line
sed '$d' "$CASE_LIST_FILE" | while read -r count case; do

  echo "[run_all] $(date) START case=$case prompts=$count"

  # 1) Run generation (resumable; skips existing in case)
  COMFY_TIMEOUT_S=${COMFY_TIMEOUT_S:-7200} \
  PYTHONUNBUFFERED=1 \
    python3 -u "$TOOLS/run_images_comfy.py" --case-dir "$ROOT/$case" \
      2>&1 | tee "$LOGDIR/${case}.run.log"

  # 2) Sync from Comfy output into case folder
  python3 "$TOOLS/sync_images_from_comfy.py" --case-dir "$ROOT/$case" \
      2>&1 | tee "$LOGDIR/${case}.sync.log"

  # 3) Count-check completeness
  python3 "$TOOLS/check_case_images.py" --case-dir "$ROOT/$case" \
      2>&1 | tee "$LOGDIR/${case}.check.log"

  echo "[run_all] $(date) DONE case=$case"
done

echo "[run_all] $(date) ALL_DONE"