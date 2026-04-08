// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {UseCaseBaseTest} from "./base/UseCaseBaseTest.sol";
import {Bridge} from "src/Bridge.sol";
import {SequencerSignature, KeyType} from "src/types/DataTypes.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {TeeKeyManager} from "src/attestation/TeeKeyManager.sol";
import {MockAttestationVerifier} from "src/attestation/MockAttestationVerifier.sol";
import {MockONFT} from "./mocks/MockONFT.sol";
import {MockCrossChainReceiver} from "./mocks/MockCrossChainReceiver.sol";
import {WETH9} from "./mocks/WETH9.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/**
 * @title CrossChainNFTTest
 * @notice Tests cross-chain NFT transfers using ONFT (LayerZero) standard
 * @dev Demonstrates burn-and-mint pattern for omnichain NFTs
 */
contract CrossChainNFTTest is UseCaseBaseTest {
    Bridge public sourceBridge;
    Bridge public destBridge;
    ValidatorSignatureThresholdModule public validatorModule;
    MockONFT public onft;
    MockCrossChainReceiver public receiver;

    address public sequencer;
    address public user = address(0x456);

    // We need a separate sequencer private key for destBridge since it doesn't have validators
    uint256 public destSequencerPrivateKey = 0xB22CE;

    function setUp() public {
        sequencer = vm.addr(sequencerPrivateKey);
        address destSequencer = vm.addr(destSequencerPrivateKey);

        // Setup source chain bridge with validators
        (sourceBridge,) = createBridgeWithWETH(address(this), sequencer);
        validatorModule = setupBridgeWithValidators(sourceBridge);

        // Setup destination chain bridge (simpler, no validators needed for this test)
        // We need to create a separate TEE key manager for destBridge
        MockAttestationVerifier destAttestationVerifier = new MockAttestationVerifier();
        TeeKeyManager destTeeKeyManager = new TeeKeyManager(destAttestationVerifier);

        WETH9 destWeth = new WETH9();
        destBridge = new Bridge(address(this), address(destWeth), address(destTeeKeyManager));

        // Set bridge on destTeeKeyManager
        destTeeKeyManager.setBridge(address(destBridge));

        // Register dest sequencer as a valid TEE key through bridge
        bytes memory destPublicValues = abi.encode(destSequencer);
        destBridge.registerKey(KeyType.Sequencer, destPublicValues, "");

        destBridge.setMessageInitializer(destSequencer, true);

        // Deploy ONFT with source bridge as the authorized bridge
        onft = new MockONFT("Omnichain NFT", "ONFT", address(sourceBridge));

        // Deploy cross-chain receiver with dest bridge as authorized bridge
        receiver = new MockCrossChainReceiver();

        // Mint initial NFT to user
        onft.mint(user);
        assertEq(onft.ownerOf(0), user);
    }

    /// @notice Helper to create sequencer signature for destBridge (uses destSequencerPrivateKey)
    function createDestSequencerSignature(bytes32 messageId, address targetAddress, bytes memory payload)
        internal
        view
        returns (SequencerSignature memory)
    {
        uint256 nonce = destBridge.sequencerNonces(vm.addr(destSequencerPrivateKey));
        bytes32 messageHash = keccak256(
            abi.encodePacked(block.chainid, messageId, targetAddress, keccak256(payload), uint256(0), uint256(0), nonce)
        );
        bytes32 ethSignedHash = MessageHashUtils.toEthSignedMessageHash(messageHash);
        (uint8 v, bytes32 r, bytes32 s) = vm.sign(destSequencerPrivateKey, ethSignedHash);
        return SequencerSignature({signature: abi.encodePacked(r, s, v), submittedAt: block.timestamp});
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
        SequencerSignature memory burnSig =
            createSequencerSignature(sourceBridge, burnMessageId, address(onft), burnPayload, 0);

        // Step 3: Execute burn on source chain
        vm.prank(sequencer);
        sourceBridge.initializeMessage(burnMessageId, address(onft), burnPayload, burnSig, 0, 0);
        submitValidatorSignatures(sourceBridge, burnMessageId);
        sourceBridge.handleMessage(burnMessageId, burnPayload);

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

        // Sequencer signature for mint (different from burn)
        SequencerSignature memory mintSig = createDestSequencerSignature(mintMessageId, address(receiver), mintPayload);

        // Execute mint on destination chain (no validators needed for this simple test)
        vm.prank(vm.addr(destSequencerPrivateKey));
        destBridge.initializeMessage(mintMessageId, address(receiver), mintPayload, mintSig, 0, 0);
        destBridge.handleMessage(mintMessageId, mintPayload);

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

        SequencerSignature memory sig = createSequencerSignature(sourceBridge, messageId, address(onft), payload, 0);

        vm.prank(sequencer);
        sourceBridge.initializeMessage(messageId, address(onft), payload, sig, 0, 0);
        submitValidatorSignatures(sourceBridge, messageId);

        // Should revert because attacker doesn't own the token
        vm.expectRevert();
        sourceBridge.handleMessage(messageId, payload);
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
        SequencerSignature memory sig1 =
            createSequencerSignature(sourceBridge, burnMsg1, address(sourceOnft), burnPayload1, 0);

        vm.prank(sequencer);
        sourceBridge.initializeMessage(burnMsg1, address(sourceOnft), burnPayload1, sig1, 0, 0);
        submitValidatorSignatures(sourceBridge, burnMsg1);
        sourceBridge.handleMessage(burnMsg1, burnPayload1);

        // Verify burned on source
        vm.expectRevert();
        sourceOnft.ownerOf(tokenId);

        // Step 2: Mint on destination
        bytes32 mintMsg = keccak256("mint-dest");
        bytes memory mintPayload = abi.encodeWithSelector(destOnft.crosschainMint.selector, user, tokenId);
        SequencerSignature memory mintSig = createDestSequencerSignature(mintMsg, address(destOnft), mintPayload);

        vm.prank(vm.addr(destSequencerPrivateKey));
        destBridge.initializeMessage(mintMsg, address(destOnft), mintPayload, mintSig, 0, 0);
        destBridge.handleMessage(mintMsg, mintPayload);

        assertEq(destOnft.ownerOf(tokenId), user);

        // Step 3: Burn on destination (return trip)
        vm.prank(user);
        destOnft.approve(address(destBridge), tokenId);

        bytes32 burnMsg2 = keccak256("burn-2");
        bytes memory burnPayload2 = abi.encodeWithSelector(destOnft.crosschainBurn.selector, user, tokenId);
        SequencerSignature memory burnSig2 = createDestSequencerSignature(burnMsg2, address(destOnft), burnPayload2);

        vm.prank(vm.addr(destSequencerPrivateKey));
        destBridge.initializeMessage(burnMsg2, address(destOnft), burnPayload2, burnSig2, 0, 0);
        destBridge.handleMessage(burnMsg2, burnPayload2);

        // Verify burned on dest
        vm.expectRevert();
        destOnft.ownerOf(tokenId);

        // Step 4: Re-mint on source
        bytes32 mintMsg2 = keccak256("mint-source");
        bytes memory mintPayload2 = abi.encodeWithSelector(sourceOnft.crosschainMint.selector, user, tokenId);
        SequencerSignature memory mintSig2 =
            createSequencerSignature(sourceBridge, mintMsg2, address(sourceOnft), mintPayload2, 0);

        vm.prank(sequencer);
        sourceBridge.initializeMessage(mintMsg2, address(sourceOnft), mintPayload2, mintSig2, 0, 0);
        submitValidatorSignatures(sourceBridge, mintMsg2);
        sourceBridge.handleMessage(mintMsg2, mintPayload2);

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
            SequencerSignature memory sig =
                createSequencerSignature(sourceBridge, messageId, address(batchOnft), payload, 0);

            vm.prank(sequencer);
            sourceBridge.initializeMessage(messageId, address(batchOnft), payload, sig, 0, 0);
            submitValidatorSignatures(sourceBridge, messageId);
            sourceBridge.handleMessage(messageId, payload);
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
        SequencerSignature memory sig =
            createSequencerSignature(sourceBridge, messageId, address(multiOnft), payload, 0);

        vm.prank(sequencer);
        sourceBridge.initializeMessage(messageId, address(multiOnft), payload, sig, 0, 0);
        submitValidatorSignatures(sourceBridge, messageId);
        sourceBridge.handleMessage(messageId, payload);

        // Verify only token 999 was burned
        assertEq(multiOnft.ownerOf(42), user);
        vm.expectRevert();
        multiOnft.ownerOf(999);
    }
}
