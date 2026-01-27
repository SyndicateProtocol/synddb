"""Pytest fixtures for price oracle tests."""

import os
import sqlite3
import tempfile
from pathlib import Path

import pytest


@pytest.fixture
def temp_db():
    """Create a temporary database file."""
    fd, path = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    yield path
    # Cleanup
    if os.path.exists(path):
        os.unlink(path)


@pytest.fixture
def memory_db():
    """Create an in-memory database connection."""
    conn = sqlite3.connect(":memory:")
    yield conn
    conn.close()
