from pathlib import Path


DISALLOWED_LITERALS = {
    "postgresql://narraciones:narraciones@localhost:5432/narraciones",
    "postgresql://narraciones:narraciones@localhost:55432/ikam_perf_report",
    "postgresql://narraciones:narraciones@postgres:5432/narraciones",
    "postgresql://narraciones:narraciones@ikam-perf-report-postgres:5432/ikam_perf_report",
    "postgresql://narraciones:narraciones@localhost:5432/ikam_perf_report",
}

ALLOWED_FILES = {
    Path("packages/test/ikam-perf-report/docker-compose.yml"),
    Path("packages/modelado/tests/test_local_credential_defaults_guard.py"),
}


def test_no_hardcoded_local_dsns_outside_allowed_local_infra_files():
    repo_root = Path(__file__).resolve().parents[3]
    checked_roots = [repo_root / "packages", repo_root / "docs"]
    matches: list[str] = []

    for root in checked_roots:
        for path in root.rglob("*"):
            if not path.is_file():
                continue
            relative = path.relative_to(repo_root)
            if relative in ALLOWED_FILES:
                continue
            if path.suffix.lower() not in {".py", ".md", ".txt", ".yml", ".yaml"}:
                continue

            text = path.read_text(encoding="utf-8")
            for literal in DISALLOWED_LITERALS:
                if literal in text:
                    matches.append(f"{relative}: {literal}")

    assert not matches, "Found hardcoded local DSNs outside allowed local infra files:\n" + "\n".join(matches)
