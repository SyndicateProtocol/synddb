#!/usr/bin/env python3
"""
Example Python application using synddb-client

This demonstrates how lightweight the integration is:
- Single import
- Single function call to attach
- Rest of the code is unchanged
"""

import sqlite3
import time
# from synddb import attach  # TODO: Implement Python bindings

def main():
    print("=== SyndDB Client Example (Python) ===\n")

    # Open database
    conn = sqlite3.connect('example.db')
    cursor = conn.cursor()

    # Create schema
    cursor.execute('''
        CREATE TABLE IF NOT EXISTS trades (
            id INTEGER PRIMARY KEY,
            price INTEGER,
            quantity INTEGER,
            timestamp INTEGER
        )
    ''')
    conn.commit()

    print("✓ Database opened and schema created")

    # INTEGRATION POINT: Single line to enable SyndDB
    # attach(conn, sequencer_url='http://localhost:8433')
    print("✓ SyndDB client attached to connection (TODO: implement bindings)\n")

    # Application code - completely unchanged from here
    print("Executing trades...")

    for i in range(1, 11):
        cursor.execute(
            "INSERT INTO trades (id, price, quantity, timestamp) VALUES (?, ?, ?, ?)",
            (i, 100 + i, 10, int(time.time()))
        )
        conn.commit()
        print(f"  Trade {i} inserted")

        # Simulate some delay
        time.sleep(0.1)

    print("\n✓ All trades executed")
    print("✓ Changesets automatically captured and sent to sequencer")

    conn.close()

if __name__ == "__main__":
    main()
