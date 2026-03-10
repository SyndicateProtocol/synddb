// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Bridge} from "src/Bridge.sol";
import {SequencerSignature} from "src/types/DataTypes.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {WETH9} from "./mocks/WETH9.sol";
import {MockNFT} from "./mocks/MockNFT.sol";
import {UseCaseBaseTest} from "./base/UseCaseBaseTest.sol";

/**
 * @title NFTMintingTest
 * @notice Tests for NFT minting through the bridge with validator signature verification
 */
contract NFTMintingTest is UseCaseBaseTest {
    Bridge public bridge;
    WETH9 public weth;
    MockNFT public freeNFT;
    MockNFT public paidNFT;
    ValidatorSignatureThresholdModule public validatorModule;

    address public sequencer;
    address public user;

    uint256 public constant NFT_PRICE = 0.1 ether;

    function setUp() public {
        sequencer = vm.addr(sequencerPrivateKey);
        user = makeAddr("user");

        (bridge, weth) = createBridgeWithWETH(address(this), sequencer);

        freeNFT = new MockNFT("Free NFT", "FREE", 0);
        paidNFT = new MockNFT("Paid NFT", "PAID", NFT_PRICE);

        validatorModule = setupBridgeWithValidators(bridge);

        vm.deal(user, 100 ether);
    }

    /*//////////////////////////////////////////////////////////////
                        FREE NFT MINTING
    //////////////////////////////////////////////////////////////*/

    function test_MintFreeNFT() public {
        bytes32 messageId = keccak256("free-mint-1");
        bytes memory payload = abi.encodeWithSelector(freeNFT.mint.selector, user);
        SequencerSignature memory sig = createSequencerSignature(bridge, messageId, address(freeNFT), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(freeNFT), payload, sig, 0, 0);

        submitValidatorSignatures(bridge, messageId);

        bridge.handleMessage(messageId, payload);

        assertEq(freeNFT.ownerOf(0), user);
        assertEq(freeNFT.balanceOf(user), 1);
    }

    function test_MintMultipleFreeNFTs() public {
        uint256 mintCount = 5;

        for (uint256 i = 0; i < mintCount; i++) {
            bytes32 messageId = keccak256(abi.encodePacked("free-mint", i));
            bytes memory payload = abi.encodeWithSelector(freeNFT.mint.selector, user);
            SequencerSignature memory sig = createSequencerSignature(bridge, messageId, address(freeNFT), payload, 0);

            vm.prank(sequencer);
            bridge.initializeMessage(messageId, address(freeNFT), payload, sig, 0, 0);

            submitValidatorSignatures(bridge, messageId);

            bridge.handleMessage(messageId, payload);
        }

        assertEq(freeNFT.balanceOf(user), mintCount);

        for (uint256 i = 0; i < mintCount; i++) {
            assertEq(freeNFT.ownerOf(i), user);
        }
    }

    function test_BatchMintFreeNFTs() public {
        address[] memory recipients = new address[](3);
        recipients[0] = makeAddr("recipient1");
        recipients[1] = makeAddr("recipient2");
        recipients[2] = makeAddr("recipient3");

        for (uint256 i = 0; i < recipients.length; i++) {
            bytes32 messageId = keccak256(abi.encodePacked("batch-mint", i));
            bytes memory payload = abi.encodeWithSelector(freeNFT.mint.selector, recipients[i]);
            SequencerSignature memory sig = createSequencerSignature(bridge, messageId, address(freeNFT), payload, 0);

            vm.prank(sequencer);
            bridge.initializeMessage(messageId, address(freeNFT), payload, sig, 0, 0);

            submitValidatorSignatures(bridge, messageId);

            bridge.handleMessage(messageId, payload);
        }

        for (uint256 i = 0; i < recipients.length; i++) {
            assertEq(freeNFT.balanceOf(recipients[i]), 1);
            assertEq(freeNFT.ownerOf(i), recipients[i]);
        }
    }

    /*//////////////////////////////////////////////////////////////
                        PAID NFT MINTING
    //////////////////////////////////////////////////////////////*/

    function test_MintPaidNFTWithWETH() public {
        vm.startPrank(user);
        weth.deposit{value: NFT_PRICE}();
        weth.transfer(address(bridge), NFT_PRICE);
        vm.stopPrank();

        bytes32 approveMessageId = keccak256("approve-weth");
        bytes memory approvePayload = abi.encodeWithSelector(weth.approve.selector, address(paidNFT), NFT_PRICE);
        SequencerSignature memory approveSig =
            createSequencerSignature(bridge, approveMessageId, address(weth), approvePayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(approveMessageId, address(weth), approvePayload, approveSig, 0, 0);

        submitValidatorSignatures(bridge, approveMessageId);

        bridge.handleMessage(approveMessageId, approvePayload);

        bytes32 mintMessageId = keccak256("paid-mint-1");
        bytes memory mintPayload = abi.encodeWithSelector(paidNFT.mintWithWETH.selector, user, address(weth), NFT_PRICE);
        SequencerSignature memory mintSig =
            createSequencerSignature(bridge, mintMessageId, address(paidNFT), mintPayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(mintMessageId, address(paidNFT), mintPayload, mintSig, 0, 0);

        submitValidatorSignatures(bridge, mintMessageId);

        bridge.handleMessage(mintMessageId, mintPayload);

        assertEq(paidNFT.ownerOf(0), user);
        assertEq(paidNFT.balanceOf(user), 1);
        assertEq(weth.balanceOf(address(paidNFT)), NFT_PRICE);
    }

    function test_MintPaidNFTWithInsufficientWrappedNativeToken_Reverts() public {
        uint256 insufficientAmount = NFT_PRICE - 0.01 ether;

        vm.startPrank(user);
        weth.deposit{value: insufficientAmount}();
        weth.transfer(address(bridge), insufficientAmount);
        vm.stopPrank();

        bytes32 approveMessageId = keccak256("approve-insufficient");
        bytes memory approvePayload =
            abi.encodeWithSelector(weth.approve.selector, address(paidNFT), insufficientAmount);
        SequencerSignature memory approveSig =
            createSequencerSignature(bridge, approveMessageId, address(weth), approvePayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(approveMessageId, address(weth), approvePayload, approveSig, 0, 0);

        submitValidatorSignatures(bridge, approveMessageId);

        bridge.handleMessage(approveMessageId, approvePayload);

        bytes32 mintMessageId = keccak256("paid-mint-insufficient");
        bytes memory mintPayload =
            abi.encodeWithSelector(paidNFT.mintWithWETH.selector, user, address(weth), insufficientAmount);
        SequencerSignature memory mintSig =
            createSequencerSignature(bridge, mintMessageId, address(paidNFT), mintPayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(mintMessageId, address(paidNFT), mintPayload, mintSig, 0, 0);

        submitValidatorSignatures(bridge, mintMessageId);

        vm.expectRevert();
        bridge.handleMessage(mintMessageId, mintPayload);
    }

    /// @notice Test minting NFT with native ETH
    /// @dev This tests the harder case: Bridge holds WETH, needs to unwrap to ETH for NFT payment
    function test_MintPaidNFTWithETH() public {
        // User deposits ETH into bridge (gets wrapped to WETH)
        vm.prank(user);
        (bool success,) = address(bridge).call{value: NFT_PRICE}("");
        assertTrue(success);
        assertEq(weth.balanceOf(address(bridge)), NFT_PRICE);

        // Bridge calls paidNFT to unwrap WETH and mint NFT with ETH
        bytes32 messageId = keccak256("withdraw-and-mint");
        bytes memory payload = abi.encodeWithSelector(paidNFT.mintWithPayment.selector, user);
        SequencerSignature memory sig =
            createSequencerSignature(bridge, messageId, address(paidNFT), payload, NFT_PRICE);

        // Bridge calls paidNFT with ETH payment
        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(paidNFT), payload, sig, NFT_PRICE, 0);
        submitValidatorSignatures(bridge, messageId);
        bridge.handleMessage(messageId, payload);

        // Verify NFT was minted to user
        assertEq(paidNFT.ownerOf(0), user);
        assertEq(paidNFT.balanceOf(user), 1);

        // Verify NFT contract received ETH payment (not WETH)
        assertEq(address(paidNFT).balance, NFT_PRICE);
        assertEq(weth.balanceOf(address(paidNFT)), 0);

        // Verify bridge unwrapped all WETH (User deposited NFT_PRICE, bridge unwrapped NFT_PRICE)
        assertEq(weth.balanceOf(address(bridge)), 0);
    }

    /*//////////////////////////////////////////////////////////////
                        EDGE CASES
    //////////////////////////////////////////////////////////////*/

    function test_MintToZeroAddress_Reverts() public {
        bytes32 messageId = keccak256("mint-zero");
        bytes memory payload = abi.encodeWithSelector(freeNFT.mint.selector, address(0));
        SequencerSignature memory sig = createSequencerSignature(bridge, messageId, address(freeNFT), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(freeNFT), payload, sig, 0, 0);

        submitValidatorSignatures(bridge, messageId);

        vm.expectRevert();
        bridge.handleMessage(messageId, payload);
    }

    function test_BridgeIsMsgSender() public {
        bytes32 messageId = keccak256("sender-test");
        bytes memory payload = abi.encodeWithSelector(freeNFT.mint.selector, user);
        SequencerSignature memory sig = createSequencerSignature(bridge, messageId, address(freeNFT), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(freeNFT), payload, sig, 0, 0);

        submitValidatorSignatures(bridge, messageId);

        bridge.handleMessage(messageId, payload);

        assertEq(freeNFT.balanceOf(user), 1);
    }

    receive() external payable {}
}
