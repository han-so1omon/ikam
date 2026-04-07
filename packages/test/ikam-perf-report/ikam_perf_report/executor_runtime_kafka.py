from __future__ import annotations

import json
from typing import Any, Callable


def encode_message(payload: dict[str, Any]) -> bytes:
    return json.dumps(payload).encode("utf-8")


def decode_message(message: bytes) -> dict[str, Any]:
    return json.loads(message.decode("utf-8"))


def wait_for_assignment(
    consumer: Any,
    *,
    poll_timeout: float,
    max_polls: int,
) -> bool:
    polls = 0
    while polls < max_polls:
        if consumer.assignment():
            return True
        consumer.poll(poll_timeout)
        polls += 1
    return bool(consumer.assignment())


def poll_json_messages(
    consumer: Any,
    *,
    limit: int,
    poll_timeout: float,
    predicate: Callable[[dict[str, Any]], bool],
    max_polls: int | None = None,
) -> list[dict[str, Any]]:
    messages: list[dict[str, Any]] = []
    polls = 0
    while len(messages) < limit:
        if max_polls is not None and polls >= max_polls:
            break
        raw = consumer.poll(poll_timeout)
        polls += 1
        if raw is None:
            continue
        error = raw.error()
        if error is not None:
            raise RuntimeError(str(error))
        payload = raw.value()
        if payload is None:
            continue
        decoded = decode_message(payload)
        if predicate(decoded):
            messages.append(decoded)
    return messages


def poll_json_topic_messages(
    consumer: Any,
    *,
    limit: int,
    poll_timeout: float,
    predicate: Callable[[dict[str, Any]], bool],
    max_polls: int | None = None,
) -> list[tuple[str, dict[str, Any]]]:
    messages: list[tuple[str, dict[str, Any]]] = []
    polls = 0
    while len(messages) < limit:
        if max_polls is not None and polls >= max_polls:
            break
        raw = consumer.poll(poll_timeout)
        polls += 1
        if raw is None:
            continue
        error = raw.error()
        if error is not None:
            raise RuntimeError(str(error))
        payload = raw.value()
        if payload is None:
            continue
        decoded = decode_message(payload)
        if predicate(decoded):
            messages.append((raw.topic(), decoded))
    return messages
