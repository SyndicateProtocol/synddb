// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Test} from "forge-std/Test.sol";
import {GasTreasury} from "src/GasTreasury.sol";
import {ITeeKeyManager} from "src/interfaces/ITeeKeyManager.sol";

contract MockKeyManagerForTreasury {
    mapping(address => bool) public validSequencerKeys;
    mapping(address => bool) public validValidatorKeys;

    function setSequencerKeyValid(address key, bool valid) external {
        validSequencerKeys[key] = valid;
    }

    function setValidatorKeyValid(address key, bool valid) external {
        validValidatorKeys[key] = valid;
    }

    function isSequencerKeyValid(address key) external view returns (bool) {
        if (!validSequencerKeys[key]) {
            revert("Key not valid");
        }
        return true;
    }

    function isValidatorKeyValid(address key) external view returns (bool) {
        if (!validValidatorKeys[key]) {
            revert("Key not valid");
        }
        return true;
    }
}

/**
 * @title GasTreasuryTest
 * @notice Test suite for GasTreasury contract
 * @dev Tests signature-based funding, caps, and treasury management
 */
contract GasTreasuryTest is Test {
    GasTreasury public treasury;
    MockKeyManagerForTreasury public mockKeyManager;

    uint256 internal teePrivateKey;
    address internal teeAddress;

    uint256 constant FUNDING_AMOUNT = 0.05 ether;
    uint256 constant MAX_FUNDING_PER_KEY = 0.2 ether;

    event KeyFunded(address indexed teeKey, uint256 amount);
    event FundsReceived(address indexed from, uint256 amount);
    event FundingParamsUpdated(uint256 fundingAmount, uint256 maxFundingPerKey);

    function setUp() public {
        mockKeyManager = new MockKeyManagerForTreasury();
        treasury = new GasTreasury(ITeeKeyManager(address(mockKeyManager)), FUNDING_AMOUNT, MAX_FUNDING_PER_KEY);

        // Fund the treasury
        vm.deal(address(treasury), 10 ether);

        // Create a test TEE key
        teePrivateKey = 0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef;
        teeAddress = vm.addr(teePrivateKey);

        // Register the key as a sequencer
        mockKeyManager.setSequencerKeyValid(teeAddress, true);
    }

    function createFundingSignature(address key, uint256 privKey, uint256 deadline)
        internal
        view
        returns (bytes memory)
    {
        uint256 nonce = treasury.nonces(key);
        bytes32 structHash = keccak256(abi.encode(treasury.FUNDKEY_TYPEHASH(), key, nonce, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", treasury.DOMAIN_SEPARATOR(), structHash));

        (uint8 v, bytes32 r, bytes32 s) = vm.sign(privKey, digest);
        return abi.encodePacked(r, s, v);
    }

    function test_Constructor_SetsCorrectValues() public view {
        assertEq(treasury.fundingAmount(), FUNDING_AMOUNT);
        assertEq(treasury.maxFundingPerKey(), MAX_FUNDING_PER_KEY);
        assertEq(address(treasury.keyManager()), address(mockKeyManager));
    }

    function test_FundKeyWithSignature_Success() public {
        uint256 deadline = block.timestamp + 3600;
        bytes memory signature = createFundingSignature(teeAddress, teePrivateKey, deadline);

        uint256 balanceBefore = teeAddress.balance;

        vm.expectEmit(true, true, false, true);
        emit KeyFunded(teeAddress, FUNDING_AMOUNT);

        treasury.fundKeyWithSignature(teeAddress, deadline, signature);

        assertEq(teeAddress.balance, balanceBefore + FUNDING_AMOUNT);
        assertEq(treasury.totalFunded(teeAddress), FUNDING_AMOUNT);
        assertEq(treasury.nonces(teeAddress), 1);
    }

    function test_FundKeyWithSignature_MultipleFundings() public {
        // First funding
        uint256 deadline = block.timestamp + 3600;
        bytes memory sig1 = createFundingSignature(teeAddress, teePrivateKey, deadline);
        treasury.fundKeyWithSignature(teeAddress, deadline, sig1);

        // Second funding (nonce incremented)
        bytes memory sig2 = createFundingSignature(teeAddress, teePrivateKey, deadline);
        treasury.fundKeyWithSignature(teeAddress, deadline, sig2);

        assertEq(treasury.totalFunded(teeAddress), FUNDING_AMOUNT * 2);
        assertEq(treasury.nonces(teeAddress), 2);
    }

    function test_FundKeyWithSignature_RevertsWhenExpired() public {
        uint256 deadline = block.timestamp - 1; // Expired
        bytes memory signature = createFundingSignature(teeAddress, teePrivateKey, deadline);

        vm.expectRevert(GasTreasury.SignatureExpired.selector);
        treasury.fundKeyWithSignature(teeAddress, deadline, signature);
    }

    function test_FundKeyWithSignature_RevertsWhenKeyNotRegistered() public {
        address unregisteredKey = address(0x9999);
        uint256 unregisteredPrivKey = 0xdeadbeef;
        uint256 deadline = block.timestamp + 3600;

        bytes memory signature = createFundingSignature(unregisteredKey, unregisteredPrivKey, deadline);

        vm.expectRevert(abi.encodeWithSelector(GasTreasury.KeyNotRegistered.selector, unregisteredKey));
        treasury.fundKeyWithSignature(unregisteredKey, deadline, signature);
    }

    function test_FundKeyWithSignature_RevertsWhenWrongSigner() public {
        uint256 differentPrivKey = 0xbeefbeef;
        uint256 deadline = block.timestamp + 3600;

        // Create signature with different private key
        bytes memory signature = createFundingSignature(teeAddress, differentPrivKey, deadline);

        vm.expectRevert(GasTreasury.InvalidSignature.selector);
        treasury.fundKeyWithSignature(teeAddress, deadline, signature);
    }

    function test_FundKeyWithSignature_RevertsWhenCapExceeded() public {
        uint256 deadline = block.timestamp + 3600;

        // Fund multiple times until cap is reached
        uint256 numFundings = MAX_FUNDING_PER_KEY / FUNDING_AMOUNT;
        for (uint256 i = 0; i < numFundings; i++) {
            bytes memory sig = createFundingSignature(teeAddress, teePrivateKey, deadline);
            treasury.fundKeyWithSignature(teeAddress, deadline, sig);
        }

        // Next funding should fail
        bytes memory sig = createFundingSignature(teeAddress, teePrivateKey, deadline);
        vm.expectRevert(
            abi.encodeWithSelector(
                GasTreasury.FundingCapExceeded.selector,
                teeAddress,
                treasury.totalFunded(teeAddress),
                MAX_FUNDING_PER_KEY
            )
        );
        treasury.fundKeyWithSignature(teeAddress, deadline, sig);
    }

    function test_FundKeyWithSignature_RevertsWhenInsufficientBalance() public {
        // Drain the treasury
        treasury.withdraw(10 ether, address(this));

        uint256 deadline = block.timestamp + 3600;
        bytes memory signature = createFundingSignature(teeAddress, teePrivateKey, deadline);

        vm.expectRevert(abi.encodeWithSelector(GasTreasury.InsufficientTreasuryBalance.selector, FUNDING_AMOUNT, 0));
        treasury.fundKeyWithSignature(teeAddress, deadline, signature);
    }

    function test_FundKeyWithSignature_PreventsReplay() public {
        uint256 deadline = block.timestamp + 3600;
        bytes memory signature = createFundingSignature(teeAddress, teePrivateKey, deadline);

        // First call succeeds
        treasury.fundKeyWithSignature(teeAddress, deadline, signature);

        // Same signature fails (nonce incremented)
        vm.expectRevert(GasTreasury.InvalidSignature.selector);
        treasury.fundKeyWithSignature(teeAddress, deadline, signature);
    }

    function test_ReceiveFunds() public {
        uint256 amount = 1 ether;
        address sender = address(0x1234);
        vm.deal(sender, amount);

        uint256 balanceBefore = treasury.balance();

        vm.prank(sender);
        vm.expectEmit(true, true, false, true);
        emit FundsReceived(sender, amount);

        (bool success,) = address(treasury).call{value: amount}("");
        assertTrue(success);

        assertEq(treasury.balance(), balanceBefore + amount);
    }

    function test_Withdraw_Success() public {
        uint256 amount = 1 ether;
        address recipient = address(0x5678);
        uint256 balanceBefore = recipient.balance;

        treasury.withdraw(amount, recipient);

        assertEq(recipient.balance, balanceBefore + amount);
    }

    function test_Withdraw_RevertsWhenNotOwner() public {
        vm.prank(address(0x123));
        vm.expectRevert();
        treasury.withdraw(1 ether, address(0x123));
    }

    function test_Withdraw_RevertsWhenInsufficientBalance() public {
        uint256 tooMuch = 100 ether;
        vm.expectRevert(abi.encodeWithSelector(GasTreasury.InsufficientTreasuryBalance.selector, tooMuch, 10 ether));
        treasury.withdraw(tooMuch, address(this));
    }

    function test_SetFundingParams() public {
        uint256 newFundingAmount = 0.1 ether;
        uint256 newMaxFunding = 1 ether;

        vm.expectEmit(true, true, false, true);
        emit FundingParamsUpdated(newFundingAmount, newMaxFunding);

        treasury.setFundingParams(newFundingAmount, newMaxFunding);

        assertEq(treasury.fundingAmount(), newFundingAmount);
        assertEq(treasury.maxFundingPerKey(), newMaxFunding);
    }

    function test_SetFundingParams_RevertsWhenNotOwner() public {
        vm.prank(address(0x123));
        vm.expectRevert();
        treasury.setFundingParams(0.1 ether, 1 ether);
    }

    function test_RelayerCanSubmitFunding() public {
        uint256 deadline = block.timestamp + 3600;
        bytes memory signature = createFundingSignature(teeAddress, teePrivateKey, deadline);

        // Relayer submits (not the TEE key itself)
        address relayer = address(0xBEEF);
        vm.prank(relayer);

        treasury.fundKeyWithSignature(teeAddress, deadline, signature);

        assertEq(treasury.totalFunded(teeAddress), FUNDING_AMOUNT);
    }

    receive() external payable {}
}
