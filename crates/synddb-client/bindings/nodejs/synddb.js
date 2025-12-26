/**
 * SyndDB Node.js Client - Pure JavaScript FFI wrapper (no compilation needed!)
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
 *   // IMPORTANT: Call publish() after commits to send changesets
 *   synddb.publish();
 *
 *   // Create a snapshot (optional)
 *   const size = synddb.snapshot();
 *
 *   // Clean up (or let Node.js garbage collector handle it)
 *   synddb.detach();
 */

const ffi = require('ffi-napi');
const ref = require('ref-napi');
const path = require('path');
const os = require('os');

// Define types
const VoidPtr = ref.refType(ref.types.void);
const SizeT = ref.types.size_t;
const SizeTPtr = ref.refType(SizeT);

// Error codes (must match Rust enum)
const SyndDBError = {
  SUCCESS: 0,
  INVALID_POINTER: 1,
  INVALID_UTF8: 2,
  DATABASE_ERROR: 3,
  ATTACH_ERROR: 4,
  PUBLISH_ERROR: 5,
  SNAPSHOT_ERROR: 6,
};

// Find libsynddb shared library
function findLibrary() {
  const platform = os.platform();
  let libName;

  if (platform === 'darwin') {
    libName = 'libsynddb_client.dylib';
  } else if (platform === 'win32') {
    libName = 'synddb_client.dll';
  } else {
    libName = 'libsynddb_client.so';
  }

  // Try common locations
  const searchPaths = [
    // Environment variable
    process.env.LIBSYNDDB_PATH,
    // Current directory
    path.join('.', libName),
    // Relative to this file
    path.join(__dirname, '..', '..', 'target', 'release', libName),
    path.join(__dirname, '..', '..', 'target', 'debug', libName),
    // System paths
    path.join('/usr', 'local', 'lib', libName),
    path.join('/usr', 'lib', libName),
  ].filter(Boolean);

  const fs = require('fs');
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
const lib = ffi.Library(findLibrary(), {
  synddb_attach: ['int', ['string', 'string', VoidPtr]],
  synddb_attach_with_config: ['int', ['string', 'string', 'uint64', 'uint64', VoidPtr]],
  synddb_publish: ['int', [VoidPtr]],
  synddb_snapshot: ['int', [VoidPtr, SizeTPtr]],
  synddb_detach: ['void', [VoidPtr]],
  synddb_last_error: ['string', []],
  synddb_version: ['string', []],
});

/**
 * SyndDB client handle - automatically captures and publishes SQLite changesets
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
    const handlePtr = ref.alloc(VoidPtr);
    const result = lib.synddb_attach(dbPath, sequencerUrl, handlePtr);

    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = lib.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to attach SyndDB (error ${result}): ${errorMsg}`);
    }

    const handle = handlePtr.deref();
    return new SyndDB(handle);
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

    const handlePtr = ref.alloc(VoidPtr);
    const result = lib.synddb_attach_with_config(
      dbPath,
      sequencerUrl,
      flushIntervalMs,
      snapshotInterval,
      handlePtr
    );

    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = lib.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to attach SyndDB (error ${result}): ${errorMsg}`);
    }

    const handle = handlePtr.deref();
    return new SyndDB(handle);
  }

  /**
   * Publish all pending changesets to the sequencer
   *
   * Call this after committing transactions to send changesets to the sequencer.
   * Also called automatically on detach for graceful shutdown.
   *
   * @throws {Error} If publish fails
   *
   * @example
   * synddb.publish();
   */
  publish() {
    if (!this._handle) {
      throw new Error('SyndDB handle already detached');
    }

    const result = lib.synddb_publish(this._handle);

    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = lib.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to publish (error ${result}): ${errorMsg}`);
    }
  }

  /**
   * Create and publish a snapshot to the sequencer.
   *
   * This creates a complete database snapshot (schema + data) and sends it
   * to the sequencer. Use this after schema changes (CREATE TABLE, etc.)
   * since DDL is NOT captured in changesets.
   *
   * This is consistent with publish() for changesets:
   * - publish() - sends pending changesets to sequencer
   * - snapshot() - creates and sends snapshot to sequencer
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
   * synddb.executeBatch('CREATE TABLE users (id INTEGER PRIMARY KEY)');
   * const size = synddb.snapshot();  // Creates AND publishes
   * console.log(`Published snapshot: ${size} bytes`);
   */
  snapshot() {
    if (!this._handle) {
      throw new Error('SyndDB handle already detached');
    }

    const sizePtr = ref.alloc(SizeT);
    const result = lib.synddb_snapshot(this._handle, sizePtr);

    if (result !== SyndDBError.SUCCESS) {
      const errorMsg = lib.synddb_last_error() || 'Unknown error';
      throw new Error(`Failed to publish snapshot (error ${result}): ${errorMsg}`);
    }

    return sizePtr.deref();
  }

  /**
   * Detach SyndDB and free resources
   *
   * This gracefully shuts down the client, publishing any pending changesets.
   * The instance cannot be used after this call.
   *
   * @example
   * synddb.detach();
   */
  detach() {
    if (this._handle) {
      lib.synddb_detach(this._handle);
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
  return lib.synddb_version();
}

/**
 * Get last error message
 *
 * @returns {string|null} Error message string, or null if no error
 */
function lastError() {
  return lib.synddb_last_error();
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
