// SPDX-License-Identifier: MIT
pragma solidity 0.8.30;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";
import {ITeeKeyManager} from "src/interfaces/ITeeKeyManager.sol";
import {IGasTreasury} from "src/interfaces/IGasTreasury.sol";
import {KeyType} from "src/types/DataTypes.sol";

/**
 * @title GasTreasury
 * @notice Manages gas funding for TEE signing keys
 * @dev Provides signature-based funding so keys without gas can request funding.
 *      A relayer submits the funding request with the key's signature.
 *      Verifies key is registered in TeeKeyManager before funding.
 */
contract GasTreasury is IGasTreasury, Ownable {
    using ECDSA for bytes32;
    using MessageHashUtils for bytes32;

    event KeyFunded(address indexed teeKey, uint256 amount);
    event FundsReceived(address indexed from, uint256 amount);
    event FundingParamsUpdated(uint256 fundingAmount, uint256 maxFundingPerKey);

    error InvalidSignature();
    error SignatureExpired();
    error KeyNotRegistered(address teeKey);
    error FundingCapExceeded(address teeKey, uint256 totalFunded, uint256 maxAllowed);
    error InsufficientTreasuryBalance(uint256 requested, uint256 available);
    error TransferFailed();

    /// @notice EIP-712 domain separator
    bytes32 public immutable DOMAIN_SEPARATOR;

    /// @notice EIP-712 typehash for FundKey
    bytes32 public constant FUNDKEY_TYPEHASH = keccak256("FundKey(address teeKey,uint256 nonce,uint256 deadline)");

    /// @notice TeeKeyManager contract for validating keys
    ITeeKeyManager public immutable keyManager;

    /// @notice Amount to fund per request (in wei)
    uint256 public fundingAmount;

    /// @notice Maximum total funding per key (in wei)
    uint256 public maxFundingPerKey;

    /// @notice Nonces for replay protection (teeKey => nonce)
    mapping(address => uint256) public nonces;

    /// @notice Total amount funded to each key
    mapping(address => uint256) public totalFunded;

    /**
     * @notice Constructs the gas treasury
     * @param _keyManager Address of the TeeKeyManager contract
     * @param _fundingAmount Amount to fund per request (in wei)
     * @param _maxFundingPerKey Maximum total funding per key (in wei)
     */
    constructor(ITeeKeyManager _keyManager, uint256 _fundingAmount, uint256 _maxFundingPerKey) Ownable(msg.sender) {
        keyManager = _keyManager;
        fundingAmount = _fundingAmount;
        maxFundingPerKey = _maxFundingPerKey;

        DOMAIN_SEPARATOR = keccak256(
            abi.encode(
                keccak256("EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)"),
                keccak256("GasTreasury"),
                keccak256("1"),
                block.chainid,
                address(this)
            )
        );
    }

    /**
     * @notice Funds a registered TEE key via signature
     * @dev Called by relayer with signature from the key requesting funding.
     *      Verifies the key is registered in TeeKeyManager as either sequencer or validator.
     * @param teeKey The key to fund
     * @param deadline Timestamp after which the signature expires
     * @param signature EIP-712 signature from the TEE key
     */
    function fundKeyWithSignature(address teeKey, uint256 deadline, bytes calldata signature) external override {
        if (block.timestamp > deadline) revert SignatureExpired();

        // Verify key is registered as either sequencer or validator
        bool isValidKey = false;
        try keyManager.isKeyValid(KeyType.Sequencer, teeKey) {
            isValidKey = true;
        } catch {}

        if (!isValidKey) {
            try keyManager.isKeyValid(KeyType.Validator, teeKey) {
                isValidKey = true;
            } catch {}
        }

        if (!isValidKey) {
            revert KeyNotRegistered(teeKey);
        }

        // Verify signature
        uint256 nonce = nonces[teeKey];
        bytes32 structHash = keccak256(abi.encode(FUNDKEY_TYPEHASH, teeKey, nonce, deadline));
        bytes32 digest = keccak256(abi.encodePacked("\x19\x01", DOMAIN_SEPARATOR, structHash));

        address signer = digest.recover(signature);
        if (signer != teeKey) revert InvalidSignature();

        // Increment nonce for replay protection
        nonces[teeKey] = nonce + 1;

        // Check funding cap
        uint256 newTotal = totalFunded[teeKey] + fundingAmount;
        if (newTotal > maxFundingPerKey) {
            revert FundingCapExceeded(teeKey, totalFunded[teeKey], maxFundingPerKey);
        }

        // Check treasury balance
        if (address(this).balance < fundingAmount) {
            revert InsufficientTreasuryBalance(fundingAmount, address(this).balance);
        }

        // Update state before transfer (CEI pattern)
        totalFunded[teeKey] = newTotal;

        // Transfer funds
        (bool success,) = teeKey.call{value: fundingAmount}("");
        if (!success) revert TransferFailed();

        emit KeyFunded(teeKey, fundingAmount);
    }

    /**
     * @notice Receives funds from old keys draining their balance
     * @dev Also used to fund the treasury
     */
    receive() external payable {
        emit FundsReceived(msg.sender, msg.value);
    }

    /**
     * @notice Withdraws funds from the treasury
     * @dev Only callable by owner
     * @param amount Amount to withdraw in wei
     * @param to Address to send funds to
     */
    function withdraw(uint256 amount, address to) external onlyOwner {
        if (address(this).balance < amount) {
            revert InsufficientTreasuryBalance(amount, address(this).balance);
        }
        (bool success,) = to.call{value: amount}("");
        if (!success) revert TransferFailed();
    }

    /**
     * @notice Updates funding parameters
     * @dev Only callable by owner
     * @param _fundingAmount New funding amount per request
     * @param _maxFundingPerKey New maximum total funding per key
     */
    function setFundingParams(uint256 _fundingAmount, uint256 _maxFundingPerKey) external onlyOwner {
        fundingAmount = _fundingAmount;
        maxFundingPerKey = _maxFundingPerKey;
        emit FundingParamsUpdated(_fundingAmount, _maxFundingPerKey);
    }

    /**
     * @notice Gets the treasury balance
     * @return Current balance in wei
     */
    function balance() external view returns (uint256) {
        return address(this).balance;
    }
}
