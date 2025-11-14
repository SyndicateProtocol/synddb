// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Bridge} from "src/Bridge.sol";
import {SequencerSignature} from "src/types/DataTypes.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {WETH9} from "./mocks/WETH9.sol";
import {MockNFT} from "./mocks/MockNFT.sol";
import {UseCaseBaseTest} from "./base/UseCaseBaseTest.sol";

/**
 * @title UseCase2_NFTMinting
 * @notice Tests for NFT minting through the bridge with validator signature verification
 */
contract UseCase2_NFTMinting is UseCaseBaseTest {
    Bridge public bridge;
    WETH9 public weth;
    MockNFT public freeNFT;
    MockNFT public paidNFT;
    ValidatorSignatureThresholdModule public validatorModule;

    address public admin;
    address public sequencer;
    address public user;

    uint256 public constant NFT_PRICE = 0.1 ether;

    function setUp() public {
        admin = address(this);
        sequencer = makeAddr("sequencer");
        user = makeAddr("user");

        weth = new WETH9();
        bridge = new Bridge(admin, address(weth));
        freeNFT = new MockNFT("Free NFT", "FREE", 0);
        paidNFT = new MockNFT("Paid NFT", "PAID", NFT_PRICE);

        setupValidators(bridge);
        validatorModule = new ValidatorSignatureThresholdModule(address(bridge), validators, 2);

        bridge.grantRole(bridge.SEQUENCER_ROLE(), sequencer);
        bridge.addPreModule(address(validatorModule));

        vm.deal(user, 100 ether);
    }

    /*//////////////////////////////////////////////////////////////
                        FREE NFT MINTING
    //////////////////////////////////////////////////////////////*/

    function test_MintFreeNFT() public {
        bytes32 messageId = keccak256("free-mint-1");

        bytes memory payload = abi.encodeWithSelector(freeNFT.mint.selector, user);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(freeNFT), payload, sig);

        submitValidatorSignatures(bridge, messageId);

        bridge.handleMessage(messageId);

        assertEq(freeNFT.ownerOf(0), user);
        assertEq(freeNFT.balanceOf(user), 1);
    }

    function test_MintMultipleFreeNFTs() public {
        uint256 mintCount = 5;

        for (uint256 i = 0; i < mintCount; i++) {
            bytes32 messageId = keccak256(abi.encodePacked("free-mint", i));

            bytes memory payload = abi.encodeWithSelector(freeNFT.mint.selector, user);

            SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

            vm.prank(sequencer);
            bridge.initializeMessage(messageId, address(freeNFT), payload, sig);

            submitValidatorSignatures(bridge, messageId);

            bridge.handleMessage(messageId);
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

            SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

            vm.prank(sequencer);
            bridge.initializeMessage(messageId, address(freeNFT), payload, sig);

            submitValidatorSignatures(bridge, messageId);

            bridge.handleMessage(messageId);
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

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(approveMessageId, address(weth), approvePayload, sig);

        submitValidatorSignatures(bridge, approveMessageId);

        bridge.handleMessage(approveMessageId);

        bytes32 mintMessageId = keccak256("paid-mint-1");
        bytes memory mintPayload = abi.encodeWithSelector(paidNFT.mintWithWETH.selector, user, address(weth), NFT_PRICE);

        vm.prank(sequencer);
        bridge.initializeMessage(mintMessageId, address(paidNFT), mintPayload, sig);

        submitValidatorSignatures(bridge, mintMessageId);

        bridge.handleMessage(mintMessageId);

        assertEq(paidNFT.ownerOf(0), user);
        assertEq(paidNFT.balanceOf(user), 1);
        assertEq(weth.balanceOf(address(paidNFT)), NFT_PRICE);
    }

    function test_MintPaidNFTWithInsufficientWETH_Reverts() public {
        uint256 insufficientAmount = NFT_PRICE - 0.01 ether;

        vm.startPrank(user);
        weth.deposit{value: insufficientAmount}();
        weth.transfer(address(bridge), insufficientAmount);
        vm.stopPrank();

        bytes32 approveMessageId = keccak256("approve-insufficient");
        bytes memory approvePayload =
            abi.encodeWithSelector(weth.approve.selector, address(paidNFT), insufficientAmount);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(approveMessageId, address(weth), approvePayload, sig);

        submitValidatorSignatures(bridge, approveMessageId);

        bridge.handleMessage(approveMessageId);

        bytes32 mintMessageId = keccak256("paid-mint-insufficient");
        bytes memory mintPayload =
            abi.encodeWithSelector(paidNFT.mintWithWETH.selector, user, address(weth), insufficientAmount);

        vm.prank(sequencer);
        bridge.initializeMessage(mintMessageId, address(paidNFT), mintPayload, sig);

        submitValidatorSignatures(bridge, mintMessageId);

        vm.expectRevert();
        bridge.handleMessage(mintMessageId);
    }

    /// @notice Test minting NFT with native ETH via Bridge.withdrawAndCall
    /// @dev This tests the harder case: Bridge holds WETH, needs to unwrap to ETH for NFT payment
    ///      The Bridge calls itself (self-call) to execute withdrawAndCall
    function test_MintPaidNFTWithETH_ViaWithdrawAndCall() public {
        // User deposits ETH into bridge (gets wrapped to WETH)
        vm.prank(user);
        (bool success,) = address(bridge).call{value: NFT_PRICE}("");
        assertTrue(success);
        assertEq(weth.balanceOf(address(bridge)), NFT_PRICE);

        // Bridge calls itself to unwrap WETH and mint NFT with ETH
        bytes32 messageId = keccak256("withdraw-and-mint");

        // Encode the NFT mint call that requires ETH
        bytes memory nftMintCall = abi.encodeWithSelector(paidNFT.mintWithPayment.selector, user);

        // Encode the Bridge.withdrawAndCall to unwrap WETH and forward ETH to NFT
        bytes memory payload =
            abi.encodeWithSelector(bridge.withdrawAndCall.selector, NFT_PRICE, address(paidNFT), nftMintCall);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        // Bridge calls itself (self-call)
        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(bridge), payload, sig);
        submitValidatorSignatures(bridge, messageId);
        bridge.handleMessage(messageId);

        // Verify NFT was minted to user
        assertEq(paidNFT.ownerOf(0), user);
        assertEq(paidNFT.balanceOf(user), 1);

        // Verify NFT contract received ETH payment (not WETH)
        assertEq(address(paidNFT).balance, NFT_PRICE);
        assertEq(weth.balanceOf(address(paidNFT)), 0);

        // Verify bridge WETH was consumed
        assertEq(weth.balanceOf(address(bridge)), 0);
    }

    /*//////////////////////////////////////////////////////////////
                        EDGE CASES
    //////////////////////////////////////////////////////////////*/

    function test_MintToZeroAddress_Reverts() public {
        bytes32 messageId = keccak256("mint-zero");

        bytes memory payload = abi.encodeWithSelector(freeNFT.mint.selector, address(0));

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(freeNFT), payload, sig);

        submitValidatorSignatures(bridge, messageId);

        vm.expectRevert();
        bridge.handleMessage(messageId);
    }

    function test_BridgeIsMsgSender() public {
        bytes32 messageId = keccak256("sender-test");

        bytes memory payload = abi.encodeWithSelector(freeNFT.mint.selector, user);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(freeNFT), payload, sig);

        submitValidatorSignatures(bridge, messageId);

        bridge.handleMessage(messageId);

        assertEq(freeNFT.balanceOf(user), 1);
    }

    /*//////////////////////////////////////////////////////////////
                    VALIDATOR SIGNATURE TESTS
    //////////////////////////////////////////////////////////////*/

    /// @notice Test NFT mint fails without sufficient validator signatures
    function test_NFTMint_FailsWithInsufficientSignatures() public {
        bytes32 messageId = keccak256("nft-insufficient-sigs");
        bytes memory payload = abi.encodeWithSelector(freeNFT.mint.selector, user);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(freeNFT), payload, sig);

        // Only submit 1 signature (threshold is 2)
        submitValidatorSignatures(bridge, messageId, 1);

        vm.expectRevert();
        bridge.handleMessage(messageId);
    }

    /// @notice Test NFT mint succeeds with exact threshold
    function test_NFTMint_SucceedsWithExactThreshold() public {
        bytes32 messageId = keccak256("nft-exact-threshold");
        bytes memory payload = abi.encodeWithSelector(freeNFT.mint.selector, user);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(freeNFT), payload, sig);

        // Submit exactly 2 signatures
        submitValidatorSignatures(bridge, messageId, 2);

        bridge.handleMessage(messageId);

        assertEq(freeNFT.balanceOf(user), 1);
    }

    receive() external payable {}
}
