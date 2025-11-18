// Package synddb provides Go bindings for SyndDB client library
//
// Usage:
//
//	import (
//	    "database/sql"
//	    _ "github.com/mattn/go-sqlite3"
//	    "github.com/syndicate/synddb-go"
//	)
//
//	db, _ := sql.Open("sqlite3", "app.db")
//	defer synddb.Attach(db, "https://sequencer:8433").Close()
//
//	// Use database normally
//	db.Exec("INSERT INTO trades VALUES (?, ?)", 1, 100)
package synddb

/*
#cgo LDFLAGS: -lsynddb
#include <stdlib.h>

typedef struct synddb_handle synddb_handle;

synddb_handle* synddb_attach(void* conn, const char* sequencer_url);
void synddb_detach(synddb_handle* handle);
const char* synddb_last_error(void);
*/
import "C"
import (
	"database/sql"
	"errors"
	"unsafe"
)

// Handle represents a SyndDB client instance
type Handle struct {
	handle *C.synddb_handle
}

// Attach attaches SyndDB to an existing SQLite connection
//
// The connection must be a *sql.DB using the sqlite3 driver.
// Returns a Handle that should be Close()'d when done.
//
// Example:
//
//	db, _ := sql.Open("sqlite3", "app.db")
//	synddb := synddb.Attach(db, "https://sequencer:8433")
//	defer synddb.Close()
//
//	// Use database normally
//	db.Exec("INSERT INTO trades VALUES (?, ?)", 1, 100)
func Attach(db *sql.DB, sequencerURL string) (*Handle, error) {
	// Get raw sqlite3* pointer from database/sql
	// This requires driver-specific code
	// For github.com/mattn/go-sqlite3, we need to extract the pointer

	// TODO: Platform-specific extraction of sqlite3*
	connPtr := unsafe.Pointer(uintptr(0))

	urlC := C.CString(sequencerURL)
	defer C.free(unsafe.Pointer(urlC))

	handle := C.synddb_attach(connPtr, urlC)
	if handle == nil {
		errMsg := C.synddb_last_error()
		if errMsg != nil {
			return nil, errors.New(C.GoString(errMsg))
		}
		return nil, errors.New("failed to attach SyndDB: unknown error")
	}

	return &Handle{handle: handle}, nil
}

// Close detaches SyndDB and flushes pending changesets
func (h *Handle) Close() error {
	if h.handle != nil {
		C.synddb_detach(h.handle)
		h.handle = nil
	}
	return nil
}

// LastError returns the last error message from the C library
func LastError() string {
	errMsg := C.synddb_last_error()
	if errMsg != nil {
		return C.GoString(errMsg)
	}
	return ""
}
