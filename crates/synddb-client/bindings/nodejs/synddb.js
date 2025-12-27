/**
 * SyndDB Node.js Client - Pure JavaScript FFI wrapper using koffi (no compilation needed!)
 *
 * Usage:
 *   const { SyndDB } = require('./synddb');
 *
 *   // Attach to database file
 *   const synddb = SyndDB.attach('app.db', 'http://localhost:8433');
 *
 *   // Use SQLite normally - changesets are automatically captured
 *   const Database = require('better-sqlite3');
 *   const db = new Database('app.db');
 *   db.prepare("INSERT INTO trades VALUES (?, ?)").run(1, 100);
 *
 *   // Optionally force immediate send (auto-sends every second)
 *   synddb.push();
 *
 *   // Create a snapshot (optional)
 *   const size = synddb.snapshot();
 *
 *   // Clean up (or let Node.js garbage collector handle it)
 *   synddb.detach();
 *
 * Installation:
 *   npm install koffi
 */

const koffi = require('koffi');
const path = require('path');
const fs = require('fs');
const os = require('os');

// Error codes (must match Rust enum)
const SyndDBError = {
  SUCCESS: 0,
  INVALID_POINTER: 1,
  INVALID_UTF8: 2,
  DATABASE_ERROR: 3,
  ATTACH_ERROR: 4,
  PUBLISH_ERROR: 5,
  SNAPSHOT_ERROR: 6,
  INVALID_URL: 7,
};

// Find libsynddb shared library
function findLibrary() {
  const platform = os.platform();
  const arch = os.arch();
  let libName;

  if (platform === 'darwin') {
    libName = 'libsynddb_client.dylib';
  } else if (platform === 'win32') {
    libName = 'synddb_client.dll';
  } else {
    libName = 'libsynddb_client.so';
  }

  // Determine platform directory for pre-built libs
  let libDir;
  if (platform === 'darwin') {
    libDir = arch === 'arm64' ? 'darwin-arm64' : 'darwin-x64';
  } else if (platform === 'linux') {
    libDir = 'linux-x64';
  } else if (platform === 'win32') {
    libDir = 'win-x64';
  }

  // Try common locations
  const searchPaths = [
    // Environment variable
    process.env.LIBSYNDDB_PATH,
    // Current directory
    path.join('.', libName),
    // Pre-built libs directory
    libDir ? path.join(__dirname, '..', '..', 'libs', libDir, libName) : null,
    // Relative to this file (target/release)
    path.join(__dirname, '..', '..', '..', '..', 'target', 'release', libName),
    path.join(__dirname, '..', '..', '..', '..', 'target', 'debug', libName),
    // System paths
    path.join('/usr', 'local', 'lib', libName),
    path.join('/usr', 'lib', libName),
  ].filter(Boolean);

  for (const libPath of searchPaths) {
    if (fs.existsSync(libPath)) {
      return libPath;
    }
  }

  throw new Error(
    `libsynddb_client not found. Build with: cargo build --package synddb-client --features ffi --release`
  );
}

// Load library
const lib = koffi.load(findLibrary());

// Define opaque pointer types
const SyndDBHandle = koffi.opaque('SyndDBHandle');
const SyndDBHandlePtr = koffi.pointer(SyndDBHandle);
const SyndDBHandlePtrPtr = koffi.out(koffi.pointer(SyndDBHandlePtr));

// Define FFI functions
const ffi = {
  synddb_version: lib.func('synddb_version', 'str', []),
  synddb_last_error: lib.func('synddb_last_error', 'str', []),
  synddb_attach: lib.func('synddb_attach', 'int', ['str', 'str', SyndDBHandlePtrPtr]),
  synddb_attach_with_config: lib.func('synddb_attach_with_config', 'int', [
    'str',      // db_path
    'str',      // sequencer_url
    'uint64',   // flush_interval_ms
    'uint64',   // snapshot_interval
    SyndDBHandlePtrPtr  // out_handle
  ]),
  synddb_push: lib.func('synddb_push', 'int', [SyndDBHandlePtr]),
  synddb_snapshot: lib.func('synddb_snapshot', 'int', [
    SyndDBHandlePtr,
    koffi.out(koffi.pointer('size_t'))
  ]),
  synddb_detach: lib.func('synddb_detach', 'void', [SyndDBHandlePtr]),
  synddb_execute: lib.func('synddb_execute', 'int64', [SyndDBHandlePtr, 'str']),
  synddb_execute_batch: lib.func('synddb_execute_batch', 'int', [SyndDBHandlePtr, 'str']),
  synddb_begin: lib.func('synddb_begin', 'int', [SyndDBHandlePtr]),
  synddb_commit: lib.func('synddb_commit', 'int', [SyndDBHandlePtr]),
  synddb_rollback: lib.func('synddb_rollback', 'int', [SyndDBHandlePtr]),
};

/**
 * SyndDB client handle - automatically captures and sends SQLite changesets
 */
class SyndDB {
  constructor(handle) {
    this._handle = handle;
  }

  /**
   * Attach SyndDB to a SQLite database file
   *
   * @param {string} dbPath - Path to SQLite database file
   * @param {string} sequencerUrl - URL of sequencer TEE (e.g., 'http://localhost:8433')
   * @returns {SyndDB} SyndDB instance
   * @throws {Error} If attachment fails
   *
   * @example
   * const synddb = SyndDB.attach('app.db', 'http://localhost:8433');
   *
   * // Now use SQLite normally
   * const Database = require('better-sqlite3');
   * const db = new Database('app.db');
   * db.prepare("INSERT INTO users VALUES (?, ?)").run(1, 'Alice');
   */
  static attach(dbPath, sequencerUrl) {
    const handlePtr = [null];
    const result = ffi.synddb_attach(dbPath, sequencerUrl, handlePtr);

    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = ffi.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to attach SyndDB (error ${result}): ${errorMsg}`);
    }

    return new SyndDB(handlePtr[0]);
  }

  /**
   * Attach SyndDB with custom configuration
   *
   * @param {string} dbPath - Path to SQLite database file
   * @param {string} sequencerUrl - URL of sequencer TEE
   * @param {Object} options - Configuration options
   * @param {number} options.flushIntervalMs - Milliseconds between sender flushes (default: 1000)
   * @param {number} options.snapshotInterval - Changesets between snapshots (default: 0 = disabled)
   * @returns {SyndDB} SyndDB instance
   *
   * @example
   * const synddb = SyndDB.attachWithConfig(
   *   'app.db',
   *   'http://localhost:8433',
   *   {
   *     flushIntervalMs: 500,   // Flush sender every 500ms
   *     snapshotInterval: 100   // Snapshot every 100 changesets
   *   }
   * );
   */
  static attachWithConfig(dbPath, sequencerUrl, options = {}) {
    const flushIntervalMs = options.flushIntervalMs || 1000;
    const snapshotInterval = options.snapshotInterval || 0;

    const handlePtr = [null];
    const result = ffi.synddb_attach_with_config(
      dbPath,
      sequencerUrl,
      flushIntervalMs,
      snapshotInterval,
      handlePtr
    );

    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = ffi.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to attach SyndDB (error ${result}): ${errorMsg}`);
    }

    return new SyndDB(handlePtr[0]);
  }

  /**
   * Push all pending changesets to the sequencer
   *
   * Call this after committing transactions to send changesets to the sequencer.
   * Also called automatically on detach for graceful shutdown.
   *
   * @throws {Error} If push fails
   *
   * @example
   * synddb.push();
   */
  push() {
    if (!this._handle) {
      throw new Error('SyndDB handle already detached');
    }

    const result = ffi.synddb_push(this._handle);

    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = ffi.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to send changeset (error ${result}): ${errorMsg}`);
    }
  }

  /**
   * Create and publish a snapshot to the sequencer.
   *
   * This creates a complete database snapshot (schema + data) and sends it
   * to the sequencer. Use this after schema changes (CREATE TABLE, etc.)
   * since DDL is NOT captured in changesets.
   *
   * When to use:
   * - After CREATE TABLE, ALTER TABLE, or other DDL statements
   * - To create periodic recovery checkpoints
   * - Before major migrations
   *
   * @returns {number} Size of snapshot in bytes
   * @throws {Error} If snapshot creation or publishing fails
   *
   * @example
   * // After creating schema
   * db.exec('CREATE TABLE users (id INTEGER PRIMARY KEY)');
   * const size = synddb.snapshot();  // Creates AND publishes
   * console.log(`Published snapshot: ${size} bytes`);
   */
  snapshot() {
    if (!this._handle) {
      throw new Error('SyndDB handle already detached');
    }

    const sizePtr = [0];
    const result = ffi.synddb_snapshot(this._handle, sizePtr);

    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = ffi.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to create snapshot (error ${result}): ${errorMsg}`);
    }

    return sizePtr[0];
  }

  /**
   * Execute a single SQL statement
   *
   * Changes made through this function are captured and published to the sequencer.
   *
   * @param {string} sql - SQL statement to execute
   * @returns {number} Number of rows affected
   * @throws {Error} If execution fails
   *
   * @example
   * const rows = synddb.execute("INSERT INTO users (name) VALUES ('Alice')");
   */
  execute(sql) {
    if (!this._handle) {
      throw new Error('SyndDB handle already detached');
    }

    const rows = ffi.synddb_execute(this._handle, sql);
    if (rows < 0) {
      const errorMsg = ffi.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to execute SQL: ${errorMsg}`);
    }
    return Number(rows);
  }

  /**
   * Execute multiple SQL statements (batch)
   *
   * This is useful for executing schema creation or multiple statements at once.
   * If DDL statements are detected, a snapshot is automatically published.
   *
   * @param {string} sql - SQL statements to execute (semicolon-separated)
   * @throws {Error} If execution fails
   *
   * @example
   * synddb.executeBatch(`
   *   CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT);
   *   CREATE INDEX IF NOT EXISTS idx_name ON users(name);
   * `);
   */
  executeBatch(sql) {
    if (!this._handle) {
      throw new Error('SyndDB handle already detached');
    }

    const result = ffi.synddb_execute_batch(this._handle, sql);
    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = ffi.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to execute batch (error ${result}): ${errorMsg}`);
    }
  }

  /**
   * Begin a transaction
   *
   * @throws {Error} If transaction start fails
   *
   * @example
   * synddb.begin();
   * try {
   *   synddb.execute("INSERT INTO users (name) VALUES ('Alice')");
   *   synddb.commit();
   * } catch (e) {
   *   synddb.rollback();
   *   throw e;
   * }
   */
  begin() {
    if (!this._handle) {
      throw new Error('SyndDB handle already detached');
    }

    const result = ffi.synddb_begin(this._handle);
    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = ffi.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to begin transaction (error ${result}): ${errorMsg}`);
    }
  }

  /**
   * Commit the current transaction
   *
   * @throws {Error} If commit fails
   */
  commit() {
    if (!this._handle) {
      throw new Error('SyndDB handle already detached');
    }

    const result = ffi.synddb_commit(this._handle);
    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = ffi.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to commit transaction (error ${result}): ${errorMsg}`);
    }
  }

  /**
   * Rollback the current transaction
   *
   * @throws {Error} If rollback fails
   */
  rollback() {
    if (!this._handle) {
      throw new Error('SyndDB handle already detached');
    }

    const result = ffi.synddb_rollback(this._handle);
    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = ffi.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to rollback transaction (error ${result}): ${errorMsg}`);
    }
  }

  /**
   * Detach SyndDB and free resources
   *
   * This gracefully shuts down the client, sending any pending changesets.
   * The instance cannot be used after this call.
   *
   * @example
   * synddb.detach();
   */
  detach() {
    if (this._handle) {
      ffi.synddb_detach(this._handle);
      this._handle = null;
    }
  }

  /**
   * Automatic cleanup (Node.js Disposable pattern)
   */
  [Symbol.dispose]() {
    this.detach();
  }
}

/**
 * Get library version string
 *
 * @returns {string} Version string (e.g., "0.1.0")
 *
 * @example
 * const synddb = require('./synddb');
 * console.log(synddb.version());
 */
function version() {
  return ffi.synddb_version();
}

/**
 * Get last error message
 *
 * @returns {string|null} Error message string, or null if no error
 */
function lastError() {
  return ffi.synddb_last_error();
}

/**
 * Convenience function to attach SyndDB
 *
 * @param {string} dbPath - Path to SQLite database file
 * @param {string} sequencerUrl - URL of sequencer TEE
 * @param {Object} options - Optional config (flushIntervalMs, snapshotInterval)
 * @returns {SyndDB} SyndDB instance
 *
 * @example
 * const { attach } = require('./synddb');
 * const synddb = attach('app.db', 'http://localhost:8433');
 */
function attach(dbPath, sequencerUrl, options) {
  if (options) {
    return SyndDB.attachWithConfig(dbPath, sequencerUrl, options);
  } else {
    return SyndDB.attach(dbPath, sequencerUrl);
  }
}

module.exports = {
  SyndDB,
  attach,
  version,
  lastError,
  SyndDBError,
};
