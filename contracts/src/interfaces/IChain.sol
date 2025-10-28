// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

/**
 * @title IChain
 * @notice Interface for the SyndDB Chain contract (state management and WASM version control)
 */
interface IChain {
    // ============ Structs ============

    struct WASMVersion {
        bytes32 versionHash;
        string ipfsCID;
        string arweaveTxId;
        uint256 activationBlock;
        bool isActive;
        uint256 addedAt;
    }

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
        string storagePointer;
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

    // ============ Events ============

    event WASMVersionAdded(
        bytes32 indexed versionHash,
        string ipfsCID,
        string arweaveTxId,
        uint256 activationBlock
    );

    event WASMVersionActivated(
        bytes32 indexed oldVersion,
        bytes32 indexed newVersion,
        uint256 blockNumber
    );

    event StateCommitted(
        uint256 indexed version,
        bytes32 indexed stateRoot,
        bytes32 indexed wasmVersionHash,
        address submitter
    );

    event DiffPublished(
        uint256 indexed fromVersion,
        uint256 indexed toVersion,
        bytes32 diffHash,
        bytes32 globalHash,
        string storagePointer
    );

    event SnapshotPublished(
        uint256 indexed version,
        bytes32 snapshotHash,
        bytes32 globalHash,
        string storagePointer
    );

    // ============ WASM Version Functions ============

    function addWASMVersion(
        bytes32 versionHash,
        string memory ipfsCID,
        string memory arweaveTxId,
        uint256 activationDelay
    ) external;

    function activateWASMVersion() external;

    function getCurrentWASMVersion() external view returns (
        bytes32 versionHash,
        string memory ipfsCID,
        string memory arweaveTxId,
        bool isActive
    );

    // ============ State Management Functions ============

    function publishDiff(
        uint256 fromVersion,
        uint256 toVersion,
        bytes32 diffHash,
        string memory storagePointer,
        uint256 size
    ) external;

    function publishSnapshot(
        uint256 version,
        bytes32 snapshotHash,
        string memory storagePointer,
        uint256 size
    ) external;

    function commitState(
        uint256 version,
        bytes32 stateRoot
    ) external;

    // ============ View Functions ============

    function isStateAvailable(uint256 requiredVersion) external view returns (bool);

    function getStateCommitment(uint256 version) external view returns (
        bytes32 stateRoot,
        bytes32 wasmVersionHash,
        uint256 blockNumber,
        uint256 timestamp,
        address submitter
    );

    function getChainState() external view returns (
        bytes32 globalHash,
        bytes32 diffHash,
        bytes32 snapshotHash,
        uint256 version
    );

    function getLatestDiff() external view returns (StateDiff memory);
    function getLatestSnapshot() external view returns (StateSnapshot memory);

    function currentStateVersion() external view returns (uint256);
    function currentStateRoot() external view returns (bytes32);
    function currentWASMVersion() external view returns (bytes32);
    function pendingWASMVersion() external view returns (bytes32);
}