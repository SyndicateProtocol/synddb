// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Bridge} from "src/Bridge.sol";
import {SequencerSignature} from "src/types/DataTypes.sol";
import {ERC20TotalSupplyCheckModule} from "src/modules/ERC20TotalSupplyCheckModule.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {WETH9} from "./mocks/WETH9.sol";
import {ETHReceiver} from "./mocks/ETHReceiver.sol";
import {UseCaseBaseTest} from "./base/UseCaseBaseTest.sol";

/**
 * @title UseCase1_ETHTransfer
 * @notice Tests for ETH transfer through the bridge with validator signatures
 */
contract UseCase1_ETHTransfer is UseCaseBaseTest {
    Bridge public bridge;
    WETH9 public weth;
    ETHReceiver public recipient;
    ERC20TotalSupplyCheckModule public supplyCheckModule;
    ValidatorSignatureThresholdModule public validatorModule;

    address public admin;
    address public sequencer;
    address public user;

    function setUp() public {
        admin = address(this);
        sequencer = makeAddr("sequencer");
        user = makeAddr("user");

        weth = new WETH9();
        bridge = new Bridge(admin, address(weth));
        recipient = new ETHReceiver();

        setupValidators(bridge);
        validatorModule = new ValidatorSignatureThresholdModule(address(bridge), validators, 2);
        supplyCheckModule = new ERC20TotalSupplyCheckModule(address(weth), 1000000 ether);

        bridge.grantRole(bridge.SEQUENCER_ROLE(), sequencer);
        bridge.addPreModule(address(supplyCheckModule));
        bridge.addPreModule(address(validatorModule));

        vm.deal(user, 100 ether);
    }

    /// @notice Test Bridge auto-wraps ETH to WETH
    function test_WrapETHToWETH() public {
        uint256 depositAmount = 1 ether;

        vm.prank(user);
        (bool success,) = address(bridge).call{value: depositAmount}("");

        assertTrue(success);
        assertEq(weth.balanceOf(address(bridge)), depositAmount);
    }

    /// @notice Test ETH transfer via WETH with supply check module and validator signatures
    function test_ETHTransferViaWETH() public {
        uint256 transferAmount = 10 ether;

        vm.prank(user);
        (bool success,) = address(bridge).call{value: transferAmount}("");
        assertTrue(success);

        bytes32 messageId = keccak256(abi.encodePacked("transfer", block.timestamp));
        bytes memory payload = abi.encodeWithSelector(weth.transfer.selector, address(recipient), transferAmount);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(weth), payload, sig);

        // Submit validator signatures (2 out of 3 threshold)
        submitValidatorSignatures(bridge, messageId);

        bridge.handleMessage(messageId);

        assertEq(weth.balanceOf(address(recipient)), transferAmount);
        assertEq(weth.balanceOf(address(bridge)), 0);
    }

    /// @notice Test WETH deposit and withdrawal
    function test_WETHDepositAndWithdraw() public {
        uint256 amount = 10 ether;

        vm.startPrank(user);

        uint256 balanceBefore = user.balance;
        weth.deposit{value: amount}();

        assertEq(user.balance, balanceBefore - amount);
        assertEq(weth.balanceOf(user), amount);

        weth.withdraw(amount);

        assertEq(user.balance, balanceBefore);
        assertEq(weth.balanceOf(user), 0);

        vm.stopPrank();
    }

    /// @notice Test multiple sequential ETH transfers with module checks and validator signatures
    function test_MultipleETHTransfers() public {
        uint256[] memory amounts = new uint256[](3);
        amounts[0] = 1 ether;
        amounts[1] = 2 ether;
        amounts[2] = 3 ether;

        for (uint256 i = 0; i < amounts.length; i++) {
            vm.prank(user);
            (bool success,) = address(bridge).call{value: amounts[i]}("");
            assertTrue(success);

            bytes32 messageId = keccak256(abi.encodePacked("transfer", i));
            bytes memory payload = abi.encodeWithSelector(weth.transfer.selector, address(recipient), amounts[i]);

            SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

            vm.prank(sequencer);
            bridge.initializeMessage(messageId, address(weth), payload, sig);

            // Submit validator signatures
            submitValidatorSignatures(bridge, messageId);

            bridge.handleMessage(messageId);
        }

        uint256 expectedTotal = amounts[0] + amounts[1] + amounts[2];
        assertEq(weth.balanceOf(address(recipient)), expectedTotal);
    }

    /*//////////////////////////////////////////////////////////////
                    VALIDATOR SIGNATURE TESTS
    //////////////////////////////////////////////////////////////*/

    /// @notice Test that message fails without sufficient validator signatures
    function test_ETHTransfer_FailsWithInsufficientSignatures() public {
        uint256 transferAmount = 10 ether;

        vm.prank(user);
        (bool success,) = address(bridge).call{value: transferAmount}("");
        assertTrue(success);

        bytes32 messageId = keccak256("insufficient-sigs");
        bytes memory payload = abi.encodeWithSelector(weth.transfer.selector, address(recipient), transferAmount);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(weth), payload, sig);

        // Only submit 1 signature (threshold is 2)
        submitValidatorSignatures(bridge, messageId, 1);

        // Should revert due to insufficient signatures
        vm.expectRevert();
        bridge.handleMessage(messageId);
    }

    /// @notice Test that message succeeds with exact threshold
    function test_ETHTransfer_SucceedsWithExactThreshold() public {
        uint256 transferAmount = 5 ether;

        vm.prank(user);
        (bool success,) = address(bridge).call{value: transferAmount}("");
        assertTrue(success);

        bytes32 messageId = keccak256("exact-threshold");
        bytes memory payload = abi.encodeWithSelector(weth.transfer.selector, address(recipient), transferAmount);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(weth), payload, sig);

        // Submit exactly 2 signatures (threshold is 2)
        submitValidatorSignatures(bridge, messageId);

        bridge.handleMessage(messageId);

        assertEq(weth.balanceOf(address(recipient)), transferAmount);
    }

    /// @notice Test that message succeeds with more than threshold
    function test_ETHTransfer_SucceedsWithMoreThanThreshold() public {
        uint256 transferAmount = 7 ether;

        vm.prank(user);
        (bool success,) = address(bridge).call{value: transferAmount}("");
        assertTrue(success);

        bytes32 messageId = keccak256("more-than-threshold");
        bytes memory payload = abi.encodeWithSelector(weth.transfer.selector, address(recipient), transferAmount);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(weth), payload, sig);

        // Submit all 3 signatures (threshold is 2)
        submitValidatorSignatures(bridge, messageId, 3);

        bridge.handleMessage(messageId);

        assertEq(weth.balanceOf(address(recipient)), transferAmount);
    }

    /// @notice Test that invalid validator signature is rejected
    function test_ETHTransfer_RejectsInvalidValidatorSignature() public {
        uint256 unauthorizedPrivateKey = 0x999;

        bytes32 messageId = keccak256("invalid-validator");

        bytes memory invalidSig = signMessage(messageId, unauthorizedPrivateKey);

        // Should revert because validator is not authorized
        vm.expectRevert();
        bridge.signMessageWithSignature(messageId, invalidSig);
    }

    receive() external payable {}
}
