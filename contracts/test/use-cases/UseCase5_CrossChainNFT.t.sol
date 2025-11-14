// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {UseCaseBaseTest} from "./base/UseCaseBaseTest.sol";
import {Bridge} from "src/Bridge.sol";
import {SequencerSignature} from "src/types/DataTypes.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {MockONFT} from "./mocks/MockONFT.sol";
import {MockCrossChainReceiver} from "./mocks/MockCrossChainReceiver.sol";

/**
 * @title UseCase5_CrossChainNFT
 * @notice Tests cross-chain NFT transfers using ONFT (LayerZero) standard
 * @dev Demonstrates burn-and-mint pattern for omnichain NFTs
 */
contract UseCase5_CrossChainNFT is UseCaseBaseTest {
    Bridge public sourceBridge;
    Bridge public destBridge;
    ValidatorSignatureThresholdModule public validatorModule;
    MockONFT public onft;
    MockCrossChainReceiver public receiver;

    address public sequencer = address(0x123);
    address public user = address(0x456);

    function setUp() public {
        // Setup source chain bridge with validators
        (sourceBridge,) = createBridgeWithWETH(address(this), sequencer);
        validatorModule = setupBridgeWithValidators(sourceBridge);

        // Setup destination chain bridge (simpler, no validators needed for this test)
        (destBridge,) = createBridgeWithWETH(address(this), sequencer);

        // Deploy ONFT with source bridge as the authorized bridge
        onft = new MockONFT("Omnichain NFT", "ONFT", address(sourceBridge));

        // Deploy cross-chain receiver with dest bridge as authorized bridge
        receiver = new MockCrossChainReceiver();

        // Mint initial NFT to user
        onft.mint(user);
        assertEq(onft.ownerOf(0), user);
    }

    /// @notice Test basic cross-chain NFT transfer via burn-and-mint
    function test_CrossChainNFTTransfer() public {
        uint256 tokenId = 0;
        address destinationUser = address(0x789);

        // Step 1: User approves bridge to burn their NFT
        vm.prank(user);
        onft.approve(address(sourceBridge), tokenId);

        // Step 2: Create message to burn NFT on source chain
        bytes32 burnMessageId = keccak256("burn-nft");
        bytes memory burnPayload = abi.encodeWithSelector(onft.crosschainBurn.selector, user, tokenId);

        // Sequencer signature for burn
        SequencerSignature memory burnSig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        // Step 3: Execute burn on source chain
        vm.prank(sequencer);
        sourceBridge.initializeMessage(burnMessageId, address(onft), burnPayload, burnSig);
        submitValidatorSignatures(sourceBridge, burnMessageId);
        sourceBridge.handleMessage(burnMessageId);

        // Verify NFT was burned
        vm.expectRevert();
        onft.ownerOf(tokenId);

        // Step 4: Simulate cross-chain message to destination chain
        // Deploy new ONFT on destination chain with dest bridge authorization
        MockONFT destOnft = new MockONFT("Omnichain NFT", "ONFT", address(receiver));

        bytes32 mintMessageId = keccak256("mint-nft-destination");
        bytes memory mintPayload = abi.encodeWithSelector(
            receiver.receiveONFTMint.selector, mintMessageId, address(destOnft), destinationUser, tokenId
        );

        // Execute mint on destination chain (no validators needed for this simple test)
        vm.prank(sequencer);
        destBridge.initializeMessage(mintMessageId, address(receiver), mintPayload, burnSig);
        destBridge.handleMessage(mintMessageId);

        // Verify NFT was minted on destination chain
        assertEq(destOnft.ownerOf(tokenId), destinationUser);
    }

    /// @notice Test burning NFT without ownership fails
    function test_CrossChainBurn_FailsWithoutOwnership() public {
        uint256 tokenId = 0;
        address attacker = address(0x999);

        // Attacker tries to burn user's NFT
        bytes32 messageId = keccak256("malicious-burn");
        bytes memory payload = abi.encodeWithSelector(onft.crosschainBurn.selector, attacker, tokenId);

        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        sourceBridge.initializeMessage(messageId, address(onft), payload, sig);
        submitValidatorSignatures(sourceBridge, messageId);

        // Should revert because attacker doesn't own the token
        vm.expectRevert();
        sourceBridge.handleMessage(messageId);
    }

    /// @notice Test only authorized bridge can mint ONFT
    function test_CrossChainMint_OnlyBridge() public {
        address unauthorized = address(0x999);

        vm.prank(unauthorized);
        vm.expectRevert(MockONFT.OnlyCrossChainBridge.selector);
        onft.crosschainMint(user, 99);
    }

    /// @notice Test only authorized bridge can burn ONFT
    function test_CrossChainBurn_OnlyBridge() public {
        address unauthorized = address(0x999);

        vm.prank(unauthorized);
        vm.expectRevert(MockONFT.OnlyCrossChainBridge.selector);
        onft.crosschainBurn(user, 0);
    }

    /// @notice Test complete round-trip: source -> destination -> back to source
    function test_RoundTripNFTTransfer() public {
        uint256 tokenId = 0;

        // Deploy ONFT contracts on both chains
        MockONFT sourceOnft = new MockONFT("Source ONFT", "SONFT", address(sourceBridge));
        MockONFT destOnft = new MockONFT("Dest ONFT", "DONFT", address(destBridge));

        // Mint on source
        sourceOnft.mint(user);
        assertEq(sourceOnft.ownerOf(tokenId), user);

        // Step 1: Burn on source
        vm.prank(user);
        sourceOnft.approve(address(sourceBridge), tokenId);

        bytes32 burnMsg1 = keccak256("burn-1");
        bytes memory burnPayload1 = abi.encodeWithSelector(sourceOnft.crosschainBurn.selector, user, tokenId);
        SequencerSignature memory sig1 = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        sourceBridge.initializeMessage(burnMsg1, address(sourceOnft), burnPayload1, sig1);
        submitValidatorSignatures(sourceBridge, burnMsg1);
        sourceBridge.handleMessage(burnMsg1);

        // Verify burned on source
        vm.expectRevert();
        sourceOnft.ownerOf(tokenId);

        // Step 2: Mint on destination
        bytes32 mintMsg = keccak256("mint-dest");
        bytes memory mintPayload = abi.encodeWithSelector(destOnft.crosschainMint.selector, user, tokenId);

        vm.prank(sequencer);
        destBridge.initializeMessage(mintMsg, address(destOnft), mintPayload, sig1);
        destBridge.handleMessage(mintMsg);

        assertEq(destOnft.ownerOf(tokenId), user);

        // Step 3: Burn on destination (return trip)
        vm.prank(user);
        destOnft.approve(address(destBridge), tokenId);

        bytes32 burnMsg2 = keccak256("burn-2");
        bytes memory burnPayload2 = abi.encodeWithSelector(destOnft.crosschainBurn.selector, user, tokenId);

        vm.prank(sequencer);
        destBridge.initializeMessage(burnMsg2, address(destOnft), burnPayload2, sig1);
        destBridge.handleMessage(burnMsg2);

        // Verify burned on dest
        vm.expectRevert();
        destOnft.ownerOf(tokenId);

        // Step 4: Re-mint on source
        bytes32 mintMsg2 = keccak256("mint-source");
        bytes memory mintPayload2 = abi.encodeWithSelector(sourceOnft.crosschainMint.selector, user, tokenId);

        vm.prank(sequencer);
        sourceBridge.initializeMessage(mintMsg2, address(sourceOnft), mintPayload2, sig1);
        submitValidatorSignatures(sourceBridge, mintMsg2);
        sourceBridge.handleMessage(mintMsg2);

        // Verify NFT is back on source chain with original owner
        assertEq(sourceOnft.ownerOf(tokenId), user);
    }

    /// @notice Test batch cross-chain NFT transfers
    function test_BatchCrossChainNFTTransfer() public {
        MockONFT batchOnft = new MockONFT("Batch ONFT", "BONFT", address(sourceBridge));

        // Mint 3 NFTs to user
        batchOnft.mint(user);
        batchOnft.mint(user);
        batchOnft.mint(user);

        assertEq(batchOnft.ownerOf(0), user);
        assertEq(batchOnft.ownerOf(1), user);
        assertEq(batchOnft.ownerOf(2), user);

        // Approve all for bridge
        vm.startPrank(user);
        batchOnft.approve(address(sourceBridge), 0);
        batchOnft.approve(address(sourceBridge), 1);
        batchOnft.approve(address(sourceBridge), 2);
        vm.stopPrank();

        // Burn all three in separate messages
        for (uint256 i = 0; i < 3; i++) {
            bytes32 messageId = keccak256(abi.encodePacked("burn-batch", i));
            bytes memory payload = abi.encodeWithSelector(batchOnft.crosschainBurn.selector, user, i);
            SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

            vm.prank(sequencer);
            sourceBridge.initializeMessage(messageId, address(batchOnft), payload, sig);
            submitValidatorSignatures(sourceBridge, messageId);
            sourceBridge.handleMessage(messageId);
        }

        // Verify all burned
        for (uint256 i = 0; i < 3; i++) {
            vm.expectRevert();
            batchOnft.ownerOf(i);
        }
    }

    /// @notice Test cross-chain NFT transfer with different token IDs
    function test_CrossChainTransfer_DifferentTokenIds() public {
        MockONFT multiOnft = new MockONFT("Multi ONFT", "MONFT", address(sourceBridge));

        // Mint tokens with specific IDs
        multiOnft.mintTokenId(user, 42);
        multiOnft.mintTokenId(user, 999);

        assertEq(multiOnft.ownerOf(42), user);
        assertEq(multiOnft.ownerOf(999), user);

        // Transfer token 999
        vm.prank(user);
        multiOnft.approve(address(sourceBridge), 999);

        bytes32 messageId = keccak256("burn-999");
        bytes memory payload = abi.encodeWithSelector(multiOnft.crosschainBurn.selector, user, 999);
        SequencerSignature memory sig = SequencerSignature({signature: new bytes(65), submittedAt: block.timestamp});

        vm.prank(sequencer);
        sourceBridge.initializeMessage(messageId, address(multiOnft), payload, sig);
        submitValidatorSignatures(sourceBridge, messageId);
        sourceBridge.handleMessage(messageId);

        // Verify only token 999 was burned
        assertEq(multiOnft.ownerOf(42), user);
        vm.expectRevert();
        multiOnft.ownerOf(999);
    }
}
