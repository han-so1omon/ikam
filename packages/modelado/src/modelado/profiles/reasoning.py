from __future__ import annotations
from typing import Any, Dict, List, Optional
from pydantic import BaseModel, Field, ConfigDict

REASONING_V1 = "modelado/reasoning@1"

class PollockClaim(BaseModel):
    """The normalized assertion payload (Subject-Predicate-Object)."""
    subject: str
    predicate: str
    object: str
    context: Dict[str, Any] = Field(default_factory=dict)
    
    model_config = ConfigDict(populate_by_name=True)

class ReasoningProfileV1:
    """Handles validation and interpretation for the reasoning@1 profile."""
    
    @staticmethod
    def validate(payload: Dict[str, Any]) -> PollockClaim:
        """Validate the payload against the PollockClaim schema."""
        return PollockClaim(**payload)

    @staticmethod
    def interpret(claim: PollockClaim, template: Optional[str] = None) -> str:
        """Translates the SPO triple into a human-readable prose fragment.
        
        This is a deterministic template-based interpretation.
        """
        if not template:
            # Default to a simple subject-predicate-object join
            # Replacing underscores in predicate for better readability
            readable_predicate = claim.predicate.replace("_", " ")
            return f"{claim.subject} {readable_predicate} {claim.object}"
        
        return template.format(
            subject=claim.subject,
            predicate=claim.predicate,
            object=claim.object,
            **claim.context
        )
