"""Tests for database schema initialization."""

import sqlite3

from app.schema import init_database, get_tracked_assets, SCHEMA


def test_schema_creates_tables(memory_db):
    """Test that schema creates all required tables."""
    cursor = memory_db.cursor()
    cursor.executescript(SCHEMA)
    memory_db.commit()

    # Check tables exist
    cursor.execute(
        "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name"
    )
    tables = {row[0] for row in cursor.fetchall()}

    assert "prices" in tables
    assert "price_snapshots" in tables
    assert "tracked_assets" in tables
    assert "message_log" in tables
    assert "inbound_message_log" in tables


def test_schema_creates_indexes(memory_db):
    """Test that schema creates all required indexes."""
    cursor = memory_db.cursor()
    cursor.executescript(SCHEMA)
    memory_db.commit()

    cursor.execute(
        "SELECT name FROM sqlite_master WHERE type='index' ORDER BY name"
    )
    indexes = {row[0] for row in cursor.fetchall()}

    assert "idx_prices_asset_time" in indexes
    assert "idx_prices_source" in indexes
    assert "idx_snapshots_asset_time" in indexes


def test_schema_inserts_default_assets(memory_db):
    """Test that schema inserts default tracked assets."""
    cursor = memory_db.cursor()
    cursor.executescript(SCHEMA)
    memory_db.commit()

    cursor.execute("SELECT symbol, display_name FROM tracked_assets")
    assets = {row[0]: row[1] for row in cursor.fetchall()}

    assert "bitcoin" in assets
    assert assets["bitcoin"] == "BTC"
    assert "ethereum" in assets
    assert assets["ethereum"] == "ETH"


def test_init_database(temp_db):
    """Test init_database creates a valid database."""
    conn = init_database(temp_db)

    cursor = conn.cursor()
    cursor.execute(
        "SELECT name FROM sqlite_master WHERE type='table'"
    )
    tables = {row[0] for row in cursor.fetchall()}

    assert "prices" in tables
    conn.close()


def test_get_tracked_assets(memory_db):
    """Test get_tracked_assets returns correct assets."""
    cursor = memory_db.cursor()
    cursor.executescript(SCHEMA)
    memory_db.commit()

    assets = get_tracked_assets(memory_db)

    assert len(assets) >= 2
    # get_tracked_assets returns rows with 'symbol' key
    symbols = [a["symbol"] if isinstance(a, dict) else a[0] for a in assets]
    assert "bitcoin" in symbols
    assert "ethereum" in symbols
