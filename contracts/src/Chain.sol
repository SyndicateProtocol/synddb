// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";
import {MessageHashUtils} from "@openzeppelin/contracts/utils/cryptography/MessageHashUtils.sol";

/**
 * @title Chain
 * @notice Manages state commitments and WASM version control for SyndDB
 * @dev Handles state publication, version pinning, and state availability for Bridge.sol
 */
contract Chain is Ownable, Pausable {
    using ECDSA for bytes32;
    using MessageHashUtils for bytes32;

    // ============ Constants ============
    uint256 public constant MAX_DIFF_SIZE = 1_000_000; // 1MB max diff size
    uint256 public constant MAX_SNAPSHOT_CHUNKS = 100;

    // ============ State Variables ============

    // WASM Version Management
    struct WASMVersion {
        bytes32 versionHash;
        string storageURI; // Immutable storage URI (recommend ipfs:// or ar:// schemes)
        bool isActive;
        uint256 addedAt;
    }

    mapping(bytes32 => WASMVersion) public wasmVersions;
    bytes32 public currentWASMVersion;

    // State Management
    struct StateCommitment {
        bytes32 stateRoot;
        bytes32 wasmVersionHash;
        uint256 sequencerVersion;
        uint256 blockNumber;
        uint256 timestamp;
        address submitter;
    }

    struct StateDiff {
        uint256 fromVersion;
        uint256 toVersion;
        bytes32 diffHash;
        string storagePointer; // IPFS CID or Arweave TX
        uint256 size;
        bytes32 wasmVersionHash;
    }

    struct StateSnapshot {
        uint256 version;
        bytes32 snapshotHash;
        string storagePointer;
        uint256 size;
        bytes32 wasmVersionHash;
        uint256 blockNumber;
    }

    // State tracking
    mapping(uint256 => StateCommitment) public stateCommitments;
    mapping(uint256 => StateDiff) public stateDiffs;
    mapping(uint256 => StateSnapshot) public stateSnapshots;

    uint256 public currentStateVersion;
    bytes32 public currentStateRoot;
    uint256 public lastSnapshotVersion;

    // Chain hash tracking (similar to OP Stack's approach)
    bytes32 public lastHash; // Global state hash
    bytes32 public lastDiffHash; // Last diff hash
    bytes32 public lastSnapshotHash; // Last snapshot hash

    // Access control - Chain is source of truth for sequencer and validators
    // Bridge.sol should read from Chain.sol
    address public sequencer;
    mapping(address => bool) public validators;
    mapping(uint256 => address) public validatorByIndex;
    uint256 public validatorCount;

    // Bridge integration
    address public bridgeContract;
    bool public stateAvailable;

    // ============ Events ============
    event WASMVersionAdded(bytes32 indexed versionHash, string storageURI);
    event WASMVersionActivated(bytes32 indexed oldVersion, bytes32 indexed newVersion, uint256 blockNumber);

    event StateCommitted(
        uint256 indexed version, bytes32 indexed stateRoot, bytes32 indexed wasmVersionHash, address submitter
    );

    event DiffPublished(
        uint256 indexed fromVersion,
        uint256 indexed toVersion,
        bytes32 diffHash,
        bytes32 globalHash,
        string storagePointer
    );

    event SnapshotPublished(uint256 indexed version, bytes32 snapshotHash, bytes32 globalHash, string storagePointer);

    event ValidatorAdded(address indexed validator);
    event ValidatorRemoved(address indexed validator);
    event SequencerUpdated(address indexed oldSequencer, address indexed newSequencer);
    event BridgeUpdated(address indexed oldBridge, address indexed newBridge);
    event ChainHalted(address indexed challenger, uint256 version, string reason);

    // ============ Modifiers ============
    modifier onlySequencer() {
        require(isSequencer(msg.sender), "Not sequencer");
        _;
    }

    modifier onlyValidator() {
        require(isValidatorActive(msg.sender), "Not validator");
        _;
    }

    modifier onlyBridge() {
        require(isBridge(msg.sender), "Not bridge");
        _;
    }

    // Public view functions for checking status
    function isSequencer(address account) public view returns (bool) {
        return account == sequencer;
    }

    function isValidatorActive(address account) public view returns (bool) {
        return validators[account];
    }

    function isBridge(address account) public view returns (bool) {
        return account == bridgeContract;
    }

    // ============ Constructor ============
    /**
     * @notice Initializes the Chain contract
     * @param _sequencer The address of the sequencer responsible for ordering transactions
     * @param _bridgeContract The address of the Bridge contract for integration
     * @param _initialWASMVersion The hash of the initial WASM version
     * @param _storageURI Storage URI for the initial WASM binary (recommend ipfs:// or ar:// schemes for immutability)
     */
    constructor(address _sequencer, address _bridgeContract, bytes32 _initialWASMVersion, string memory _storageURI)
        Ownable(msg.sender)
    {
        require(_sequencer != address(0), "Invalid sequencer");
        require(_bridgeContract != address(0), "Invalid bridge");
        require(_initialWASMVersion != bytes32(0), "Invalid WASM version");
        require(bytes(_storageURI).length > 0, "Invalid storage URI");

        sequencer = _sequencer;
        bridgeContract = _bridgeContract;

        // Set initial WASM version
        wasmVersions[_initialWASMVersion] = WASMVersion({
            versionHash: _initialWASMVersion, storageURI: _storageURI, isActive: true, addedAt: block.timestamp
        });

        currentWASMVersion = _initialWASMVersion;
    }

    // ============ WASM Version Management ============

    /**
     * @notice Add and activate a new WASM version (governance controlled)
     * @dev Use immutable storage URIs (ipfs:// or ar:// schemes recommended)
     * @param versionHash The hash of the WASM binary
     * @param storageURI Storage URI where WASM binary is stored
     */
    function addWASMVersion(bytes32 versionHash, string memory storageURI) external onlyOwner {
        require(versionHash != bytes32(0), "Invalid version hash");
        require(bytes(storageURI).length > 0, "Invalid storage URI");
        require(wasmVersions[versionHash].addedAt == 0, "Version already exists");

        bytes32 oldVersion = currentWASMVersion;

        // Deactivate old version
        if (oldVersion != bytes32(0)) {
            wasmVersions[oldVersion].isActive = false;
        }

        // Add and activate new version immediately
        wasmVersions[versionHash] =
            WASMVersion({versionHash: versionHash, storageURI: storageURI, isActive: true, addedAt: block.timestamp});

        currentWASMVersion = versionHash;

        emit WASMVersionAdded(versionHash, storageURI);
        emit WASMVersionActivated(oldVersion, versionHash, block.number);
    }

    /**
     * @notice Get current WASM version details
     */
    function getCurrentWASMVersion()
        external
        view
        returns (bytes32 versionHash, string memory storageURI, bool isActive)
    {
        WASMVersion memory version = wasmVersions[currentWASMVersion];
        return (version.versionHash, version.storageURI, version.isActive);
    }

    // ============ State Management ============

    /**
     * @notice Publish a state diff with validator sign-off
     * @dev Validators publish data after verifying sequencer signature. Ensures sequential version progression.
     * @param fromVersion Starting version (must equal currentStateVersion)
     * @param toVersion Ending version (must be fromVersion + 1 for sequential progression)
     * @param diffHash Hash of the diff
     * @param storagePointer Storage URI (recommend ipfs:// or ar:// schemes)
     * @param size Size of the diff
     * @param sequencerSignature Sequencer's signature over the diff data
     */
    function publishDiff(
        uint256 fromVersion,
        uint256 toVersion,
        bytes32 diffHash,
        string memory storagePointer,
        uint256 size,
        bytes memory sequencerSignature
    ) external onlyValidator whenNotPaused {
        // Sequential validation - prevent skipping versions
        require(toVersion == fromVersion + 1, "Non-sequential version");
        require(fromVersion == currentStateVersion, "Version mismatch");
        require(size <= MAX_DIFF_SIZE, "Diff too large");
        require(diffHash != bytes32(0), "Invalid diff hash");

        // Verify sequencer signature
        bytes32 messageHash = keccak256(abi.encodePacked(fromVersion, toVersion, diffHash, storagePointer, size));
        bytes32 ethSignedHash = messageHash.toEthSignedMessageHash();
        address recovered = ethSignedHash.recover(sequencerSignature);
        require(recovered == sequencer, "Invalid sequencer signature");

        // Update global hash chain
        bytes32 newGlobalHash = keccak256(abi.encode(lastHash, diffHash, toVersion));

        stateDiffs[toVersion] = StateDiff({
            fromVersion: fromVersion,
            toVersion: toVersion,
            diffHash: diffHash,
            storagePointer: storagePointer,
            size: size,
            wasmVersionHash: currentWASMVersion
        });

        // Update state
        currentStateVersion = toVersion;
        lastHash = newGlobalHash;
        lastDiffHash = diffHash;

        emit DiffPublished(fromVersion, toVersion, diffHash, newGlobalHash, storagePointer);
    }

    /**
     * @notice Publish a state snapshot with validator sign-off
     * @dev Validators publish snapshots after verifying sequencer signature. Snapshots are sequential.
     * @param version Version of the snapshot (must equal currentStateVersion)
     * @param snapshotHash Hash of the snapshot
     * @param storagePointer Storage URI (recommend ipfs:// or ar:// schemes)
     * @param size Size of the snapshot
     * @param sequencerSignature Sequencer's signature over the snapshot data
     */
    function publishSnapshot(
        uint256 version,
        bytes32 snapshotHash,
        string memory storagePointer,
        uint256 size,
        bytes memory sequencerSignature
    ) external onlyValidator whenNotPaused {
        require(version == currentStateVersion, "Version mismatch");
        require(snapshotHash != bytes32(0), "Invalid snapshot hash");

        // Ensure sequential snapshots
        require(version > lastSnapshotVersion, "Non-sequential snapshot");

        // Verify sequencer signature
        bytes32 messageHash = keccak256(abi.encodePacked(version, snapshotHash, storagePointer, size));
        bytes32 ethSignedHash = messageHash.toEthSignedMessageHash();
        address recovered = ethSignedHash.recover(sequencerSignature);
        require(recovered == sequencer, "Invalid sequencer signature");

        // Update global hash chain
        bytes32 newGlobalHash = keccak256(abi.encode(lastHash, snapshotHash, version));

        stateSnapshots[version] = StateSnapshot({
            version: version,
            snapshotHash: snapshotHash,
            storagePointer: storagePointer,
            size: size,
            wasmVersionHash: currentWASMVersion,
            blockNumber: block.number
        });

        // Update state
        lastSnapshotVersion = version;
        lastHash = newGlobalHash;
        lastSnapshotHash = snapshotHash;

        emit SnapshotPublished(version, snapshotHash, newGlobalHash, storagePointer);
    }

    /**
     * @notice Commit a state hash (called by validators)
     * @dev In a SQL environment, this represents a hash of the database state for verification purposes.
     *      Not a Merkle root like in EVM - instead a deterministic hash of the full DB state.
     *      Used for validators to attest that they agree on the current state.
     * @param version State version
     * @param stateHash The deterministic hash of the database state
     */
    function commitState(uint256 version, bytes32 stateHash) external onlyValidator whenNotPaused {
        require(version <= currentStateVersion, "Future version");
        require(stateHash != bytes32(0), "Invalid state hash");

        stateCommitments[version] = StateCommitment({
            stateRoot: stateHash,
            wasmVersionHash: currentWASMVersion,
            sequencerVersion: version,
            blockNumber: block.number,
            timestamp: block.timestamp,
            submitter: msg.sender
        });

        currentStateRoot = stateHash;
        stateAvailable = true;

        emit StateCommitted(version, stateHash, currentWASMVersion, msg.sender);
    }

    /**
     * @notice Challenge and halt the chain if a validator detects invalid data
     * @dev Prioritizes safety over liveness - any validator can halt the chain
     * @param version The version being challenged
     * @param reason Description of the challenge
     */
    function challengeAndHalt(uint256 version, string memory reason) external onlyValidator {
        require(bytes(reason).length > 0, "Must provide reason");

        // Immediately pause the chain
        _pause();

        emit ChainHalted(msg.sender, version, reason);
    }

    // ============ Bridge Integration ============

    /**
     * @notice Check if state is available for withdrawal processing
     * @param requiredVersion Minimum version required
     */
    function isStateAvailable(uint256 requiredVersion) external view returns (bool) {
        return stateAvailable && currentStateVersion >= requiredVersion;
    }

    /**
     * @notice Get state commitment for a version
     */
    function getStateCommitment(uint256 version)
        external
        view
        returns (bytes32 stateRoot, bytes32 wasmVersionHash, uint256 blockNumber, uint256 timestamp, address submitter)
    {
        StateCommitment memory commitment = stateCommitments[version];
        return (
            commitment.stateRoot,
            commitment.wasmVersionHash,
            commitment.blockNumber,
            commitment.timestamp,
            commitment.submitter
        );
    }

    // ============ Access Control ============

    /**
     * @notice Add a validator
     * @dev Bridge.sol should read validator info from Chain.sol to avoid duplication
     */
    function addValidator(address validator) external onlyOwner {
        require(validator != address(0), "Invalid validator");
        require(!validators[validator], "Already validator");

        validators[validator] = true;
        validatorByIndex[validatorCount] = validator;
        validatorCount++;

        emit ValidatorAdded(validator);
    }

    /**
     * @notice Remove a validator by index
     */
    function removeValidator(uint256 validatorIndex) external onlyOwner {
        require(validatorIndex < validatorCount, "Invalid index");
        address validator = validatorByIndex[validatorIndex];
        require(validators[validator], "Not validator");

        validators[validator] = false;

        // Move last validator to removed slot
        if (validatorIndex != validatorCount - 1) {
            validatorByIndex[validatorIndex] = validatorByIndex[validatorCount - 1];
        }
        delete validatorByIndex[validatorCount - 1];
        validatorCount--;

        emit ValidatorRemoved(validator);
    }

    /**
     * @notice Update sequencer
     */
    function updateSequencer(address newSequencer) external onlyOwner {
        require(newSequencer != address(0), "Invalid sequencer");
        address oldSequencer = sequencer;
        sequencer = newSequencer;
        emit SequencerUpdated(oldSequencer, newSequencer);
    }

    /**
     * @notice Update bridge contract
     */
    function updateBridge(address newBridge) external onlyOwner {
        require(newBridge != address(0), "Invalid bridge");
        address oldBridge = bridgeContract;
        bridgeContract = newBridge;
        emit BridgeUpdated(oldBridge, newBridge);
    }

    // ============ View Functions ============

    /**
     * @notice Get current chain state
     */
    function getChainState()
        external
        view
        returns (bytes32 globalHash, bytes32 diffHash, bytes32 snapshotHash, uint256 version)
    {
        return (lastHash, lastDiffHash, lastSnapshotHash, currentStateVersion);
    }

    /**
     * @notice Get latest diff
     */
    function getLatestDiff() external view returns (StateDiff memory) {
        return stateDiffs[currentStateVersion];
    }

    /**
     * @notice Get latest snapshot
     */
    function getLatestSnapshot() external view returns (StateSnapshot memory) {
        return stateSnapshots[lastSnapshotVersion];
    }

    /**
     * @notice Check if an address is a validator
     */
    function isValidator(address account) external view returns (bool) {
        return validators[account];
    }

    /**
     * @notice Get total validator count
     */
    function getValidatorCount() external view returns (uint256) {
        return validatorCount;
    }

    // ============ Emergency Functions ============

    /**
     * @notice Pause the contract
     */
    function pause() external onlyOwner {
        _pause();
    }

    /**
     * @notice Unpause the contract
     */
    function unpause() external onlyOwner {
        _unpause();
    }

    /**
     * @notice Get all validators
     */
    function getValidators() external view returns (address[] memory) {
        address[] memory validatorAddresses = new address[](validatorCount);
        for (uint256 i = 0; i < validatorCount; i++) {
            validatorAddresses[i] = validatorByIndex[i];
        }
        return validatorAddresses;
    }

    // ============ Receive Function ============
    /**
     * @notice Reject direct ETH transfers to prevent loss of funds
     */
    receive() external payable {
        revert("Direct ETH transfers not accepted");
    }
}
