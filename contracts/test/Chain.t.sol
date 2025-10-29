// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {Test, console} from "forge-std/Test.sol";
import "../src/Chain.sol";

contract ChainTest is Test {
    Chain public chain;
    address public owner;
    address public sequencer;
    address public validator;
    address public bridge;

    event VersionActivationDelayUpdated(uint256 oldDelay, uint256 newDelay);
    event WASMVersionAdded(bytes32 indexed versionHash, string ipfsCID, string arweaveTxId, uint256 activationTimestamp);

    function setUp() public {
        owner = address(this);
        sequencer = makeAddr("sequencer");
        validator = makeAddr("validator");
        bridge = makeAddr("bridge");

        // Initialize with a test WASM version
        bytes32 initialWasmVersion = keccak256("initial-version");
        string memory ipfsCID = "QmInitialVersion";
        string memory arweaveTxId = "arweave-initial";

        chain = new Chain(sequencer, bridge, initialWasmVersion, ipfsCID, arweaveTxId);
    }

    function test_InitialActivationDelay() public view {
        assertEq(chain.versionActivationDelay(), 1 days, "Initial delay should be 1 day");
    }

    function test_UpdateVersionActivationDelay() public {
        uint256 newDelay = 2 days;

        // Expect the event to be emitted
        vm.expectEmit(true, true, false, true);
        emit VersionActivationDelayUpdated(1 days, newDelay);

        // Update the delay
        chain.updateVersionActivationDelay(newDelay);

        // Check the delay was updated
        assertEq(chain.versionActivationDelay(), newDelay, "Delay should be updated");
    }

    function test_UpdateDelayToZero() public {
        // Admin can set zero delay for immediate activation if desired
        chain.updateVersionActivationDelay(0);
        assertEq(chain.versionActivationDelay(), 0, "Should accept zero delay");
    }

    function test_UpdateDelayToLargeValue() public {
        uint256 largeDelay = 365 days; // 1 year
        chain.updateVersionActivationDelay(largeDelay);
        assertEq(chain.versionActivationDelay(), largeDelay, "Should accept large delay");
    }

    function test_RevertWhen_NonOwnerUpdatesDelay() public {
        address nonOwner = makeAddr("nonOwner");

        vm.startPrank(nonOwner);
        vm.expectRevert();
        chain.updateVersionActivationDelay(2 days);
        vm.stopPrank();
    }

    function testFuzz_UpdateVersionActivationDelay(uint256 newDelay) public {
        uint256 oldDelay = chain.versionActivationDelay();

        // Expect the event
        vm.expectEmit(true, true, false, true);
        emit VersionActivationDelayUpdated(oldDelay, newDelay);

        chain.updateVersionActivationDelay(newDelay);

        assertEq(chain.versionActivationDelay(), newDelay, "Delay should be updated to fuzzed value");
    }

    function test_DelayDoesNotAffectPendingVersion() public {
        // First, add a WASM version with current delay
        bytes32 versionHash = keccak256("v1.0.0");
        string memory ipfsCID = "QmTest123";
        string memory arweaveTxId = "arweave123";
        uint256 initialDelay = chain.versionActivationDelay();

        // Take a snapshot of the current time
        uint256 startTime = block.timestamp;

        chain.addWASMVersion(versionHash, ipfsCID, arweaveTxId, initialDelay);

        // Get the pending version's activation timestamp
        (,,,uint256 activationTimestamp,,) = chain.wasmVersions(versionHash);
        uint256 expectedActivationTimestamp = startTime + initialDelay;
        assertEq(activationTimestamp, expectedActivationTimestamp, "Activation timestamp should use initial delay");

        // Now update the delay
        uint256 newDelay = 2 days;
        chain.updateVersionActivationDelay(newDelay);

        // Check that pending version's activation timestamp hasn't changed
        (,,,uint256 activationTimestampAfter,,) = chain.wasmVersions(versionHash);
        assertEq(activationTimestampAfter, expectedActivationTimestamp, "Pending version activation should not change");

        // Add another version with the new delay
        bytes32 versionHash2 = keccak256("v2.0.0");
        chain.addWASMVersion(versionHash2, ipfsCID, arweaveTxId, newDelay);

        // This one should use the new delay
        (,,,uint256 activationTimestamp2,,) = chain.wasmVersions(versionHash2);
        assertEq(activationTimestamp2, block.timestamp + newDelay, "New version should use updated delay");
    }


    function test_VersionActivationWithTimestamp() public {
        bytes32 versionHash = keccak256("v1.0.0");
        string memory ipfsCID = "QmTest123";
        string memory arweaveTxId = "arweave123";
        uint256 delay = 1 days;

        uint256 expectedActivation = block.timestamp + delay;

        vm.expectEmit(true, false, false, true);
        emit WASMVersionAdded(versionHash, ipfsCID, arweaveTxId, expectedActivation);

        chain.addWASMVersion(versionHash, ipfsCID, arweaveTxId, delay);

        // Try to activate before time
        vm.expectRevert("Activation not ready");
        chain.activateWASMVersion();

        // Warp to activation time
        vm.warp(expectedActivation);

        // Now activation should work
        chain.activateWASMVersion();

        // Verify it's active
        (,,, uint256 activationTimestamp, bool isActive,) = chain.wasmVersions(versionHash);
        assertTrue(isActive, "Version should be active");
        assertEq(activationTimestamp, expectedActivation, "Activation timestamp should match");
    }
}