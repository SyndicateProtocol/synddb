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
    SYNDDB_SNAPSHOT_ERROR = 6
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
 * @param snapshot_interval Number of changesets between snapshots (must be > 0)
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
 * Publish all pending changesets to the sequencer
 *
 * Call this after committing transactions to send changesets to the sequencer.
 * Also called automatically on detach for graceful shutdown.
 *
 * @param handle SyndDB handle from synddb_attach()
 * @return SYNDDB_SUCCESS on success, error code otherwise
 */
SyndDBError synddb_publish(SyndDBHandle* handle);

/**
 * Create and publish a snapshot to the sequencer
 *
 * Creates a complete database snapshot (schema + data) and sends it to the
 * sequencer. Use this after schema changes (CREATE TABLE, etc.) since DDL
 * is NOT captured in changesets.
 *
 * This is consistent with synddb_publish() for changesets:
 * - synddb_publish() - sends pending changesets to sequencer
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
 * Detach SyndDB and free resources
 *
 * This gracefully shuts down the client, publishing any pending changesets.
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
