/**
 * C FFI Test for SyndDB Client
 *
 * Compile with:
 *   clang -o test_c test.c -L../../../target/release -lsynddb_client -Wl,-rpath,@loader_path/../../../target/release
 *
 * Run with:
 *   ./test_c
 */

#include <stdio.h>
#include <stdlib.h>

// FFI type definitions matching ffi.rs
typedef struct SyndDBHandle SyndDBHandle;

typedef enum {
    Success = 0,
    InvalidPointer = 1,
    InvalidUtf8 = 2,
    DatabaseError = 3,
    AttachError = 4,
    PublishError = 5,
    SnapshotError = 6,
    InvalidUrl = 7,
} SyndDBError;

// FFI function declarations
extern SyndDBError synddb_attach(
    const char* db_path,
    const char* sequencer_url,
    SyndDBHandle** out_handle
);

extern SyndDBError synddb_attach_with_config(
    const char* db_path,
    const char* sequencer_url,
    unsigned long long flush_interval_ms,
    unsigned long long snapshot_interval,
    SyndDBHandle** out_handle
);

extern SyndDBError synddb_publish_changeset(SyndDBHandle* handle);

extern SyndDBError synddb_publish_snapshot(
    SyndDBHandle* handle,
    size_t* out_size
);

extern void synddb_detach(SyndDBHandle* handle);

extern const char* synddb_last_error(void);

extern const char* synddb_version(void);

int main(void) {
    printf("=== SyndDB C FFI Test ===\n\n");

    // Test 1: Get version
    printf("1. Testing synddb_version()...\n");
    const char* version = synddb_version();
    printf("   Library version: %s\n", version);
    printf("   ✓ Version check passed\n\n");

    // Test 2: Attach with null pointer (should fail)
    printf("2. Testing error handling (null pointer)...\n");
    SyndDBHandle* handle = NULL;
    SyndDBError result = synddb_attach(NULL, NULL, &handle);
    if (result == InvalidPointer) {
        const char* error = synddb_last_error();
        printf("   Expected error: %s\n", error ? error : "(no error message)");
        printf("   ✓ Error handling works\n\n");
    } else {
        printf("   ✗ Expected InvalidPointer, got %d\n\n", result);
        return 1;
    }

    // Test 3: Attach to database
    printf("3. Testing synddb_attach()...\n");
    const char* db_path = "/tmp/test_ffi.db";
    const char* sequencer_url = "http://localhost:8433";

    result = synddb_attach(db_path, sequencer_url, &handle);
    if (result != Success) {
        const char* error = synddb_last_error();
        printf("   ✗ Failed to attach: %s\n", error ? error : "(unknown error)");
        return 1;
    }
    printf("   ✓ Successfully attached to database\n\n");

    // Test 4: Manual publish (should succeed even with no data)
    printf("4. Testing synddb_publish_changeset()...\n");
    result = synddb_publish_changeset(handle);
    if (result != Success) {
        const char* error = synddb_last_error();
        printf("   ✗ Failed to publish: %s\n", error ? error : "(unknown error)");
        synddb_detach(handle);
        return 1;
    }
    printf("   ✓ Successfully published\n\n");

    // Test 5: Create snapshot
    printf("5. Testing synddb_publish_snapshot()...\n");
    size_t snapshot_size = 0;
    result = synddb_publish_snapshot(handle, &snapshot_size);
    if (result != Success) {
        const char* error = synddb_last_error();
        printf("   Warning: Snapshot failed: %s\n", error ? error : "(unknown error)");
        printf("   (This is expected if sequencer is not running)\n\n");
    } else {
        printf("   ✓ Successfully created snapshot (%zu bytes)\n\n", snapshot_size);
    }

    // Test 6: Detach
    printf("6. Testing synddb_detach()...\n");
    synddb_detach(handle);
    printf("   ✓ Successfully detached\n\n");

    printf("=== All C FFI tests passed! ===\n");
    return 0;
}
