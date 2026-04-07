from __future__ import annotations

import json
import os
import time
from io import BytesIO
from pathlib import Path
from typing import TYPE_CHECKING, Any, Dict, Optional, cast

try:  # pragma: no cover - prefers integration coverage
    from fastavro import (
        parse_schema as _parse_schema,
        schemaless_reader as _schemaless_reader,
        schemaless_writer as _schemaless_writer,
    )
    _FASTAVRO_IMPORT_ERROR: ModuleNotFoundError | None = None
except ModuleNotFoundError as exc:  # pragma: no cover - handled in unit tests without deps
    if exc.name != "fastavro":
        raise
    _parse_schema = _schemaless_reader = _schemaless_writer = None  # type: ignore[assignment]
    _FASTAVRO_IMPORT_ERROR = exc

if TYPE_CHECKING:  # pragma: no cover - import for type checkers only
    from confluent_kafka import Consumer, Producer
    from confluent_kafka.schema_registry import Schema as CKSchema
    from confluent_kafka.schema_registry import SchemaRegistryClient


try:  # pragma: no cover - exercised via integration tests that load Kafka deps
    from confluent_kafka import Consumer as _Consumer, Producer as _Producer
    from confluent_kafka.schema_registry import Schema as _CKSchema
    from confluent_kafka.schema_registry import SchemaRegistryClient as _SchemaRegistryClient
    _KAFKA_IMPORT_ERROR: ModuleNotFoundError | None = None
except ModuleNotFoundError as exc:  # pragma: no cover - handled in unit tests without deps
    if exc.name != "confluent_kafka":
        raise
    _Consumer = _Producer = _CKSchema = _SchemaRegistryClient = None  # type: ignore[assignment]
    _KAFKA_IMPORT_ERROR = exc


def default_kafka_config() -> Dict[str, Any]:
    return {
        "bootstrap.servers": os.getenv("KAFKA_BOOTSTRAP_SERVERS", "localhost:9092"),
        "enable.idempotence": True,
        "acks": "all",
        "compression.type": "lz4",
        "linger.ms": 10,
        "batch.num.messages": 1000,
    }


def _require_kafka() -> None:
    if _KAFKA_IMPORT_ERROR is not None:
        raise RuntimeError(
            "Kafka support requires the optional 'confluent_kafka' package. "
            "Install it locally (pip install confluent-kafka) or run the stack via Docker Compose. "
            f"Original import error: {_KAFKA_IMPORT_ERROR}"
        ) from _KAFKA_IMPORT_ERROR


def _require_fastavro() -> None:
    if _FASTAVRO_IMPORT_ERROR is not None:
        raise RuntimeError(
            "Avro serialization support requires the optional 'fastavro' package. "
            "Install it locally (pip install fastavro) or run the stack via Docker Compose. "
            f"Original import error: {_FASTAVRO_IMPORT_ERROR}"
        ) from _FASTAVRO_IMPORT_ERROR


class AvroSerde:
    def __init__(self, schema: Dict[str, Any]):
        """Initialize an Avro serializer/deserializer.

        If fastavro is unavailable, mark the instance inactive so callers can
        degrade gracefully (e.g., fall back to JSON) instead of crashing the
        entire service during import.
        """

        if _parse_schema is None:
            self.schema = None  # type: ignore[assignment]
            self._active = False
        else:
            self.schema = _parse_schema(schema)
            self._active = True

    def _ensure_active(self) -> None:
        if not getattr(self, "_active", False):
            raise RuntimeError("AvroSerde inactive (fastavro not installed)")

    def serialize(self, value: Dict[str, Any]) -> bytes:
        self._ensure_active()
        assert _schemaless_writer is not None and self.schema is not None
        buf = BytesIO()
        _schemaless_writer(buf, self.schema, value)
        return buf.getvalue()

    def deserialize(self, data: bytes) -> Dict[str, Any]:
        self._ensure_active()
        assert _schemaless_reader is not None and self.schema is not None
        buf = BytesIO(data)
        return _schemaless_reader(buf, self.schema)


def load_avro_schema(path: str) -> Dict[str, Any]:
    original = Path(path)
    candidates: list[Path] = []

    def _add_candidate(candidate: Path) -> None:
        if candidate not in candidates:
            candidates.append(candidate)

    _add_candidate(original)

    env_dir = os.getenv("AVRO_SCHEMAS_DIR")
    if env_dir:
        env_path = Path(env_dir).expanduser()
        candidate = env_path / original.name if env_path.is_dir() else env_path
        _add_candidate(candidate)

    for parent in Path(__file__).resolve().parents:
        schemas_dir = parent / "schemas"
        if schemas_dir.is_dir():
            _add_candidate(schemas_dir / original.name)

    for candidate in candidates:
        if candidate.is_file():
            with candidate.open("r", encoding="utf-8") as f:
                return json.load(f)

    searched = ", ".join(str(c) for c in candidates)
    raise FileNotFoundError(f"Avro schema not found for {original.name}; searched: {searched}")


def build_producer() -> "Producer":
    _require_kafka()
    assert _Producer is not None  # for type checkers
    return cast("Producer", _Producer(default_kafka_config()))


def build_consumer(group_id: str, extra: Optional[Dict[str, Any]] = None) -> "Consumer":
    _require_kafka()
    assert _Consumer is not None  # for type checkers
    cfg = {
        **default_kafka_config(),
        "group.id": group_id,
        "auto.offset.reset": "earliest",
        "enable.auto.commit": False,
    }
    if extra:
        cfg.update(extra)
    return cast("Consumer", _Consumer(cfg))


class ConfluentAvroSerde:
    """Serialize Avro with Confluent framing (magic byte + 4-byte schema id).

    This enables decoding in Kafka UI via Schema Registry.
    """

    def __init__(self, subject: str, schema: Dict[str, Any], registry_url: Optional[str] = None):
        _require_kafka()
        assert _SchemaRegistryClient is not None and _CKSchema is not None
        _require_fastavro()
        assert _parse_schema is not None
        url = registry_url or os.getenv("SCHEMA_REGISTRY_URL", "http://localhost:8081/apis/ccompat/v6")
        self.client = _SchemaRegistryClient({"url": url})
        self.subject = subject
        # Ensure schema is registered and get id
        ck = _CKSchema(json.dumps(schema), "AVRO")
        reg = self.client.register_schema(subject, ck)
        # register_schema may return an integer schema id or a RegisteredSchema
        if isinstance(reg, int):
            self.schema_id = reg
        else:
            self.schema_id = getattr(reg, "id", getattr(reg, "schema_id", None))
            if self.schema_id is None:
                # Fallback to querying latest if type is unexpected
                latest = self.client.get_latest_version(subject)
                self.schema_id = getattr(latest, "schema_id", getattr(latest, "id"))
        self.parsed = _parse_schema(schema)

    def serialize(self, value: Dict[str, Any]) -> bytes:
        # Confluent framing: magic byte 0 + 4-byte big-endian schema id
        _require_fastavro()
        assert _schemaless_writer is not None
        header = bytes([0]) + self.schema_id.to_bytes(4, byteorder="big")
        buf = BytesIO()
        _schemaless_writer(buf, self.parsed, value)
        return header + buf.getvalue()

    def deserialize(self, payload: bytes) -> Dict[str, Any]:
        """Decode data produced with :class:`ConfluentAvroSerde`.

        Currently this is only used in tests to assert on job events.
        """

        _require_fastavro()
        assert _schemaless_reader is not None
        if not payload:
            raise ValueError("empty payload")
        magic = payload[0]
        if magic != 0:
            raise ValueError(f"expected magic byte 0, got {magic}")
        schema_id = int.from_bytes(payload[1:5], byteorder="big")
        if schema_id != self.schema_id:
            # Fallback: fetch schema from registry if a different ID is seen
            latest = self.client.get_latest_version(self.subject)
            schema_str = latest.schema.schema_str
            parsed = _parse_schema(json.loads(schema_str))
        else:
            parsed = self.parsed
        buf = BytesIO(payload[5:])
        return _schemaless_reader(buf, parsed)


__all__ = [
    "AvroSerde",
    "ConfluentAvroSerde",
    "build_producer",
    "build_consumer",
    "default_kafka_config",
    "load_avro_schema",
]
