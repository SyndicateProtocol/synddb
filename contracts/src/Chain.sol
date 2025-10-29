// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

import {Ownable} from "@openzeppelin/contracts/access/Ownable.sol";
import {Pausable} from "@openzeppelin/contracts/utils/Pausable.sol";
import {ECDSA} from "@openzeppelin/contracts/utils/cryptography/ECDSA.sol";

/**
 * @title Chain
 * @notice Manages state commitments and WASM version control for SyndDB
 * @dev Handles state publication, version pinning, and state availability for Bridge.sol
 */
contract Chain is Ownable, Pausable {
    using ECDSA for bytes32;

    // ============ Constants ============
    uint256 public constant MAX_DIFF_SIZE = 1_000_000; // 1MB max diff size
    uint256 public constant MAX_SNAPSHOT_CHUNKS = 100;

    // ============ State Variables ============

    // WASM Version Management
    struct WASMVersion {
        bytes32 versionHash;
        string ipfsCID; // IPFS CID of WASM binary
        string arweaveTxId; // Arweave transaction ID (backup)
        uint256 activationTimestamp; // Unix timestamp when version becomes active
        bool isActive;
        uint256 addedAt;
    }

    mapping(bytes32 => WASMVersion) public wasmVersions;
    bytes32 public currentWASMVersion;
    bytes32 public pendingWASMVersion;
    uint256 public versionActivationDelay = 1 days; // 1 day default delay (configurable)

    // Governance constraints for activation delay (in seconds)
    uint256 public constant MIN_VERSION_ACTIVATION_DELAY = 12 hours; // 12 hours minimum
    uint256 public constant MAX_VERSION_ACTIVATION_DELAY = 10 days; // 10 days maximum

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
    uint256 public snapshotInterval = 10000; // Snapshot every 10k versions

    // Chain hash tracking (similar to OP Stack's approach)
    bytes32 public lastHash; // Global state hash
    bytes32 public lastDiffHash; // Last diff hash
    bytes32 public lastSnapshotHash; // Last snapshot hash

    // Access control
    address public sequencer;
    mapping(address => bool) public validators;
    uint256 public validatorCount;

    // Bridge integration
    address public bridgeContract;
    bool public stateAvailable;

    // ============ Events ============
    event WASMVersionAdded(bytes32 indexed versionHash, string ipfsCID, string arweaveTxId, uint256 activationTimestamp);

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
    event VersionActivationDelayUpdated(uint256 oldDelay, uint256 newDelay);

    // ============ Modifiers ============
    modifier onlySequencer() {
        _checkSequencer();
        _;
    }

    modifier onlyValidator() {
        _checkValidator();
        _;
    }

    modifier onlyBridge() {
        _checkBridge();
        _;
    }

    function _checkSequencer() private view {
        require(msg.sender == sequencer, "Not sequencer");
    }

    function _checkValidator() private view {
        require(validators[msg.sender], "Not validator");
    }

    function _checkBridge() private view {
        require(msg.sender == bridgeContract, "Not bridge");
    }

    // ============ Constructor ============
    constructor(
        address _sequencer,
        address _bridgeContract,
        bytes32 _initialWASMVersion,
        string memory _ipfsCID,
        string memory _arweaveTxId
    ) Ownable(msg.sender) {
        require(_sequencer != address(0), "Invalid sequencer");
        require(_bridgeContract != address(0), "Invalid bridge");
        require(_initialWASMVersion != bytes32(0), "Invalid WASM version");

        sequencer = _sequencer;
        bridgeContract = _bridgeContract;

        // Set initial WASM version
        wasmVersions[_initialWASMVersion] = WASMVersion({
            versionHash: _initialWASMVersion,
            ipfsCID: _ipfsCID,
            arweaveTxId: _arweaveTxId,
            activationTimestamp: block.timestamp,
            isActive: true,
            addedAt: block.timestamp
        });

        currentWASMVersion = _initialWASMVersion;
    }

    // ============ WASM Version Management ============

    /**
     * @notice Add a new WASM version (governance controlled)
     * @param versionHash The hash of the WASM binary
     * @param ipfsCID IPFS CID where WASM is stored
     * @param arweaveTxId Arweave transaction ID (backup storage)
     * @param activationDelay Time in seconds until activation
     */
    function addWASMVersion(
        bytes32 versionHash,
        string memory ipfsCID,
        string memory arweaveTxId,
        uint256 activationDelay
    ) external onlyOwner {
        require(versionHash != bytes32(0), "Invalid version hash");
        require(bytes(ipfsCID).length > 0, "Invalid IPFS CID");
        require(activationDelay >= versionActivationDelay, "Activation too soon");
        require(!wasmVersions[versionHash].isActive, "Version already exists");

        uint256 activationTimestamp = block.timestamp + activationDelay;

        wasmVersions[versionHash] = WASMVersion({
            versionHash: versionHash,
            ipfsCID: ipfsCID,
            arweaveTxId: arweaveTxId,
            activationTimestamp: activationTimestamp,
            isActive: false,
            addedAt: block.timestamp
        });

        pendingWASMVersion = versionHash;

        emit WASMVersionAdded(versionHash, ipfsCID, arweaveTxId, activationTimestamp);
    }

    /**
     * @notice Activate a pending WASM version
     */
    function activateWASMVersion() external {
        require(pendingWASMVersion != bytes32(0), "No pending version");

        WASMVersion storage pending = wasmVersions[pendingWASMVersion];
        require(block.timestamp >= pending.activationTimestamp, "Activation not ready");
        require(!pending.isActive, "Already active");

        bytes32 oldVersion = currentWASMVersion;

        // Deactivate old version
        if (oldVersion != bytes32(0)) {
            wasmVersions[oldVersion].isActive = false;
        }

        // Activate new version
        pending.isActive = true;
        currentWASMVersion = pendingWASMVersion;
        pendingWASMVersion = bytes32(0);

        emit WASMVersionActivated(oldVersion, currentWASMVersion, block.number);
    }

    /**
     * @notice Get current WASM version details
     */
    function getCurrentWASMVersion()
        external
        view
        returns (bytes32 versionHash, string memory ipfsCID, string memory arweaveTxId, bool isActive)
    {
        WASMVersion memory version = wasmVersions[currentWASMVersion];
        return (version.versionHash, version.ipfsCID, version.arweaveTxId, version.isActive);
    }

    // ============ State Management ============

    /**
     * @notice Publish a state diff
     * @param fromVersion Starting version
     * @param toVersion Ending version
     * @param diffHash Hash of the diff
     * @param storagePointer IPFS CID or Arweave TX
     * @param size Size of the diff
     */
    function publishDiff(
        uint256 fromVersion,
        uint256 toVersion,
        bytes32 diffHash,
        string memory storagePointer,
        uint256 size
    ) external onlySequencer whenNotPaused {
        require(toVersion > fromVersion, "Invalid version range");
        require(fromVersion == currentStateVersion, "Version mismatch");
        require(size <= MAX_DIFF_SIZE, "Diff too large");
        require(diffHash != bytes32(0), "Invalid diff hash");

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
     * @notice Publish a state snapshot
     * @param version Version of the snapshot
     * @param snapshotHash Hash of the snapshot
     * @param storagePointer IPFS CID or Arweave TX
     * @param size Size of the snapshot
     */
    function publishSnapshot(uint256 version, bytes32 snapshotHash, string memory storagePointer, uint256 size)
        external
        onlySequencer
        whenNotPaused
    {
        require(version == currentStateVersion, "Version mismatch");
        require(snapshotHash != bytes32(0), "Invalid snapshot hash");

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
     * @notice Commit a state root (called by validators)
     * @param version State version
     * @param stateRoot The state root hash
     */
    function commitState(uint256 version, bytes32 stateRoot) external onlyValidator whenNotPaused {
        require(version <= currentStateVersion, "Future version");
        require(stateRoot != bytes32(0), "Invalid state root");

        stateCommitments[version] = StateCommitment({
            stateRoot: stateRoot,
            wasmVersionHash: currentWASMVersion,
            sequencerVersion: version,
            blockNumber: block.number,
            timestamp: block.timestamp,
            submitter: msg.sender
        });

        currentStateRoot = stateRoot;
        stateAvailable = true;

        emit StateCommitted(version, stateRoot, currentWASMVersion, msg.sender);
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
     */
    function addValidator(address validator) external onlyOwner {
        require(validator != address(0), "Invalid validator");
        require(!validators[validator], "Already validator");

        validators[validator] = true;
        validatorCount++;

        emit ValidatorAdded(validator);
    }

    /**
     * @notice Remove a validator
     */
    function removeValidator(address validator) external onlyOwner {
        require(validators[validator], "Not validator");

        validators[validator] = false;
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

    /**
     * @notice Update the WASM version activation delay
     * @param newDelay New delay in seconds (must be between MIN and MAX constraints)
     * @dev Only owner can update. Affects future version activations, not pending ones.
     */
    function updateVersionActivationDelay(uint256 newDelay) external onlyOwner {
        require(
            newDelay >= MIN_VERSION_ACTIVATION_DELAY && newDelay <= MAX_VERSION_ACTIVATION_DELAY,
            "Delay out of bounds"
        );

        uint256 oldDelay = versionActivationDelay;
        versionActivationDelay = newDelay;

        emit VersionActivationDelayUpdated(oldDelay, newDelay);
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
}
