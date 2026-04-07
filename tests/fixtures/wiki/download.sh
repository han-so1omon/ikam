#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"

download_and_extract() {
  local name="$1"
  local url="$2"
  local zip_path="$ROOT_DIR/${name}.zip"
  local out_path="$ROOT_DIR/${name}"

  if [[ -f "$out_path" ]]; then
    printf '%s already exists at %s\n' "$name" "$out_path"
    return 0
  fi

  curl -L "$url" -o "$zip_path"
  unzip -o "$zip_path" -d "$ROOT_DIR"
  rm -f "$zip_path"
}

download_and_extract "enwik8" "https://mattmahoney.net/dc/enwik8.zip"
download_and_extract "enwik9" "https://mattmahoney.net/dc/enwik9.zip"

printf 'Wiki fixtures ready in %s\n' "$ROOT_DIR"
