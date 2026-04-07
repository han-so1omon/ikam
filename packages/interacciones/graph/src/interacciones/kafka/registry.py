from __future__ import annotations

from pathlib import Path

try:
    from confluent_kafka.schema_registry import Schema as CKSchema  # type: ignore
    from confluent_kafka.schema_registry import SchemaRegistryClient  # type: ignore
    from confluent_kafka.schema_registry.schema_registry_client import RegisteredSchema  # type: ignore
except Exception:  # noqa: BLE001 - optional dependency, provide graceful fallback
    CKSchema = None  # type: ignore
    SchemaRegistryClient = None  # type: ignore

    class RegisteredSchema:  # type: ignore[override]
        def __init__(self, subject: str, schema_id: int | None = None) -> None:
            self.subject = subject
            self.schema_id = schema_id or -1

    # NOTE: Fallback implementation used only when schema registry libs are
    # unavailable (e.g., lightweight dev/test without Kafka). Methods that rely
    # on real registry functionality will raise at call sites if used.


class AvroSchemas:
    def __init__(self, registry_url: str, schemas_dir: str | Path):
        if SchemaRegistryClient is None:  # Kafka disabled; no real client
            self.client = None
        else:
            self.client = SchemaRegistryClient({"url": registry_url})
        self.schemas_dir = Path(schemas_dir)

    def _load(self, filename: str) -> str:
        with open(self.schemas_dir / filename, "r", encoding="utf-8") as f:
            return f.read()

    def register(self, subject: str, filename: str) -> RegisteredSchema:
        schema_str = self._load(filename)
        if self.client is None or CKSchema is None:  # pragma: no cover - fallback path
            # Return a placeholder RegisteredSchema; callers using it for codec
            # construction should detect disabled Kafka via config flags.
            return RegisteredSchema(subject, -1)
        schema = CKSchema(schema_str, "AVRO")
        _ = self.client.register_schema(subject, schema)
        return self.client.get_latest_version(subject)

    def ensure_subjects(self) -> dict[str, RegisteredSchema]:
        subjects = {
            "base.requests-value": "base-model.avsc",
            "base.results-value": "base-model-result.avsc",
            "econ.requests-value": "econ-plan.avsc",
            "econ.results-value": "econ-plan-result.avsc",
            "story.requests-value": "story.avsc",
            "story.results-value": "story-result.avsc",
            # Jobs orchestration
            "jobs.requested-value": "jobs-requested.avsc",
            # Job events for UI visibility
            "jobs.events-value": "jobs-event.avsc",
        }
        out: dict[str, RegisteredSchema] = {}
        for sub, file in subjects.items():
            out[sub] = self.register(sub, file)
        return out
