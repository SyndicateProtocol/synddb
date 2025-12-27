#!/usr/bin/env python3
"""
Example Python application using synddb-client

This example demonstrates the Python FFI bindings for SyndDB.

Prerequisites:
    1. Build the shared library:
       cargo build --package synddb-client --features ffi --release

    2. The library will be at:
       - macOS: target/release/libsynddb_client.dylib
       - Linux: target/release/libsynddb_client.so
       - Windows: target/release/synddb_client.dll

Usage:
    cd crates/synddb-client/bindings/python
    python synddb.py  # or import from your app

For the full Python API, see: crates/synddb-client/bindings/python/synddb.py
"""

import sys
import os

# Add bindings directory to path
bindings_path = os.path.join(os.path.dirname(__file__), '..', 'bindings', 'python')
sys.path.insert(0, bindings_path)

try:
    from synddb import SyndDB, version
    print(f"SyndDB Python bindings loaded (version {version()})")
except ImportError as e:
    print(f"Failed to load SyndDB bindings: {e}")
    print("\nTo build the shared library:")
    print("  cargo build --package synddb-client --features ffi --release")
    sys.exit(1)


def main():
    print("=== SyndDB Client Example (Python) ===\n")

    # Example usage (requires a running sequencer)
    sequencer_url = os.environ.get('SEQUENCER_URL', 'http://localhost:8433')
    db_path = 'example.db'

    print(f"Database: {db_path}")
    print(f"Sequencer: {sequencer_url}\n")

    try:
        # Attach SyndDB to a database file
        with SyndDB.attach(db_path, sequencer_url) as synddb:
            print("Connected to SyndDB\n")

            # Create schema
            synddb.execute_batch('''
                CREATE TABLE IF NOT EXISTS trades (
                    id INTEGER PRIMARY KEY,
                    price INTEGER,
                    quantity INTEGER,
                    timestamp INTEGER
                )
            ''')
            print("Schema created")

            # Publish snapshot after schema changes
            size = synddb.snapshot()
            print(f"Snapshot published ({size} bytes)\n")

            # Insert some data
            print("Executing trades...")
            for i in range(1, 6):
                rows = synddb.execute(
                    f"INSERT INTO trades (id, price, quantity, timestamp) "
                    f"VALUES ({i}, {100 + i}, 10, strftime('%s', 'now'))"
                )
                print(f"  Trade {i} inserted ({rows} row)")

            print("\nAll trades executed and changesets captured")

    except RuntimeError as e:
        print(f"Error: {e}")
        print("\nMake sure the sequencer is running:")
        print("  cargo run --package synddb-sequencer")
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
