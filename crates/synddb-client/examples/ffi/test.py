#!/usr/bin/env python3
"""
Python ctypes FFI Test for SyndDB Client

Run with:
    python3 test.py
"""

import ctypes
import sys
import platform
from pathlib import Path

# Load the shared library (platform-specific)
def get_library_path():
    system = platform.system()
    machine = platform.machine().lower()

    # Determine platform-specific paths
    if system == "Darwin":
        if machine == "arm64":
            lib_dir = "darwin-arm64"
        else:
            lib_dir = "darwin-x64"
        lib_name = "libsynddb_client.dylib"
    elif system == "Linux":
        lib_dir = "linux-x64"
        lib_name = "libsynddb_client.so"
    elif system == "Windows":
        lib_dir = "win-x64"
        lib_name = "synddb_client.dll"
    else:
        raise RuntimeError(f"Unsupported platform: {system}")

    # Try committed library first (for development)
    committed_lib = Path(__file__).parent / f"../../libs/{lib_dir}/{lib_name}"
    if committed_lib.exists():
        return committed_lib

    # Fall back to target/release (for local builds)
    release_lib = Path(__file__).parent / f"../../../../target/release/{lib_name}"
    if release_lib.exists():
        return release_lib

    print(f"Error: Library not found")
    print(f"  Checked: {committed_lib}")
    print(f"  Checked: {release_lib}")
    print(f"\nBuild the library: cargo build --release --package synddb-client --features ffi")
    sys.exit(1)

lib_path = get_library_path()

lib = ctypes.CDLL(str(lib_path))

# Error codes (matching SyndDBError in ffi.rs)
class SyndDBError:
    SUCCESS = 0
    INVALID_POINTER = 1
    INVALID_UTF8 = 2
    DATABASE_ERROR = 3
    ATTACH_ERROR = 4
    PUBLISH_ERROR = 5
    SNAPSHOT_ERROR = 6

# Opaque handle type
class SyndDBHandle(ctypes.Structure):
    pass

# Function signatures
lib.synddb_version.argtypes = []
lib.synddb_version.restype = ctypes.c_char_p

lib.synddb_last_error.argtypes = []
lib.synddb_last_error.restype = ctypes.c_char_p

lib.synddb_attach.argtypes = [
    ctypes.c_char_p,  # db_path
    ctypes.c_char_p,  # sequencer_url
    ctypes.POINTER(ctypes.POINTER(SyndDBHandle))  # out_handle
]
lib.synddb_attach.restype = ctypes.c_int

lib.synddb_attach_with_config.argtypes = [
    ctypes.c_char_p,  # db_path
    ctypes.c_char_p,  # sequencer_url
    ctypes.c_ulonglong,  # publish_interval_ms
    ctypes.c_ulonglong,  # snapshot_interval
    ctypes.POINTER(ctypes.POINTER(SyndDBHandle))  # out_handle
]
lib.synddb_attach_with_config.restype = ctypes.c_int

lib.synddb_publish.argtypes = [ctypes.POINTER(SyndDBHandle)]
lib.synddb_publish.restype = ctypes.c_int

lib.synddb_snapshot.argtypes = [
    ctypes.POINTER(SyndDBHandle),
    ctypes.POINTER(ctypes.c_size_t)  # out_size
]
lib.synddb_snapshot.restype = ctypes.c_int

lib.synddb_detach.argtypes = [ctypes.POINTER(SyndDBHandle)]
lib.synddb_detach.restype = None


def main():
    print("=== SyndDB Python ctypes FFI Test ===\n")

    # Test 1: Get version
    print("1. Testing synddb_version()...")
    version = lib.synddb_version()
    print(f"   Library version: {version.decode('utf-8')}")
    print("   ✓ Version check passed\n")

    # Test 2: Attach with null pointer (should fail)
    print("2. Testing error handling (null pointer)...")
    handle = ctypes.POINTER(SyndDBHandle)()
    result = lib.synddb_attach(None, None, ctypes.byref(handle))
    if result == SyndDBError.INVALID_POINTER:
        error = lib.synddb_last_error()
        error_str = error.decode('utf-8') if error else "(no error message)"
        print(f"   Expected error: {error_str}")
        print("   ✓ Error handling works\n")
    else:
        print(f"   ✗ Expected INVALID_POINTER, got {result}\n")
        return 1

    # Test 3: Attach to database
    print("3. Testing synddb_attach()...")
    db_path = b"/tmp/test_ffi_python.db"
    sequencer_url = b"http://localhost:8433"

    handle = ctypes.POINTER(SyndDBHandle)()
    result = lib.synddb_attach(db_path, sequencer_url, ctypes.byref(handle))
    if result != SyndDBError.SUCCESS:
        error = lib.synddb_last_error()
        error_str = error.decode('utf-8') if error else "(unknown error)"
        print(f"   ✗ Failed to attach: {error_str}")
        return 1
    print("   ✓ Successfully attached to database\n")

    # Test 4: Manual publish
    print("4. Testing synddb_publish()...")
    result = lib.synddb_publish(handle)
    if result != SyndDBError.SUCCESS:
        error = lib.synddb_last_error()
        error_str = error.decode('utf-8') if error else "(unknown error)"
        print(f"   ✗ Failed to publish: {error_str}")
        lib.synddb_detach(handle)
        return 1
    print("   ✓ Successfully published\n")

    # Test 5: Create snapshot
    print("5. Testing synddb_snapshot()...")
    snapshot_size = ctypes.c_size_t()
    result = lib.synddb_snapshot(handle, ctypes.byref(snapshot_size))
    if result != SyndDBError.SUCCESS:
        error = lib.synddb_last_error()
        error_str = error.decode('utf-8') if error else "(unknown error)"
        print(f"   Warning: Snapshot failed: {error_str}")
        print("   (This is expected if sequencer is not running)\n")
    else:
        print(f"   ✓ Successfully created snapshot ({snapshot_size.value} bytes)\n")

    # Test 6: Test attach_with_config
    print("6. Testing synddb_attach_with_config()...")
    lib.synddb_detach(handle)  # Detach previous handle first

    handle2 = ctypes.POINTER(SyndDBHandle)()
    db_path2 = b"/tmp/test_ffi_python2.db"
    result = lib.synddb_attach_with_config(
        db_path2,
        sequencer_url,
        500,  # publish_interval_ms
        10,   # snapshot_interval
        ctypes.byref(handle2)
    )
    if result != SyndDBError.SUCCESS:
        error = lib.synddb_last_error()
        error_str = error.decode('utf-8') if error else "(unknown error)"
        print(f"   ✗ Failed to attach with config: {error_str}")
        return 1
    print("   ✓ Successfully attached with custom config\n")

    # Test 7: Detach
    print("7. Testing synddb_detach()...")
    lib.synddb_detach(handle2)
    print("   ✓ Successfully detached\n")

    print("=== All Python ctypes FFI tests passed! ===")
    return 0


if __name__ == "__main__":
    sys.exit(main())
