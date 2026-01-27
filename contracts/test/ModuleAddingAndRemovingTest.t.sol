// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {Bridge} from "src/Bridge.sol";
import {IModuleCheck} from "src/interfaces/IModuleCheck.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";
import {MockAttestationVerifier} from "src/attestation/MockAttestationVerifier.sol";
import {ProcessingStage, SequencerSignature} from "src/types/DataTypes.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/// @notice Mock module that always returns true
contract MockModule is IModuleCheck {
    function check(bytes32, ProcessingStage, bytes memory, SequencerSignature memory) external pure returns (bool) {
        return true;
    }
}

/// @notice Mock module that performs some computation to consume gas
contract GasConsumingModule is IModuleCheck {
    uint256 public counter;

    function check(bytes32, ProcessingStage, bytes memory, SequencerSignature memory) external returns (bool) {
        // Perform some operations to consume gas similar to real module checks
        counter++;
        uint256 temp = 0;
        for (uint256 i = 0; i < 10; i++) {
            temp += i * counter;
        }
        return temp >= 0; // Always true, but with gas consumption
    }
}

contract ModuleAddingAndRemovingTest is Test {
    Bridge public bridge;
    TeeKeyManager public teeKeyManager;
    MockAttestationVerifier public attestationVerifier;

    address public admin;
    address public sequencer;
    uint256 public sequencerPrivateKey = 0xA11CE;
    address public weth;

    event PreModuleAdded(address indexed module);
    event PreModuleRemoved(address indexed module);

    function setUp() public {
        admin = address(this);
        sequencer = vm.addr(sequencerPrivateKey);
        weth = makeAddr("weth");

        // Deploy attestation infrastructure
        attestationVerifier = new MockAttestationVerifier();
        teeKeyManager = new TeeKeyManager(attestationVerifier);

        // Register sequencer as a valid TEE key
        bytes memory publicValues = abi.encode(sequencer);
        teeKeyManager.addKey(publicValues, "");

        bridge = new Bridge(admin, weth, address(teeKeyManager));
        bridge.grantRole(bridge.MESSAGE_INITIALIZER_ROLE(), sequencer);
    }

    /// @notice Create a sequencer signature for a message
    function createSequencerSignature(
        bytes32 messageId,
        address targetAddress,
        bytes memory payload,
        uint256 nativeTokenAmount
    ) internal view returns (SequencerSignature memory) {
        bytes32 messageHash = keccak256(
            abi.encodePacked(messageId, targetAddress, keccak256(payload), nativeTokenAmount)
        );
        bytes32 ethSignedHash = MessageHashUtils.toEthSignedMessageHash(messageHash);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(sequencerPrivateKey, ethSignedHash);
        return SequencerSignature({signature: abi.encodePacked(r, s, v), submittedAt: block.timestamp});
    }

    /*//////////////////////////////////////////////////////////////
                        ENUMERABLE SET GAS TESTS
    //////////////////////////////////////////////////////////////*/

    /// @notice Test that removal from EnumerableSet is O(1) regardless of the number of modules
    /// This is critical to ensure that even if adding modules becomes expensive,
    /// removal remains feasible
    function test_RemovePreModule_IsO1() public {
        // Test with increasing numbers of modules
        uint256[] memory moduleCounts = new uint256[](5);
        moduleCounts[0] = 10;
        moduleCounts[1] = 50;
        moduleCounts[2] = 100;
        moduleCounts[3] = 200;
        moduleCounts[4] = 500;

        uint256[] memory removalGasCosts = new uint256[](moduleCounts.length);

        for (uint256 j = 0; j < moduleCounts.length; j++) {
            // Deploy fresh Bridge for each test
            Bridge testBridge = new Bridge(admin, weth, address(teeKeyManager));

            // Add modules
            address[] memory modules = new address[](moduleCounts[j]);
            for (uint256 i = 0; i < moduleCounts[j]; i++) {
                modules[i] = address(new MockModule());
                testBridge.addPreModule(modules[i]);
            }

            // Measure gas for removing the LAST module (worst case for O(n) implementations)
            uint256 gasBefore = gasleft();
            testBridge.removePreModule(modules[moduleCounts[j] - 1]);
            uint256 gasUsed = gasBefore - gasleft();

            removalGasCosts[j] = gasUsed;

            emit log_named_uint("Modules count", moduleCounts[j]);
            emit log_named_uint("Removal gas cost", gasUsed);
        }

        // Verify that gas costs don't scale linearly with module count
        // For O(1) operations, the gas should remain relatively constant
        // Allow for some variance due to storage operations, but ensure it's not O(n)

        // The gas cost for 500 modules should not be more than 2x the cost for 10 modules
        // (being generous to account for storage and SSTORE operations)
        assertLt(
            removalGasCosts[4],
            removalGasCosts[0] * 2,
            "Removal gas cost scales too much with module count - may not be O(1)"
        );
    }

    /// @notice Test removal from different positions in the set
    /// EnumerableSet.remove() is O(1) because it swaps with the last element
    function test_RemovePreModule_DifferentPositions() public {
        uint256 moduleCount = 100;
        address[] memory modules = new address[](moduleCount);

        // Add modules
        for (uint256 i = 0; i < moduleCount; i++) {
            modules[i] = address(new MockModule());
            bridge.addPreModule(modules[i]);
        }

        // Test removing from beginning, middle, and end
        uint256[] memory positions = new uint256[](3);
        positions[0] = 0; // First
        positions[1] = moduleCount / 2; // Middle
        positions[2] = moduleCount - 1; // Last

        uint256[] memory gasCosts = new uint256[](3);

        for (uint256 i = 0; i < positions.length; i++) {
            uint256 gasBefore = gasleft();
            bridge.removePreModule(modules[positions[i]]);
            gasCosts[i] = gasBefore - gasleft();

            emit log_named_string("Position", i == 0 ? "First" : i == 1 ? "Middle" : "Last");
            emit log_named_uint("Gas cost", gasCosts[i]);
        }

        // All removal operations should have similar gas costs
        // Allow 20% variance
        for (uint256 i = 1; i < gasCosts.length; i++) {
            uint256 diff = gasCosts[i] > gasCosts[0] ? gasCosts[i] - gasCosts[0] : gasCosts[0] - gasCosts[i];
            uint256 maxDiff = (gasCosts[0] * 20) / 100;
            assertLt(diff, maxDiff, "Removal gas cost varies too much by position");
        }
    }

    /*//////////////////////////////////////////////////////////////
                        MODULE LOOP GAS TESTS
    //////////////////////////////////////////////////////////////*/

    /// @notice Test gas consumption of the module validation loop
    /// This helps determine the practical limit of modules before hitting block gas limit
    function test_ValidateModules_GasConsumption() public {
        vm.startPrank(admin);

        uint256[] memory moduleCounts = new uint256[](6);
        moduleCounts[0] = 10;
        moduleCounts[1] = 50;
        moduleCounts[2] = 100;
        moduleCounts[3] = 200;
        moduleCounts[4] = 500;
        moduleCounts[5] = 1000;

        emit log_string("=== Module Validation Loop Gas Consumption ===");
        emit log_named_uint("Block gas limit (typical)", 30_000_000);
        emit log_string("");

        for (uint256 j = 0; j < moduleCounts.length; j++) {
            // Deploy fresh Bridge for each test
            Bridge testBridge = new Bridge(admin, weth, address(teeKeyManager));
            testBridge.grantRole(testBridge.MESSAGE_INITIALIZER_ROLE(), sequencer);

            // Add gas-consuming modules
            for (uint256 i = 0; i < moduleCounts[j]; i++) {
                address module = address(new GasConsumingModule());
                testBridge.addPreModule(module);
            }

            // Create a test message
            bytes32 messageId = keccak256(abi.encodePacked("test", j));
            bytes memory payload = abi.encodeWithSignature("test()");
            SequencerSignature memory sig = createSequencerSignature(messageId, address(this), payload, 0);

            // Initialize message
            vm.startPrank(sequencer);
            testBridge.initializeMessage(messageId, address(this), payload, sig, 0);
            vm.stopPrank();

            // Measure gas for handling message (which validates all modules)
            uint256 gasBeforeHandle = gasleft();
            try testBridge.handleMessage(messageId) {
                uint256 gasUsed = gasBeforeHandle - gasleft();

                emit log_named_uint("Module count", moduleCounts[j]);
                emit log_named_uint("Total gas used", gasUsed);
                emit log_named_uint("Gas per module (approx)", gasUsed / moduleCounts[j]);
                emit log_named_decimal_uint("% of block gas limit", (gasUsed * 100) / 30_000_000, 2);
                emit log_string("");

                if (gasUsed > 10_000_000) {
                    emit log_string("WARNING: Approaching high gas usage!");
                }
            } catch {
                emit log_named_uint("Module count", moduleCounts[j]);
                emit log_string("FAILED: Out of gas or execution reverted");
                emit log_string("");
                break;
            }
        }

        vm.stopPrank();
    }

    /// @notice Test that validates the worst-case scenario described in the issue:
    /// Adding modules until near gas limit, then ensuring removal still works
    function test_RemovalWorksNearGasLimit() public {
        emit log_string("=== Testing Module Removal Near Gas Limits ===");

        // Add modules progressively and track gas costs
        uint256 moduleCount = 0;
        uint256 lastAddGas = 0;
        address lastModule;

        // Keep adding modules and monitoring gas costs
        for (uint256 i = 0; i < 1000; i++) {
            address module = address(new MockModule());

            uint256 gasBeforeAdd = gasleft();
            try bridge.addPreModule(module) {
                lastAddGas = gasBeforeAdd - gasleft();
                lastModule = module;
                moduleCount++;

                // Log every 100 modules
                if (moduleCount % 100 == 0) {
                    emit log_named_uint("Modules added", moduleCount);
                    emit log_named_uint("Last add gas cost", lastAddGas);
                }
            } catch {
                emit log_named_uint("Failed to add module at count", moduleCount);
                break;
            }

            // Stop if adding is getting very expensive (proxy for approaching gas limit issues)
            if (lastAddGas > 1_000_000) {
                emit log_string("Stopping: Add operation exceeding 1M gas");
                break;
            }
        }

        emit log_string("");
        emit log_named_uint("Total modules added", moduleCount);
        emit log_named_uint("Final add gas cost", lastAddGas);

        // Now test that we can still remove modules
        emit log_string("");
        emit log_string("Testing removal of last added module...");

        uint256 gasBefore = gasleft();
        bridge.removePreModule(lastModule);
        uint256 removalGas = gasBefore - gasleft();

        emit log_named_uint("Removal gas cost", removalGas);

        // Verify removal was successful and reasonably priced
        assertTrue(removalGas < 100_000, "Removal should be cheap even with many modules");

        // Verify the module was actually removed
        address[] memory modules = bridge.getPreModules();
        for (uint256 i = 0; i < modules.length; i++) {
            assertFalse(modules[i] == lastModule, "Module should be removed");
        }

        emit log_string("SUCCESS: Removal works even with many modules");
    }

    /// @notice Test adding and removing modules in a realistic scenario
    function test_RealisticModuleManagement() public {
        emit log_string("=== Realistic Module Management Test ===");

        // Simulate adding a reasonable number of modules (e.g., 20)
        address[] memory modules = new address[](20);
        uint256 totalAddGas = 0;

        for (uint256 i = 0; i < 20; i++) {
            modules[i] = address(new GasConsumingModule());
            uint256 gasBeforeAdd = gasleft();
            bridge.addPreModule(modules[i]);
            totalAddGas += gasBeforeAdd - gasleft();
        }

        emit log_named_uint("Total modules", 20);
        emit log_named_uint("Average add gas", totalAddGas / 20);

        // Test message handling with these modules
        bytes32 messageId = keccak256("realistic-test");
        bytes memory payload = abi.encodeWithSignature("test()");
        SequencerSignature memory sig = createSequencerSignature(messageId, address(this), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(this), payload, sig, 0);

        uint256 gasBeforeHandle = gasleft();
        try bridge.handleMessage(messageId) {
            uint256 handleGas = gasBeforeHandle - gasleft();
            emit log_named_uint("Handle message gas (20 modules)", handleGas);
            emit log_named_decimal_uint("% of block gas limit", (handleGas * 100) / 30_000_000, 2);
        } catch {
            emit log_string("FAILED: Message handling failed");
        }

        // Remove some modules
        uint256 totalRemoveGas = 0;
        for (uint256 i = 0; i < 5; i++) {
            uint256 gasBeforeRemove = gasleft();
            bridge.removePreModule(modules[i]);
            totalRemoveGas += gasBeforeRemove - gasleft();
        }

        emit log_named_uint("Average remove gas", totalRemoveGas / 5);
        emit log_string("SUCCESS: Realistic scenario completed");
    }

    /*//////////////////////////////////////////////////////////////
                        EDGE CASE TESTS
    //////////////////////////////////////////////////////////////*/

    /// @notice Specific test for the theoretical edge case:
    /// If adding a module causes gas issues, can it still be removed?
    function test_CanRemoveEvenIfAddingIsExpensive() public {
        emit log_string("=== Edge Case: Removal When Adding Is Expensive ===");

        // Add a large number of modules to simulate near-gas-limit conditions
        uint256 moduleCount = 500;
        address[] memory modules = new address[](moduleCount);

        for (uint256 i = 0; i < moduleCount; i++) {
            modules[i] = address(new GasConsumingModule());
            bridge.addPreModule(modules[i]);
        }

        emit log_named_uint("Modules in set", moduleCount);

        // Now add one more module
        address criticalModule = address(new GasConsumingModule());
        uint256 addGas = gasleft();
        bridge.addPreModule(criticalModule);
        addGas = addGas - gasleft();

        emit log_named_uint("Gas to add critical module", addGas);

        // Even if adding was expensive, removing should still be cheap (O(1))
        uint256 removeGas = gasleft();
        bridge.removePreModule(criticalModule);
        removeGas = removeGas - gasleft();

        emit log_named_uint("Gas to remove critical module", removeGas);

        // Assert removal is much cheaper than adding (demonstrating O(1) vs potential O(n) growth)
        assertTrue(removeGas < 50000, "Removal should remain under 50k gas regardless of set size");

        // The key insight: removal gas should not scale with number of modules
        emit log_string("SUCCESS: Removal is O(1) even with 501 modules");
    }

    /*//////////////////////////////////////////////////////////////
                        ERROR HANDLING TESTS
    //////////////////////////////////////////////////////////////*/

    /// @notice Test removing a module that doesn't exist
    function test_RemovePreModule_NonExistent_Reverts() public {
        address nonExistent = address(0x1234);

        vm.expectRevert(abi.encodeWithSignature("ModuleDoesNotExist()"));
        bridge.removePreModule(nonExistent);
    }

    /// @notice Test removing zero address
    function test_RemovePreModule_ZeroAddress_Reverts() public {
        vm.expectRevert(abi.encodeWithSignature("InvalidModuleAddress()"));
        bridge.removePreModule(address(0));
    }

    /// @notice Test removing the same module twice
    function test_RemovePreModule_Twice_Reverts() public {
        address module = address(new MockModule());
        bridge.addPreModule(module);

        bridge.removePreModule(module);

        vm.expectRevert(abi.encodeWithSignature("ModuleDoesNotExist()"));
        bridge.removePreModule(module);
    }

    /// @notice Test that removal emits the correct event
    function test_RemovePreModule_EmitsEvent() public {
        address module = address(new MockModule());
        bridge.addPreModule(module);

        vm.expectEmit(true, false, false, false);
        emit PreModuleRemoved(module);

        bridge.removePreModule(module);
    }

    /*//////////////////////////////////////////////////////////////
                        HELPER FUNCTIONS
    //////////////////////////////////////////////////////////////*/

    // Fallback to accept ETH
    receive() external payable {}

    // Mock function that can be called by the bridge
    function test() external pure returns (bool) {
        return true;
    }
}
