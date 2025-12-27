/**
 * SyndDB Client - C FFI Header
 *
 * Lightweight client for sending SQLite changesets to SyndDB sequencer.
 * This header can be used from C, C++, or any language with C FFI support.
 */

#ifndef SYNDDB_H
#define SYNDDB_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Opaque handle to SyndDB instance */
typedef struct SyndDBHandle SyndDBHandle;

/* Error codes */
typedef enum {
    SYNDDB_SUCCESS = 0,
    SYNDDB_INVALID_POINTER = 1,
    SYNDDB_INVALID_UTF8 = 2,
    SYNDDB_DATABASE_ERROR = 3,
    SYNDDB_ATTACH_ERROR = 4,
    SYNDDB_PUBLISH_ERROR = 5,
    SYNDDB_SNAPSHOT_ERROR = 6,
    SYNDDB_INVALID_URL = 7
} SyndDBError;

/**
 * Attach SyndDB to a SQLite database file
 *
 * @param db_path Path to SQLite database file (UTF-8, null-terminated)
 * @param sequencer_url URL of sequencer TEE (UTF-8, null-terminated)
 * @param out_handle Output pointer to receive SyndDB handle
 * @return SYNDDB_SUCCESS on success, error code otherwise
 *
 * Example:
 *   SyndDBHandle* handle;
 *   int result = synddb_attach("app.db", "http://localhost:8433", &handle);
 *   if (result != SYNDDB_SUCCESS) {
 *       fprintf(stderr, "Failed to attach SyndDB\n");
 *   }
 */
SyndDBError synddb_attach(
    const char* db_path,
    const char* sequencer_url,
    SyndDBHandle** out_handle
);

/**
 * Attach SyndDB with custom configuration
 *
 * @param db_path Path to SQLite database file
 * @param sequencer_url URL of sequencer TEE
 * @param flush_interval_ms Milliseconds between sender flushes (must be > 0)
 * @param snapshot_interval Number of changesets between snapshots (0 = disabled)
 * @param out_handle Output pointer to receive SyndDB handle
 * @return SYNDDB_SUCCESS on success, error code otherwise
 */
SyndDBError synddb_attach_with_config(
    const char* db_path,
    const char* sequencer_url,
    uint64_t flush_interval_ms,
    uint64_t snapshot_interval,
    SyndDBHandle** out_handle
);

/**
 * Push all pending changesets to the sequencer
 *
 * Call this after committing transactions to send changesets to the sequencer.
 * Also called automatically on detach for graceful shutdown.
 *
 * @param handle SyndDB handle from synddb_attach()
 * @return SYNDDB_SUCCESS on success, error code otherwise
 */
SyndDBError synddb_push(SyndDBHandle* handle);

/**
 * Create and publish a snapshot to the sequencer
 *
 * Creates a complete database snapshot (schema + data) and sends it to the
 * sequencer. Use this after schema changes (CREATE TABLE, etc.) since DDL
 * is NOT captured in changesets.
 *
 * This is consistent with synddb_push() for changesets:
 * - synddb_push() - sends pending changesets to sequencer
 * - synddb_snapshot() - creates and sends snapshot to sequencer
 *
 * When to use:
 * - After CREATE TABLE, ALTER TABLE, or other DDL statements
 * - To create periodic recovery checkpoints
 * - Before major migrations
 *
 * @param handle SyndDB handle from synddb_attach()
 * @param out_size Output pointer to receive snapshot size (optional, can be NULL)
 * @return SYNDDB_SUCCESS on success, error code otherwise
 *
 * Example:
 *   synddb_snapshot(handle, NULL);  // Creates AND publishes
 */
SyndDBError synddb_snapshot(SyndDBHandle* handle, size_t* out_size);

/**
 * Execute a single SQL statement
 *
 * Changes made through this function are captured and published to the sequencer.
 *
 * @param handle SyndDB handle from synddb_attach()
 * @param sql SQL statement to execute (UTF-8, null-terminated)
 * @return Number of rows affected on success, -1 on error
 *
 * Example:
 *   int64_t rows = synddb_execute(handle, "INSERT INTO users VALUES (1, 'Alice')");
 *   if (rows < 0) {
 *       printf("Error: %s\n", synddb_last_error());
 *   }
 */
int64_t synddb_execute(SyndDBHandle* handle, const char* sql);

/**
 * Execute multiple SQL statements (batch)
 *
 * This is useful for executing schema creation or multiple statements at once.
 * If DDL statements are detected, a snapshot is automatically published.
 *
 * @param handle SyndDB handle from synddb_attach()
 * @param sql SQL statements to execute (UTF-8, null-terminated, semicolon-separated)
 * @return SYNDDB_SUCCESS on success, error code otherwise
 *
 * Example:
 *   synddb_execute_batch(handle,
 *       "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT);"
 *       "CREATE INDEX IF NOT EXISTS idx_name ON users(name);");
 */
SyndDBError synddb_execute_batch(SyndDBHandle* handle, const char* sql);

/**
 * Begin a transaction
 *
 * @param handle SyndDB handle from synddb_attach()
 * @return SYNDDB_SUCCESS on success, error code otherwise
 */
SyndDBError synddb_begin(SyndDBHandle* handle);

/**
 * Commit the current transaction
 *
 * @param handle SyndDB handle from synddb_attach()
 * @return SYNDDB_SUCCESS on success, error code otherwise
 */
SyndDBError synddb_commit(SyndDBHandle* handle);

/**
 * Rollback the current transaction
 *
 * @param handle SyndDB handle from synddb_attach()
 * @return SYNDDB_SUCCESS on success, error code otherwise
 */
SyndDBError synddb_rollback(SyndDBHandle* handle);

/**
 * Detach SyndDB and free resources
 *
 * This gracefully shuts down the client, sending any pending changesets.
 * The handle must not be used after this call.
 *
 * @param handle SyndDB handle from synddb_attach()
 */
void synddb_detach(SyndDBHandle* handle);

/**
 * Get error message for the last error
 *
 * @return Pointer to null-terminated UTF-8 string (valid until next FFI call)
 */
const char* synddb_last_error(void);

/**
 * Get library version string
 *
 * @return Pointer to null-terminated UTF-8 string (e.g., "0.1.0")
 */
const char* synddb_version(void);

#ifdef __cplusplus
}
#endif

#endif /* SYNDDB_H */
