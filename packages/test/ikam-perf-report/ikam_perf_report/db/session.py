from __future__ import annotations

import os

from psycopg import Connection, connect


def get_database_url() -> str:
    return os.environ.get(
        "DATABASE_URL",
        "postgresql://narraciones:narraciones@localhost:5432/ikam_perf_report",
    )


def open_connection() -> Connection:
    return connect(get_database_url())
