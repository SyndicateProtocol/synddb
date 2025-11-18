/**
 * SyndDB Node.js Client - Pure JavaScript FFI wrapper
 *
 * No compilation needed! Just npm install ffi-napi and use.
 *
 * Usage:
 *   const Database = require('better-sqlite3');
 *   const { attach } = require('./synddb');
 *
 *   const db = new Database('app.db');
 *   attach(db, { sequencerUrl: 'https://sequencer:8433' });
 *
 *   // Use SQLite normally
 *   db.prepare("INSERT INTO trades ...").run();
 */

const ffi = require('ffi-napi');
const ref = require('ref-napi');
const path = require('path');

// Find libsynddb
const libPath = process.env.LIBSYNDDB_PATH ||
                path.join(__dirname, '../../target/release/libsynddb.so');

// Load library
const lib = ffi.Library(libPath, {
  'synddb_attach': ['pointer', ['pointer', 'string']],
  'synddb_detach': ['void', ['pointer']],
  'synddb_last_error': ['string', []],
});

/**
 * SyndDB handle wrapper
 */
class SyndDBHandle {
  constructor(handle) {
    this._handle = handle;
  }

  /**
   * Detach and flush pending changesets
   */
  detach() {
    if (this._handle) {
      lib.synddb_detach(this._handle);
      this._handle = null;
    }
  }

  /**
   * Automatic cleanup
   */
  [Symbol.dispose]() {
    this.detach();
  }
}

/**
 * Attach SyndDB to better-sqlite3 database
 *
 * @param {Database} db - better-sqlite3 Database instance
 * @param {Object} options - Configuration options
 * @param {string} options.sequencerUrl - URL of sequencer
 * @returns {SyndDBHandle} Handle to SyndDB instance
 *
 * @example
 * const Database = require('better-sqlite3');
 * const { attach } = require('./synddb');
 *
 * const db = new Database('app.db');
 * const synddb = attach(db, { sequencerUrl: 'https://sequencer:8433' });
 *
 * // Use SQLite normally
 * db.prepare("INSERT INTO trades VALUES (?, ?)").run(1, 100);
 */
function attach(db, options) {
  const { sequencerUrl } = options;

  if (!sequencerUrl) {
    throw new Error('sequencerUrl is required');
  }

  // Get raw sqlite3* pointer from better-sqlite3
  // This is implementation-dependent
  const connPtr = db.handle || db._handle;

  if (!connPtr) {
    throw new Error('Unable to get sqlite3 handle from database');
  }

  // Call FFI
  const handle = lib.synddb_attach(connPtr, sequencerUrl);

  if (handle.isNull()) {
    const error = lib.synddb_last_error();
    throw new Error(`Failed to attach SyndDB: ${error || 'Unknown error'}`);
  }

  return new SyndDBHandle(handle);
}

/**
 * Get last error message
 * @returns {string|null} Error message or null
 */
function lastError() {
  return lib.synddb_last_error();
}

module.exports = {
  attach,
  lastError,
  SyndDBHandle,
};
