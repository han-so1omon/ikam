import os
import pytest


# IKAM tests require explicit opt-in and a reachable Postgres.
# Opt-in env var prevents accidental execution inside unrelated suites/containers.
RUN_IKAM = os.environ.get("RUN_IKAM_TESTS") == "1"
DB_URL = os.environ.get("TEST_DATABASE_URL") or os.environ.get("PYTEST_DATABASE_URL")

def _can_connect(url: str) -> bool:
    if not url:
        return False
    try:
        import psycopg
        # Add short timeout to avoid long hangs during collection
        with psycopg.connect(url, connect_timeout=2):
            return True
    except Exception:
        return False

SHOULD_SKIP = not RUN_IKAM or not _can_connect(DB_URL or "")

pytestmark = pytest.mark.skipif(
    SHOULD_SKIP,
    reason=(
        "IKAM tests skipped: require RUN_IKAM_TESTS=1 and reachable Postgres via "
        "TEST_DATABASE_URL or PYTEST_DATABASE_URL (see AGENTS.md)."
    ),
)
