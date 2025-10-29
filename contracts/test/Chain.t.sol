// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {Test, console} from "forge-std/Test.sol";
import {Chain} from "../src/Chain.sol";

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

        chain = new Chain(sequencer, bridge);
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

    function test_RevertWhen_DelayBelowMinimum() public {
        uint256 tooSmallDelay = chain.MIN_VERSION_ACTIVATION_DELAY() - 1;

        vm.expectRevert("Delay out of bounds");
        chain.updateVersionActivationDelay(tooSmallDelay);
    }

    function test_RevertWhen_DelayAboveMaximum() public {
        uint256 tooLargeDelay = chain.MAX_VERSION_ACTIVATION_DELAY() + 1;

        vm.expectRevert("Delay out of bounds");
        chain.updateVersionActivationDelay(tooLargeDelay);
    }

    function test_UpdateDelayAtMinimumBoundary() public {
        uint256 minDelay = chain.MIN_VERSION_ACTIVATION_DELAY();

        chain.updateVersionActivationDelay(minDelay);
        assertEq(chain.versionActivationDelay(), minDelay, "Should accept minimum delay");
    }

    function test_UpdateDelayAtMaximumBoundary() public {
        uint256 maxDelay = chain.MAX_VERSION_ACTIVATION_DELAY();

        chain.updateVersionActivationDelay(maxDelay);
        assertEq(chain.versionActivationDelay(), maxDelay, "Should accept maximum delay");
    }

    function test_RevertWhen_NonOwnerUpdatesDelay() public {
        address nonOwner = makeAddr("nonOwner");

        vm.startPrank(nonOwner);
        vm.expectRevert();
        chain.updateVersionActivationDelay(2 days);
        vm.stopPrank();
    }

    function testFuzz_UpdateVersionActivationDelay(uint256 newDelay) public {
        // Bound the delay to valid range
        newDelay = bound(newDelay, chain.MIN_VERSION_ACTIVATION_DELAY(), chain.MAX_VERSION_ACTIVATION_DELAY());

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

    function test_ConstantValues() public view {
        assertEq(chain.MIN_VERSION_ACTIVATION_DELAY(), 12 hours, "Min delay should be 12 hours");
        assertEq(chain.MAX_VERSION_ACTIVATION_DELAY(), 10 days, "Max delay should be 10 days");
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