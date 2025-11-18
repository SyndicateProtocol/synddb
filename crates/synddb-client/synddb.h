/**
 * SyndDB Client Library - C Header
 *
 * Cross-language FFI interface for SyndDB client.
 * Can be used from any language with C FFI support.
 */

#ifndef SYNDDB_H
#define SYNDDB_H

#ifdef __cplusplus
extern "C" {
#endif

#include <stdint.h>
#include <sqlite3.h>

/**
 * Opaque handle to SyndDB instance
 */
typedef struct synddb_handle synddb_handle;

/**
 * Attach SyndDB to an existing SQLite connection
 *
 * @param conn SQLite connection handle
 * @param sequencer_url URL of the sequencer (e.g. "https://sequencer:8433")
 * @return Handle to SyndDB instance, or NULL on error
 *
 * Example:
 *   sqlite3* conn;
 *   sqlite3_open("app.db", &conn);
 *   synddb_handle* synddb = synddb_attach(conn, "https://sequencer:8433");
 */
synddb_handle* synddb_attach(
    sqlite3* conn,
    const char* sequencer_url
);

/**
 * Detach SyndDB and flush pending changesets
 *
 * @param handle Handle returned from synddb_attach
 */
void synddb_detach(synddb_handle* handle);

/**
 * Get last error message (thread-local)
 *
 * @return Error message string, or NULL if no error
 */
const char* synddb_last_error(void);

/**
 * Free error string returned by synddb_last_error
 *
 * @param error Error string to free
 */
void synddb_free_error(char* error);

#ifdef __cplusplus
}
#endif

#endif /* SYNDDB_H */
