"""Test configuration for interacciones.graph tests.

Ensure asyncio tests run reliably across different pytest invocations by
defaulting to pytest-asyncio for this test package.
"""
from __future__ import annotations

import pytest

# Mark all tests in this package to run with pytest-asyncio.
# This avoids failures like "async def functions are not natively supported"
# when global pytest config/plugins differ across environments.
pytestmark = pytest.mark.asyncio
