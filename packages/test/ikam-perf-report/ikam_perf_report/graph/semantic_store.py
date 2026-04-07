from __future__ import annotations

from dataclasses import dataclass, field

from ikam_perf_report.graph.semantic_models import SemanticEntity, SemanticRelation


@dataclass
class SemanticGraphStore:
    entities: list[SemanticEntity] = field(default_factory=list)
    relations: list[SemanticRelation] = field(default_factory=list)

    def add_entity(self, entity: dict) -> None:
        self.entities.append(SemanticEntity(**entity))

    def add_relation(self, relation: dict) -> None:
        self.relations.append(SemanticRelation(**relation))

    def summary(self) -> dict[str, int]:
        return {
            "entities": len(self.entities),
            "relations": len(self.relations),
        }
