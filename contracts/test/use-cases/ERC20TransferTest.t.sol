// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Bridge} from "src/Bridge.sol";
import {SequencerSignature} from "src/types/DataTypes.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {MockERC20} from "./mocks/MockERC20.sol";
import {WETH9} from "./mocks/WETH9.sol";
import {UseCaseBaseTest} from "./base/UseCaseBaseTest.sol";

/**
 * @title ERC20TransferTest
 * @notice Tests for ERC-20 token transfers through the bridge with validator signatures
 */
contract ERC20TransferTest is UseCaseBaseTest {
    Bridge public bridge;
    MockERC20 public usdc;
    MockERC20 public dai;
    WETH9 public weth;
    ValidatorSignatureThresholdModule public validatorModule;

    address public sequencer;
    address public user;
    address public recipient;

    uint256 public constant INITIAL_BALANCE = 1_000_000e18;

    function setUp() public {
        sequencer = vm.addr(sequencerPrivateKey);
        user = makeAddr("user");
        recipient = makeAddr("recipient");

        usdc = new MockERC20("USD Coin", "USDC");
        dai = new MockERC20("Dai Stablecoin", "DAI");

        (bridge, weth) = createBridgeWithWETH(address(this), sequencer);
        validatorModule = setupBridgeWithValidators(bridge);

        usdc.transfer(user, INITIAL_BALANCE);
        dai.transfer(user, INITIAL_BALANCE);

        vm.deal(user, 100 ether);
    }

    /*//////////////////////////////////////////////////////////////
                        DIRECT TRANSFERS
    //////////////////////////////////////////////////////////////*/

    function test_TransferERC20() public {
        uint256 transferAmount = 1000e18;

        vm.prank(user);
        usdc.transfer(address(bridge), transferAmount);

        bytes32 messageId = keccak256("erc20-transfer-1");
        bytes memory payload = abi.encodeWithSelector(usdc.transfer.selector, recipient, transferAmount);

        SequencerSignature memory sig = createSequencerSignature(messageId, address(usdc), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(usdc), payload, sig, 0);

        submitValidatorSignatures(bridge, messageId);

        bridge.handleMessage(messageId);

        assertEq(usdc.balanceOf(recipient), transferAmount);
        assertEq(usdc.balanceOf(address(bridge)), 0);
    }

    function test_TransferFromERC20() public {
        uint256 transferAmount = 1000e18;

        vm.prank(user);
        usdc.approve(address(bridge), transferAmount);

        bytes32 messageId = keccak256("erc20-transferFrom-1");
        bytes memory payload = abi.encodeWithSelector(usdc.transferFrom.selector, user, recipient, transferAmount);

        SequencerSignature memory sig = createSequencerSignature(messageId, address(usdc), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(usdc), payload, sig, 0);

        submitValidatorSignatures(bridge, messageId);

        bridge.handleMessage(messageId);

        assertEq(usdc.balanceOf(recipient), transferAmount);
        assertEq(usdc.balanceOf(user), INITIAL_BALANCE - transferAmount);
    }

    /*//////////////////////////////////////////////////////////////
                        MULTI-TOKEN TRANSFERS
    //////////////////////////////////////////////////////////////*/

    function test_MultiTokenTransfer() public {
        uint256 usdcAmount = 500e18;
        uint256 daiAmount = 750e18;
        uint256 wethAmount = 1e18;

        vm.startPrank(user);
        usdc.transfer(address(bridge), usdcAmount);
        dai.transfer(address(bridge), daiAmount);
        weth.deposit{value: wethAmount}();
        weth.transfer(address(bridge), wethAmount);
        vm.stopPrank();

        // USDC transfer
        bytes32 usdcMessageId = keccak256("multi-usdc");
        bytes memory usdcPayload = abi.encodeWithSelector(usdc.transfer.selector, recipient, usdcAmount);
        SequencerSignature memory usdcSig = createSequencerSignature(usdcMessageId, address(usdc), usdcPayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(usdcMessageId, address(usdc), usdcPayload, usdcSig, 0);
        submitValidatorSignatures(bridge, usdcMessageId);
        bridge.handleMessage(usdcMessageId);

        // DAI transfer
        bytes32 daiMessageId = keccak256("multi-dai");
        bytes memory daiPayload = abi.encodeWithSelector(dai.transfer.selector, recipient, daiAmount);
        SequencerSignature memory daiSig = createSequencerSignature(daiMessageId, address(dai), daiPayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(daiMessageId, address(dai), daiPayload, daiSig, 0);
        submitValidatorSignatures(bridge, daiMessageId);
        bridge.handleMessage(daiMessageId);

        // WETH transfer
        bytes32 wethMessageId = keccak256("multi-weth");
        bytes memory wethPayload = abi.encodeWithSelector(weth.transfer.selector, recipient, wethAmount);
        SequencerSignature memory wethSig = createSequencerSignature(wethMessageId, address(weth), wethPayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(wethMessageId, address(weth), wethPayload, wethSig, 0);
        submitValidatorSignatures(bridge, wethMessageId);
        bridge.handleMessage(wethMessageId);

        assertEq(usdc.balanceOf(recipient), usdcAmount);
        assertEq(dai.balanceOf(recipient), daiAmount);
        assertEq(weth.balanceOf(recipient), wethAmount);
    }

    /*//////////////////////////////////////////////////////////////
                        BATCH TRANSFERS
    //////////////////////////////////////////////////////////////*/

    function test_BatchTransferToMultipleRecipients() public {
        address[] memory recipients = new address[](3);
        recipients[0] = makeAddr("recipient1");
        recipients[1] = makeAddr("recipient2");
        recipients[2] = makeAddr("recipient3");

        uint256 amountEach = 100e18;
        uint256 totalAmount = amountEach * recipients.length;

        vm.prank(user);
        usdc.transfer(address(bridge), totalAmount);

        for (uint256 i = 0; i < recipients.length; i++) {
            bytes32 messageId = keccak256(abi.encodePacked("batch-transfer", i));
            bytes memory payload = abi.encodeWithSelector(usdc.transfer.selector, recipients[i], amountEach);
            SequencerSignature memory sig = createSequencerSignature(messageId, address(usdc), payload, 0);

            vm.prank(sequencer);
            bridge.initializeMessage(messageId, address(usdc), payload, sig, 0);
            submitValidatorSignatures(bridge, messageId);
            bridge.handleMessage(messageId);
        }

        for (uint256 i = 0; i < recipients.length; i++) {
            assertEq(usdc.balanceOf(recipients[i]), amountEach);
        }

        assertEq(usdc.balanceOf(address(bridge)), 0);
    }

    /*//////////////////////////////////////////////////////////////
                        APPROVALS
    //////////////////////////////////////////////////////////////*/

    function test_ManageApprovals() public {
        uint256 approvalAmount = 1000e18;

        vm.prank(user);
        usdc.transfer(address(bridge), approvalAmount);

        bytes32 approveMessageId = keccak256("approve-test");
        bytes memory approvePayload = abi.encodeWithSelector(usdc.approve.selector, recipient, approvalAmount);

        SequencerSignature memory sig = createSequencerSignature(approveMessageId, address(usdc), approvePayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(approveMessageId, address(usdc), approvePayload, sig, 0);
        submitValidatorSignatures(bridge, approveMessageId);
        bridge.handleMessage(approveMessageId);

        assertEq(usdc.allowance(address(bridge), recipient), approvalAmount);

        vm.prank(recipient);
        usdc.transferFrom(address(bridge), recipient, approvalAmount);

        assertEq(usdc.balanceOf(recipient), approvalAmount);
    }

    /*//////////////////////////////////////////////////////////////
                        EDGE CASES
    //////////////////////////////////////////////////////////////*/

    function test_TransferInsufficientBalance_Reverts() public {
        uint256 bridgeBalance = 100e18;
        uint256 transferAmount = 200e18;

        vm.prank(user);
        usdc.transfer(address(bridge), bridgeBalance);

        bytes32 messageId = keccak256("insufficient-balance");
        bytes memory payload = abi.encodeWithSelector(usdc.transfer.selector, recipient, transferAmount);

        SequencerSignature memory sig = createSequencerSignature(messageId, address(usdc), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(usdc), payload, sig, 0);
        submitValidatorSignatures(bridge, messageId);

        vm.expectRevert();
        bridge.handleMessage(messageId);
    }

    function test_TransferToZeroAddress_Reverts() public {
        uint256 amount = 100e18;

        vm.prank(user);
        usdc.transfer(address(bridge), amount);

        bytes32 messageId = keccak256("zero-address");
        bytes memory payload = abi.encodeWithSelector(usdc.transfer.selector, address(0), amount);

        SequencerSignature memory sig = createSequencerSignature(messageId, address(usdc), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(usdc), payload, sig, 0);
        submitValidatorSignatures(bridge, messageId);

        vm.expectRevert();
        bridge.handleMessage(messageId);
    }

    function test_TransferZeroAmount() public {
        vm.prank(user);
        usdc.transfer(address(bridge), 1000e18);

        bytes32 messageId = keccak256("zero-amount");
        bytes memory payload = abi.encodeWithSelector(usdc.transfer.selector, recipient, 0);

        SequencerSignature memory sig = createSequencerSignature(messageId, address(usdc), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(usdc), payload, sig, 0);
        submitValidatorSignatures(bridge, messageId);
        bridge.handleMessage(messageId);

        assertEq(usdc.balanceOf(recipient), 0);
    }

    /*//////////////////////////////////////////////////////////////
                        INTEGRATION
    //////////////////////////////////////////////////////////////*/

    function test_CompleteDepositTransferFlow() public {
        uint256 depositAmount = 1000e18;
        uint256 transferAmount = 600e18;
        uint256 remainingAmount = depositAmount - transferAmount;

        vm.prank(user);
        usdc.transfer(address(bridge), depositAmount);

        assertEq(usdc.balanceOf(address(bridge)), depositAmount);

        // First transfer to recipient
        bytes32 transferMessageId = keccak256("complete-transfer");
        bytes memory transferPayload = abi.encodeWithSelector(usdc.transfer.selector, recipient, transferAmount);
        SequencerSignature memory transferSig =
            createSequencerSignature(transferMessageId, address(usdc), transferPayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(transferMessageId, address(usdc), transferPayload, transferSig, 0);
        submitValidatorSignatures(bridge, transferMessageId);
        bridge.handleMessage(transferMessageId);

        assertEq(usdc.balanceOf(recipient), transferAmount);

        // Return remaining to user
        bytes32 returnMessageId = keccak256("complete-return");
        bytes memory returnPayload = abi.encodeWithSelector(usdc.transfer.selector, user, remainingAmount);
        SequencerSignature memory returnSig = createSequencerSignature(returnMessageId, address(usdc), returnPayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(returnMessageId, address(usdc), returnPayload, returnSig, 0);
        submitValidatorSignatures(bridge, returnMessageId);
        bridge.handleMessage(returnMessageId);

        assertEq(usdc.balanceOf(user), INITIAL_BALANCE - transferAmount);
        assertEq(usdc.balanceOf(recipient), transferAmount);
        assertEq(usdc.balanceOf(address(bridge)), 0);
    }

    receive() external payable {}
}
