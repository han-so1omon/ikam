from __future__ import annotations

from pathlib import Path


def load_enwiki8_bytes() -> str:
    data_path = Path(__file__).resolve().parents[5] / "tests" / "fixtures" / "enwik8"
    if not data_path.exists():
        raise FileNotFoundError(f"enwiki8 fixture missing at {data_path}")
    return data_path.read_text()
