#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../../../../.." && pwd)"
OUT_BASE="${IKAM_STAGEHAND_OUT:-/tmp/ikam-perf-stagehand}"
TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RUN_DIR="${OUT_BASE}/matrix-${TIMESTAMP}"
mkdir -p "${RUN_DIR}"

CASES="${IKAM_STAGEHAND_CASES:-s-construction-v01,m-construction-v01,l-construction-v01}"
IFS=',' read -r -a CASE_LIST <<< "$CASES"

SUMMARY_FILE="${RUN_DIR}/summary.jsonl"
touch "${SUMMARY_FILE}"

for CASE_ID in "${CASE_LIST[@]}"; do
  CASE_ID="$(echo "$CASE_ID" | xargs)"
  if [[ -z "$CASE_ID" ]]; then
    continue
  fi

  echo "[stagehand-matrix] running case=${CASE_ID}"
  CASE_OUT_DIR="${RUN_DIR}/${CASE_ID}"
  mkdir -p "${CASE_OUT_DIR}"

  if IKAM_STAGEHAND_CASE="$CASE_ID" IKAM_STAGEHAND_OUT="$CASE_OUT_DIR" python "${ROOT_DIR}/packages/test/ikam-perf-report/scripts/stagehand_perf_report.py"; then
    OUTPUT_JSON="$(ls -t "${CASE_OUT_DIR}"/outputs-*.json | head -n 1)"
    VIDEO_FILE="$(python - <<'PY' "$OUTPUT_JSON"
import json,sys
path=sys.argv[1]
with open(path) as f:
    data=json.load(f)
print(data.get('artifacts',{}).get('video',''))
PY
)"
    printf '{"case_id":"%s","status":"pass","outputs":"%s","video":"%s"}\n' "$CASE_ID" "$OUTPUT_JSON" "$VIDEO_FILE" >> "$SUMMARY_FILE"
  else
    printf '{"case_id":"%s","status":"fail"}\n' "$CASE_ID" >> "$SUMMARY_FILE"
    echo "[stagehand-matrix] failed case=${CASE_ID}" >&2
    exit 1
  fi
done

echo "[stagehand-matrix] complete. summary=${SUMMARY_FILE}"
