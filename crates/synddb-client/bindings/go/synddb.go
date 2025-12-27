// Package synddb provides Go bindings for SyndDB client library
//
// Usage:
//
//	import "github.com/syndicate/synddb-go"
//
//	// Attach to database file
//	handle, err := synddb.Attach("/path/to/app.db", "http://sequencer:8433")
//	if err != nil {
//	    log.Fatal(err)
//	}
//	defer handle.Detach()
//
//	// Execute SQL - changes are captured and published
//	rows, err := handle.Execute("INSERT INTO trades VALUES (1, 100)")
//	if err != nil {
//	    log.Fatal(err)
//	}
//
//	// Create snapshot after schema changes
//	size, err := handle.Snapshot()
package synddb

/*
#cgo LDFLAGS: -lsynddb_client
#include <stdlib.h>
#include <stdint.h>

// Opaque handle type
typedef struct SyndDBHandle SyndDBHandle;

// Error codes
typedef enum {
    SyndDBSuccess = 0,
    SyndDBInvalidPointer = 1,
    SyndDBInvalidUtf8 = 2,
    SyndDBDatabaseError = 3,
    SyndDBAttachError = 4,
    SyndDBPublishError = 5,
    SyndDBSnapshotError = 6,
    SyndDBInvalidUrl = 7
} SyndDBError;

// FFI function declarations
extern const char* synddb_version(void);
extern const char* synddb_last_error(void);
extern SyndDBError synddb_attach(const char* db_path, const char* sequencer_url, SyndDBHandle** out_handle);
extern SyndDBError synddb_attach_with_config(const char* db_path, const char* sequencer_url, uint64_t flush_interval_ms, uint64_t snapshot_interval, SyndDBHandle** out_handle);
extern SyndDBError synddb_publish_changeset(SyndDBHandle* handle);
extern SyndDBError synddb_publish_snapshot(SyndDBHandle* handle, size_t* out_size);
extern void synddb_detach(SyndDBHandle* handle);
extern int64_t synddb_execute(SyndDBHandle* handle, const char* sql);
extern SyndDBError synddb_execute_batch(SyndDBHandle* handle, const char* sql);
extern SyndDBError synddb_begin(SyndDBHandle* handle);
extern SyndDBError synddb_commit(SyndDBHandle* handle);
extern SyndDBError synddb_rollback(SyndDBHandle* handle);
*/
import "C"
import (
	"errors"
	"fmt"
	"unsafe"
)

// Handle represents a SyndDB client instance attached to a database
type Handle struct {
	handle *C.SyndDBHandle
}

// Config holds configuration options for SyndDB attachment
type Config struct {
	// FlushIntervalMs is the milliseconds between automatic changeset flushes (default: 1000)
	FlushIntervalMs uint64
	// SnapshotInterval is the number of changesets between automatic snapshots (0 = disabled)
	SnapshotInterval uint64
}

// DefaultConfig returns the default configuration
func DefaultConfig() Config {
	return Config{
		FlushIntervalMs:  1000,
		SnapshotInterval: 0,
	}
}

// Version returns the library version string
func Version() string {
	return C.GoString(C.synddb_version())
}

// LastError returns the last error message from the library
func LastError() string {
	errMsg := C.synddb_last_error()
	if errMsg == nil {
		return ""
	}
	return C.GoString(errMsg)
}

// getError creates an error from the last error message
func getError(code C.SyndDBError, context string) error {
	errMsg := LastError()
	if errMsg != "" {
		return fmt.Errorf("%s: %s", context, errMsg)
	}
	return fmt.Errorf("%s: error code %d", context, int(code))
}

// Attach connects SyndDB to a SQLite database file
//
// The database file will be created if it doesn't exist.
// Changes made through the Handle's methods are captured and published
// to the sequencer.
//
// Example:
//
//	handle, err := synddb.Attach("/tmp/app.db", "http://localhost:8433")
//	if err != nil {
//	    log.Fatal(err)
//	}
//	defer handle.Detach()
func Attach(dbPath, sequencerURL string) (*Handle, error) {
	dbPathC := C.CString(dbPath)
	defer C.free(unsafe.Pointer(dbPathC))

	urlC := C.CString(sequencerURL)
	defer C.free(unsafe.Pointer(urlC))

	var handle *C.SyndDBHandle
	result := C.synddb_attach(dbPathC, urlC, &handle)
	if result != C.SyndDBSuccess {
		return nil, getError(result, "failed to attach")
	}

	return &Handle{handle: handle}, nil
}

// AttachWithConfig connects SyndDB with custom configuration
//
// Example:
//
//	config := synddb.Config{
//	    FlushIntervalMs: 500,
//	    SnapshotInterval: 100,
//	}
//	handle, err := synddb.AttachWithConfig("/tmp/app.db", "http://localhost:8433", config)
func AttachWithConfig(dbPath, sequencerURL string, config Config) (*Handle, error) {
	dbPathC := C.CString(dbPath)
	defer C.free(unsafe.Pointer(dbPathC))

	urlC := C.CString(sequencerURL)
	defer C.free(unsafe.Pointer(urlC))

	var handle *C.SyndDBHandle
	result := C.synddb_attach_with_config(
		dbPathC,
		urlC,
		C.uint64_t(config.FlushIntervalMs),
		C.uint64_t(config.SnapshotInterval),
		&handle,
	)
	if result != C.SyndDBSuccess {
		return nil, getError(result, "failed to attach with config")
	}

	return &Handle{handle: handle}, nil
}

// Detach disconnects from SyndDB and frees resources
//
// This gracefully shuts down the client, publishing any pending changesets.
// The Handle must not be used after calling Detach.
func (h *Handle) Detach() {
	if h.handle != nil {
		C.synddb_detach(h.handle)
		h.handle = nil
	}
}

// Publish forces immediate publication of all pending changesets
//
// Changesets are automatically published on a timer. Use this to force
// immediate publication for low-latency or high-value changes.
func (h *Handle) Publish() error {
	if h.handle == nil {
		return errors.New("handle is nil or already detached")
	}

	result := C.synddb_publish_changeset(h.handle)
	if result != C.SyndDBSuccess {
		return getError(result, "failed to publish")
	}
	return nil
}

// Snapshot creates and publishes a database snapshot
//
// This creates a complete database snapshot (schema + data) and sends it
// to the sequencer. Use this after schema changes (CREATE TABLE, etc.)
// since DDL is NOT captured in changesets.
//
// Returns the size of the snapshot in bytes.
func (h *Handle) Snapshot() (int, error) {
	if h.handle == nil {
		return 0, errors.New("handle is nil or already detached")
	}

	var size C.size_t
	result := C.synddb_publish_snapshot(h.handle, &size)
	if result != C.SyndDBSuccess {
		return 0, getError(result, "failed to create snapshot")
	}
	return int(size), nil
}

// Execute runs a single SQL statement and returns the number of affected rows
//
// Changes made through this function are captured and published to the sequencer.
//
// Example:
//
//	rows, err := handle.Execute("INSERT INTO users (name) VALUES ('Alice')")
func (h *Handle) Execute(sql string) (int64, error) {
	if h.handle == nil {
		return 0, errors.New("handle is nil or already detached")
	}

	sqlC := C.CString(sql)
	defer C.free(unsafe.Pointer(sqlC))

	rows := C.synddb_execute(h.handle, sqlC)
	if rows < 0 {
		return 0, errors.New(LastError())
	}
	return int64(rows), nil
}

// ExecuteBatch runs multiple SQL statements separated by semicolons
//
// This is useful for executing schema creation or multiple statements at once.
// If DDL statements are detected, a snapshot is automatically published.
//
// Example:
//
//	err := handle.ExecuteBatch(`
//	    CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT);
//	    CREATE INDEX IF NOT EXISTS idx_name ON users(name);
//	`)
func (h *Handle) ExecuteBatch(sql string) error {
	if h.handle == nil {
		return errors.New("handle is nil or already detached")
	}

	sqlC := C.CString(sql)
	defer C.free(unsafe.Pointer(sqlC))

	result := C.synddb_execute_batch(h.handle, sqlC)
	if result != C.SyndDBSuccess {
		return getError(result, "failed to execute batch")
	}
	return nil
}

// Begin starts a new transaction
func (h *Handle) Begin() error {
	if h.handle == nil {
		return errors.New("handle is nil or already detached")
	}

	result := C.synddb_begin(h.handle)
	if result != C.SyndDBSuccess {
		return getError(result, "failed to begin transaction")
	}
	return nil
}

// Commit commits the current transaction
func (h *Handle) Commit() error {
	if h.handle == nil {
		return errors.New("handle is nil or already detached")
	}

	result := C.synddb_commit(h.handle)
	if result != C.SyndDBSuccess {
		return getError(result, "failed to commit transaction")
	}
	return nil
}

// Rollback rolls back the current transaction
func (h *Handle) Rollback() error {
	if h.handle == nil {
		return errors.New("handle is nil or already detached")
	}

	result := C.synddb_rollback(h.handle)
	if result != C.SyndDBSuccess {
		return getError(result, "failed to rollback transaction")
	}
	return nil
}
