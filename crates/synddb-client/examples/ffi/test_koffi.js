#!/usr/bin/env node
/**
 * Node.js koffi FFI Test for SyndDB Client
 *
 * Install dependencies:
 *   npm install koffi
 *
 * Run with:
 *   node test_koffi.js
 */

const koffi = require('koffi');
const path = require('path');

// Library path
const libPath = path.join(__dirname, '../../../../target/release/libsynddb_client.dylib');

// Load the library
const lib = koffi.load(libPath);

// Error codes (matching SyndDBError in ffi.rs)
const SyndDBError = {
    SUCCESS: 0,
    INVALID_POINTER: 1,
    INVALID_UTF8: 2,
    DATABASE_ERROR: 3,
    ATTACH_ERROR: 4,
    PUBLISH_ERROR: 5,
    SNAPSHOT_ERROR: 6,
};

// Opaque pointer types
const SyndDBHandle = koffi.opaque('SyndDBHandle');
const SyndDBHandlePtr = koffi.pointer(SyndDBHandle);
const SyndDBHandlePtrPtr = koffi.out(koffi.pointer(SyndDBHandlePtr));

// Function definitions
const synddb_version = lib.func('synddb_version', 'str', []);
const synddb_last_error = lib.func('synddb_last_error', 'str', []);
const synddb_attach = lib.func('synddb_attach', 'int', ['str', 'str', SyndDBHandlePtrPtr]);
const synddb_attach_with_config = lib.func('synddb_attach_with_config', 'int', [
    'str',      // db_path
    'str',      // sequencer_url
    'uint64',   // publish_interval_ms
    'uint64',   // snapshot_interval
    SyndDBHandlePtrPtr  // out_handle
]);
const synddb_publish = lib.func('synddb_publish', 'int', [SyndDBHandlePtr]);
const synddb_snapshot = lib.func('synddb_snapshot', 'int', [
    SyndDBHandlePtr,
    koffi.out(koffi.pointer('size_t'))
]);
const synddb_detach = lib.func('synddb_detach', 'void', [SyndDBHandlePtr]);

async function main() {
    console.log('=== SyndDB Node.js koffi FFI Test ===\n');

    try {
        // Test 1: Get version
        console.log('1. Testing synddb_version()...');
        const version = synddb_version();
        console.log(`   Library version: ${version}`);
        console.log('   ✓ Version check passed\n');

        // Test 2: Attach with null pointer (should fail)
        console.log('2. Testing error handling (null pointer)...');
        let handle = [null];
        let result = synddb_attach(null, null, handle);
        if (result === SyndDBError.INVALID_POINTER) {
            const error = synddb_last_error();
            console.log(`   Expected error: ${error || '(no error message)'}`);
            console.log('   ✓ Error handling works\n');
        } else {
            console.log(`   ✗ Expected INVALID_POINTER, got ${result}\n`);
            return 1;
        }

        // Test 3: Attach to database
        console.log('3. Testing synddb_attach()...');
        const dbPath = '/tmp/test_ffi_nodejs_koffi.db';
        const sequencerUrl = 'http://localhost:8433';

        handle = [null];
        result = synddb_attach(dbPath, sequencerUrl, handle);
        if (result !== SyndDBError.SUCCESS) {
            const error = synddb_last_error();
            console.log(`   ✗ Failed to attach: ${error || '(unknown error)'}`);
            return 1;
        }
        console.log('   ✓ Successfully attached to database\n');

        // Test 4: Manual publish
        console.log('4. Testing synddb_publish()...');
        result = synddb_publish(handle[0]);
        if (result !== SyndDBError.SUCCESS) {
            const error = synddb_last_error();
            console.log(`   ✗ Failed to publish: ${error || '(unknown error)'}`);
            synddb_detach(handle[0]);
            return 1;
        }
        console.log('   ✓ Successfully published\n');

        // Test 5: Create snapshot
        console.log('5. Testing synddb_snapshot()...');
        const size = [0];
        result = synddb_snapshot(handle[0], size);
        if (result !== SyndDBError.SUCCESS) {
            const error = synddb_last_error();
            console.log(`   Warning: Snapshot failed: ${error || '(unknown error)'}`);
            console.log('   (This is expected if sequencer is not running)\n');
        } else {
            console.log(`   ✓ Successfully created snapshot (${size[0]} bytes)\n`);
        }

        // Test 6: Test attach_with_config
        console.log('6. Testing synddb_attach_with_config()...');
        synddb_detach(handle[0]); // Detach previous handle first

        const handle2 = [null];
        const dbPath2 = '/tmp/test_ffi_nodejs_koffi2.db';
        result = synddb_attach_with_config(
            dbPath2,
            sequencerUrl,
            500,  // publish_interval_ms
            10,   // snapshot_interval
            handle2
        );
        if (result !== SyndDBError.SUCCESS) {
            const error = synddb_last_error();
            console.log(`   ✗ Failed to attach with config: ${error || '(unknown error)'}`);
            return 1;
        }
        console.log('   ✓ Successfully attached with custom config\n');

        // Test 7: Detach
        console.log('7. Testing synddb_detach()...');
        synddb_detach(handle2[0]);
        console.log('   ✓ Successfully detached\n');

        console.log('=== All Node.js koffi FFI tests passed! ===');
        return 0;

    } catch (error) {
        console.error('Error:', error.message);
        console.error(error.stack);
        return 1;
    }
}

main().then(code => process.exit(code));
