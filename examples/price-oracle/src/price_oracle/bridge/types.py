"""Types for Bridge client failure handling and status tracking."""

from typing import Optional

from pydantic import BaseModel


class PushResult(BaseModel):
    """Result of pushing a message to the Bridge validator."""

    success: bool
    message_id: Optional[str] = None
    signature: Optional[str] = None
    error_code: Optional[str] = None
    error_message: Optional[str] = None
    is_retryable: bool = False
    attempts: int = 1


class MessageStatus(BaseModel):
    """Status of a message on the Bridge.

    Message stages:
    - 0: not_initialized
    - 1: pending
    - 2: ready
    - 3: pre_execution
    - 4: executing
    - 5: post_execution
    - 6: completed
    - 7: failed
    - 8: expired
    """

    message_id: str
    stage: int
    status: str  # Human-readable status string
    executed: bool
    signatures_collected: int = 0
    signature_threshold: int = 1

    @property
    def is_terminal(self) -> bool:
        """Check if the message has reached a terminal state."""
        return self.stage in (6, 7, 8)  # completed, failed, expired

    @property
    def is_success(self) -> bool:
        """Check if the message was successfully executed."""
        return self.stage == 6  # completed


# Error codes that indicate the error is transient and can be retried
RETRYABLE_ERRORS = {
    "STORAGE_PUBLISH_FAILED",
    "BRIDGE_SUBMIT_FAILED",
    "BRIDGE_CONNECTION_FAILED",
    "INVARIANT_DATA_UNAVAILABLE",
    "INTERNAL_ERROR",
}

# Map stage numbers to human-readable status strings
STAGE_STATUS_MAP = {
    0: "not_initialized",
    1: "pending",
    2: "ready",
    3: "pre_execution",
    4: "executing",
    5: "post_execution",
    6: "completed",
    7: "failed",
    8: "expired",
}


def is_retryable_error(error_code: Optional[str]) -> bool:
    """Check if an error code indicates a retryable error."""
    return error_code in RETRYABLE_ERRORS if error_code else False


def stage_to_status(stage: int) -> str:
    """Convert a stage number to a human-readable status string."""
    return STAGE_STATUS_MAP.get(stage, f"unknown_{stage}")
