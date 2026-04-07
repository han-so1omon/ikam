from __future__ import annotations

import os
import json
import importlib
import urllib.error
import urllib.parse
import urllib.request
from typing import Any

import psycopg
from modelado.ikam_graph_schema import create_ikam_schema, truncate_ikam_tables


def reset_ikam_graph_state(project_ids: list[str] | None = None) -> dict[str, Any]:
    """Reset IKAM graph state via modelado-managed backends.

    - Postgres reset is always attempted.
    - HugeGraph/Redis/MinIO reset is attempted when corresponding env vars are configured.
    """

    backends: list[str] = []
    errors: dict[str, str] = {}
    _reset_postgres(project_ids)
    backends.append("postgres")

    if _has_hugegraph_configured():
        try:
            _reset_hugegraph(project_ids)
            backends.append("hugegraph")
        except Exception as exc:
            errors["hugegraph"] = str(exc)

    if os.getenv("REDIS_URL"):
        try:
            _reset_redis()
            backends.append("redis")
        except Exception as exc:
            errors["redis"] = str(exc)

    if _has_minio_configured():
        try:
            _reset_minio()
            backends.append("minio")
        except Exception as exc:
            errors["minio"] = str(exc)

    return {
        "scope": "partial" if project_ids else "all",
        "project_ids": project_ids or [],
        "status": "reset_with_warnings" if errors else "reset",
        "backends": backends,
        "errors": errors,
    }


def _postgres_url() -> str:
    return os.getenv(
        "DATABASE_URL",
        os.getenv("NARRACIONES_DB", "postgresql://narraciones:narraciones@localhost:5432/narraciones"),
    )


def _reset_postgres(project_ids: list[str] | None = None) -> None:
    with psycopg.connect(_postgres_url()) as cx:
        create_ikam_schema(cx)
        if not project_ids:
            truncate_ikam_tables(cx)
            return
        cx.execute(
            "DELETE FROM ikam_artifacts WHERE project_id = ANY(%s)",
            (project_ids,),
        )
        cx.execute(
            """
            DELETE FROM ikam_fragments f
            WHERE NOT EXISTS (
              SELECT 1 FROM ikam_artifact_fragments af WHERE af.fragment_id = f.id
            )
            """,
        )


def _has_hugegraph_configured() -> bool:
    return bool((os.getenv("HUGEGRAPH_URL") or os.getenv("HUGEGRAPH_BASE_URL") or "").strip())


def _reset_hugegraph(project_ids: list[str] | None = None) -> None:
    base_url = (os.getenv("HUGEGRAPH_URL") or os.getenv("HUGEGRAPH_BASE_URL") or "").strip()
    graph = (os.getenv("HUGEGRAPH_GRAPH") or "hugegraph").strip() or "hugegraph"
    endpoint = urllib.parse.urljoin(base_url.rstrip("/") + "/", "gremlin")

    if project_ids:
        selectors = ", ".join([f"'{item}'" for item in project_ids])
        gremlin = (
            "g.E().has('project_id', within(" + selectors + ")).drop();"
            "g.V().has('project_id', within(" + selectors + ")).drop();"
        )
    else:
        gremlin = "g.E().drop(); g.V().drop();"

    payload = json.dumps(
        {
            "gremlin": gremlin,
            "aliases": {"g": f"__g_{graph}"},
        }
    ).encode("utf-8")
    request = urllib.request.Request(
        endpoint,
        data=payload,
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=15):
            pass
    except urllib.error.URLError as exc:  # pragma: no cover
        raise RuntimeError(f"HugeGraph reset failed: {exc}") from exc


def _reset_redis() -> None:
    try:
        redis_module = importlib.import_module("redis")
    except Exception as exc:  # pragma: no cover
        raise RuntimeError("REDIS_URL is set but redis package is unavailable") from exc
    client = redis_module.from_url(os.getenv("REDIS_URL", ""))
    client.flushdb()


def _has_minio_configured() -> bool:
    endpoint = (os.getenv("NARRACIONES_STORAGE_ENDPOINT") or "").strip()
    bucket = (os.getenv("NARRACIONES_STORAGE_BUCKET") or "").strip()
    access = (os.getenv("NARRACIONES_STORAGE_ACCESS_KEY") or "").strip()
    secret = (os.getenv("NARRACIONES_STORAGE_SECRET_KEY") or "").strip()
    return bool(endpoint and bucket and access and secret)


def _reset_minio() -> None:
    try:
        from minio import Minio
    except Exception as exc:  # pragma: no cover
        raise RuntimeError("Storage reset requested but minio package is unavailable") from exc

    endpoint = os.getenv("NARRACIONES_STORAGE_ENDPOINT", "")
    bucket = os.getenv("NARRACIONES_STORAGE_BUCKET", "")
    access = os.getenv("NARRACIONES_STORAGE_ACCESS_KEY", "")
    secret = os.getenv("NARRACIONES_STORAGE_SECRET_KEY", "")
    secure = os.getenv("NARRACIONES_STORAGE_SECURE", "0") in {"1", "true", "True"}

    client = Minio(endpoint, access_key=access, secret_key=secret, secure=secure)
    if not client.bucket_exists(bucket):
        return
    for obj in client.list_objects(bucket, recursive=True):
        object_name = getattr(obj, "object_name", None)
        if object_name:
            client.remove_object(bucket, object_name)
