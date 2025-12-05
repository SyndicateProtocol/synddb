// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Bridge} from "src/Bridge.sol";
import {SequencerSignature} from "src/types/DataTypes.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {MockERC20} from "./mocks/MockERC20.sol";
import {WETH9} from "./mocks/WETH9.sol";
import {UseCaseBaseTest} from "./base/UseCaseBaseTest.sol";

/**
 * @title UseCase3_ERC20Transfer
 * @notice Tests for ERC-20 token transfers through the bridge with validator signatures
 */
contract UseCase3_ERC20Transfer is UseCaseBaseTest {
    Bridge public bridge;
    MockERC20 public usdc;
    MockERC20 public dai;
    WETH9 public weth;
    ValidatorSignatureThresholdModule public validatorModule;

    address public admin;
    address public sequencer;
    address public user;
    address public recipient;

    uint256 public constant INITIAL_BALANCE = 1_000_000e18;

    function setUp() public {
        admin = address(this);
        sequencer = makeAddr("sequencer");
        user = makeAddr("user");
        recipient = makeAddr("recipient");

        usdc = new MockERC20("USD Coin", "USDC");
        dai = new MockERC20("Dai Stablecoin", "DAI");
        weth = new WETH9();
        bridge = new Bridge(admin, address(weth));

        setupValidators(bridge);
        validatorModule = new ValidatorSignatureThresholdModule(address(bridge), validators, 2);

        bridge.grantRole(bridge.MESSAGE_INITIALIZER_ROLE(), sequencer);
        bridge.addPreModule(address(validatorModule));

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

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

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

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

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

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.startPrank(sequencer);

        bytes32 usdcMessageId = keccak256("multi-usdc");
        bridge.initializeMessage(
            usdcMessageId, address(usdc), abi.encodeWithSelector(usdc.transfer.selector, recipient, usdcAmount), sig, 0
        );
        vm.stopPrank();
        submitValidatorSignatures(bridge, usdcMessageId);
        bridge.handleMessage(usdcMessageId);

        vm.prank(sequencer);
        bytes32 daiMessageId = keccak256("multi-dai");
        bridge.initializeMessage(
            daiMessageId, address(dai), abi.encodeWithSelector(dai.transfer.selector, recipient, daiAmount), sig, 0
        );
        submitValidatorSignatures(bridge, daiMessageId);
        bridge.handleMessage(daiMessageId);

        vm.prank(sequencer);
        bytes32 wethMessageId = keccak256("multi-weth");
        bridge.initializeMessage(
            wethMessageId, address(weth), abi.encodeWithSelector(weth.transfer.selector, recipient, wethAmount), sig, 0
        );
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

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        for (uint256 i = 0; i < recipients.length; i++) {
            bytes32 messageId = keccak256(abi.encodePacked("batch-transfer", i));
            bytes memory payload = abi.encodeWithSelector(usdc.transfer.selector, recipients[i], amountEach);

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

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

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

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

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

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

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

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

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

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        bytes32 transferMessageId = keccak256("complete-transfer");
        vm.prank(sequencer);
        bridge.initializeMessage(
            transferMessageId,
            address(usdc),
            abi.encodeWithSelector(usdc.transfer.selector, recipient, transferAmount),
            sig,
            0
        );
        submitValidatorSignatures(bridge, transferMessageId);
        bridge.handleMessage(transferMessageId);

        assertEq(usdc.balanceOf(recipient), transferAmount);

        bytes32 returnMessageId = keccak256("complete-return");
        vm.prank(sequencer);
        bridge.initializeMessage(
            returnMessageId,
            address(usdc),
            abi.encodeWithSelector(usdc.transfer.selector, user, remainingAmount),
            sig,
            0
        );
        submitValidatorSignatures(bridge, returnMessageId);
        bridge.handleMessage(returnMessageId);

        assertEq(usdc.balanceOf(user), INITIAL_BALANCE - transferAmount);
        assertEq(usdc.balanceOf(recipient), transferAmount);
        assertEq(usdc.balanceOf(address(bridge)), 0);
    }

    /*//////////////////////////////////////////////////////////////
                    VALIDATOR SIGNATURE TESTS
    //////////////////////////////////////////////////////////////*/

    /// @notice Test ERC20 transfer fails without sufficient validator signatures
    function test_ERC20Transfer_FailsWithInsufficientSignatures() public {
        uint256 transferAmount = 1000e18;

        vm.prank(user);
        usdc.transfer(address(bridge), transferAmount);

        bytes32 messageId = keccak256("erc20-insufficient");
        bytes memory payload = abi.encodeWithSelector(usdc.transfer.selector, recipient, transferAmount);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(usdc), payload, sig, 0);

        // Only submit 1 signature (threshold is 2)
        submitValidatorSignatures(bridge, messageId, 1);

        vm.expectRevert();
        bridge.handleMessage(messageId);
    }

    /// @notice Test ERC20 transfer succeeds with exact threshold
    function test_ERC20Transfer_SucceedsWithExactThreshold() public {
        uint256 transferAmount = 500e18;

        vm.prank(user);
        usdc.transfer(address(bridge), transferAmount);

        bytes32 messageId = keccak256("erc20-exact");
        bytes memory payload = abi.encodeWithSelector(usdc.transfer.selector, recipient, transferAmount);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(usdc), payload, sig, 0);

        // Submit exactly 2 signatures
        submitValidatorSignatures(bridge, messageId, 2);

        bridge.handleMessage(messageId);

        assertEq(usdc.balanceOf(recipient), transferAmount);
    }

    receive() external payable {}
}
