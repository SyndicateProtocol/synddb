// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {UseCaseBaseTest} from "./base/UseCaseBaseTest.sol";
import {Bridge} from "src/Bridge.sol";
import {SequencerSignature} from "src/types/DataTypes.sol";
import {ValidatorSignatureThresholdModule} from "src/modules/ValidatorSignatureThresholdModule.sol";
import {MockERC20} from "./mocks/MockERC20.sol";
import {MockCrossChainReceiver} from "./mocks/MockCrossChainReceiver.sol";
import {WETH9} from "./mocks/WETH9.sol";

/**
 * @title CrossChainMessagingTest
 * @notice Tests for cross-chain message passing with arbitrary data
 */
contract CrossChainMessagingTest is UseCaseBaseTest {
    Bridge public bridge;
    WETH9 public weth;
    MockERC20 public token;
    MockCrossChainReceiver public destinationChain;
    ValidatorSignatureThresholdModule public validatorModule;

    address public sequencer;
    address public user;
    address public recipient;

    function setUp() public {
        sequencer = vm.addr(sequencerPrivateKey);
        user = makeAddr("user");
        recipient = makeAddr("recipient");

        token = new MockERC20("Cross Chain Token", "CCT");
        destinationChain = new MockCrossChainReceiver();

        (bridge, weth) = createBridgeWithWETH(address(this), sequencer);
        validatorModule = setupBridgeWithValidators(bridge);

        token.transfer(user, 1_000_000e18);
    }

    /*//////////////////////////////////////////////////////////////
                    SIMPLE CROSS-CHAIN OPERATIONS
    //////////////////////////////////////////////////////////////*/

    function test_CrossChainMint() public {
        uint256 mintAmount = 1000e18;
        bytes32 crossChainMessageId = keccak256("cross-chain-mint-1");

        bytes memory crossChainData = abi.encode("MINT", recipient, mintAmount);

        bytes memory payload = abi.encodeWithSelector(
            destinationChain.receiveMintMessage.selector,
            crossChainMessageId,
            address(token),
            recipient,
            mintAmount,
            crossChainData
        );

        SequencerSignature memory sig =
            createSequencerSignature(crossChainMessageId, address(destinationChain), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(crossChainMessageId, address(destinationChain), payload, sig, 0);
        submitValidatorSignatures(bridge, crossChainMessageId);
        bridge.handleMessage(crossChainMessageId);

        assertEq(token.balanceOf(recipient), mintAmount);
    }

    /*//////////////////////////////////////////////////////////////
                    STRUCTURED DATA MESSAGES
    //////////////////////////////////////////////////////////////*/

    function test_CrossChainMessageWithStructuredData() public {
        bytes32 messageId = keccak256("structured-message-1");
        uint256 amount = 500e18;

        bytes memory structuredData =
            abi.encode("NFT_MINT", recipient, "ipfs://QmExample", uint256(1), block.timestamp + 7 days);

        bytes memory payload = abi.encodeWithSelector(
            destinationChain.receiveMintMessage.selector, messageId, address(token), recipient, amount, structuredData
        );

        SequencerSignature memory sig = createSequencerSignature(messageId, address(destinationChain), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(destinationChain), payload, sig, 0);
        submitValidatorSignatures(bridge, messageId);
        bridge.handleMessage(messageId);

        assertEq(token.balanceOf(recipient), amount);
    }

    /*//////////////////////////////////////////////////////////////
                    MULTI-STEP OPERATIONS
    //////////////////////////////////////////////////////////////*/

    function test_BurnAndMintCrossChain() public {
        uint256 bridgeAmount = 1000e18;

        vm.prank(user);
        token.transfer(address(bridge), bridgeAmount);

        assertEq(token.balanceOf(address(bridge)), bridgeAmount);

        bytes32 messageId = keccak256("burn-mint-1");

        bytes memory crossChainData =
            abi.encode("BRIDGE_MINT", user, recipient, bridgeAmount, block.chainid, block.timestamp);

        bytes memory payload = abi.encodeWithSelector(
            destinationChain.receiveMintMessage.selector,
            messageId,
            address(token),
            recipient,
            bridgeAmount,
            crossChainData
        );

        SequencerSignature memory sig = createSequencerSignature(messageId, address(destinationChain), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(destinationChain), payload, sig, 0);
        submitValidatorSignatures(bridge, messageId);
        bridge.handleMessage(messageId);

        assertEq(token.balanceOf(address(bridge)), bridgeAmount);
        assertEq(token.balanceOf(recipient), bridgeAmount);
    }

    /*//////////////////////////////////////////////////////////////
                    BATCH OPERATIONS
    //////////////////////////////////////////////////////////////*/

    function test_BatchCrossChainMessages() public {
        uint256 messageCount = 5;
        uint256 amountPerMessage = 100e18;

        address[] memory recipients = new address[](messageCount);
        for (uint256 i = 0; i < messageCount; i++) {
            recipients[i] = makeAddr(string(abi.encodePacked("recipient", i)));
        }

        for (uint256 i = 0; i < messageCount; i++) {
            bytes32 messageId = keccak256(abi.encodePacked("batch-cc", i));

            bytes memory crossChainData = abi.encode("BATCH_MINT", i, recipients[i], amountPerMessage);

            bytes memory payload = abi.encodeWithSelector(
                destinationChain.receiveMintMessage.selector,
                messageId,
                address(token),
                recipients[i],
                amountPerMessage,
                crossChainData
            );

            SequencerSignature memory sig = createSequencerSignature(messageId, address(destinationChain), payload, 0);

            vm.prank(sequencer);
            bridge.initializeMessage(messageId, address(destinationChain), payload, sig, 0);
            submitValidatorSignatures(bridge, messageId);
            bridge.handleMessage(messageId);
        }

        for (uint256 i = 0; i < messageCount; i++) {
            assertEq(token.balanceOf(recipients[i]), amountPerMessage);
        }
    }

    /*//////////////////////////////////////////////////////////////
                    METADATA & CALLBACKS
    //////////////////////////////////////////////////////////////*/

    function test_CrossChainWithMetadata() public {
        bytes32 messageId = keccak256("metadata-message");
        uint256 amount = 250e18;

        bytes memory metadata = abi.encode(
            user,
            recipient,
            "USDC",
            block.chainid,
            uint256(137),
            amount,
            block.timestamp,
            uint256(0),
            "Transfer from Ethereum to Polygon"
        );

        bytes memory payload = abi.encodeWithSelector(
            destinationChain.receiveMintMessage.selector, messageId, address(token), recipient, amount, metadata
        );

        SequencerSignature memory sig = createSequencerSignature(messageId, address(destinationChain), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(destinationChain), payload, sig, 0);
        submitValidatorSignatures(bridge, messageId);
        bridge.handleMessage(messageId);

        assertEq(token.balanceOf(recipient), amount);
    }

    function test_CrossChainWithCallback() public {
        bytes32 messageId = keccak256("callback-message");
        uint256 amount = 500e18;

        bytes memory callbackData = abi.encode(
            address(bridge), abi.encodeWithSignature("confirmCrossChainTransfer(bytes32,bool)", messageId, true)
        );

        bytes memory crossChainData = abi.encode("TRANSFER_WITH_CALLBACK", recipient, amount, callbackData);

        bytes memory payload = abi.encodeWithSelector(
            destinationChain.receiveMintMessage.selector, messageId, address(token), recipient, amount, crossChainData
        );

        SequencerSignature memory sig = createSequencerSignature(messageId, address(destinationChain), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(destinationChain), payload, sig, 0);
        submitValidatorSignatures(bridge, messageId);
        bridge.handleMessage(messageId);

        assertEq(token.balanceOf(recipient), amount);
    }

    /*//////////////////////////////////////////////////////////////
                    EDGE CASES
    //////////////////////////////////////////////////////////////*/

    function test_CrossChainWithEmptyData() public {
        bytes32 messageId = keccak256("empty-data");
        uint256 amount = 100e18;
        bytes memory emptyData = "";

        bytes memory payload = abi.encodeWithSelector(
            destinationChain.receiveMintMessage.selector, messageId, address(token), recipient, amount, emptyData
        );

        SequencerSignature memory sig = createSequencerSignature(messageId, address(destinationChain), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(destinationChain), payload, sig, 0);
        submitValidatorSignatures(bridge, messageId);
        bridge.handleMessage(messageId);

        assertEq(token.balanceOf(recipient), amount);
    }

    function test_CrossChainWithLargeData() public {
        bytes32 messageId = keccak256("large-data");
        uint256 amount = 100e18;

        bytes memory largeData = new bytes(4096);
        for (uint256 i = 0; i < largeData.length; i++) {
            largeData[i] = bytes1(uint8(i % 256));
        }

        bytes memory payload = abi.encodeWithSelector(
            destinationChain.receiveMintMessage.selector, messageId, address(token), recipient, amount, largeData
        );

        SequencerSignature memory sig = createSequencerSignature(messageId, address(destinationChain), payload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(messageId, address(destinationChain), payload, sig, 0);
        submitValidatorSignatures(bridge, messageId);
        bridge.handleMessage(messageId);

        assertEq(token.balanceOf(recipient), amount);
    }

    /*//////////////////////////////////////////////////////////////
                    INTEGRATION
    //////////////////////////////////////////////////////////////*/

    function test_CompleteCrossChainFlow() public {
        uint256 totalAmount = 1000e18;

        vm.prank(user);
        token.transfer(address(bridge), totalAmount);

        // First mint message
        bytes32 mintMessageId = keccak256("complete-mint");
        bytes memory mintData = abi.encode("INITIAL_MINT", recipient, totalAmount);
        bytes memory mintPayload = abi.encodeWithSelector(
            destinationChain.receiveMintMessage.selector,
            mintMessageId,
            address(token),
            recipient,
            totalAmount,
            mintData
        );
        SequencerSignature memory mintSig =
            createSequencerSignature(mintMessageId, address(destinationChain), mintPayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(mintMessageId, address(destinationChain), mintPayload, mintSig, 0);
        submitValidatorSignatures(bridge, mintMessageId);
        bridge.handleMessage(mintMessageId);

        assertEq(token.balanceOf(recipient), totalAmount);

        // Return mint message
        address returnRecipient = makeAddr("returnRecipient");
        uint256 returnAmount = 300e18;

        bytes32 returnMessageId = keccak256("return-mint");
        bytes memory returnData = abi.encode("RETURN_BRIDGE", returnRecipient, returnAmount);
        bytes memory returnPayload = abi.encodeWithSelector(
            destinationChain.receiveMintMessage.selector,
            returnMessageId,
            address(token),
            returnRecipient,
            returnAmount,
            returnData
        );
        SequencerSignature memory returnSig =
            createSequencerSignature(returnMessageId, address(destinationChain), returnPayload, 0);

        vm.prank(sequencer);
        bridge.initializeMessage(returnMessageId, address(destinationChain), returnPayload, returnSig, 0);
        submitValidatorSignatures(bridge, returnMessageId);
        bridge.handleMessage(returnMessageId);

        assertEq(token.balanceOf(recipient), totalAmount);
        assertEq(token.balanceOf(returnRecipient), returnAmount);
        assertEq(token.balanceOf(address(bridge)), totalAmount);
    }

    receive() external payable {}
}
