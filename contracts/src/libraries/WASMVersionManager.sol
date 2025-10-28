// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

/**
 * @title WASMVersionManager
 * @notice Library for managing WASM version transitions and validation
 */
library WASMVersionManager {
    // ============ Structs ============

    struct Version {
        bytes32 hash;
        string ipfsCID;
        string arweaveTxId;
        uint256 activationBlock;
        uint256 deprecationBlock;
        bool isActive;
    }

    struct VersionTransition {
        bytes32 fromVersion;
        bytes32 toVersion;
        uint256 transitionBlock;
        bool completed;
    }

    // ============ Events (emitted via contract) ============

    event VersionScheduled(
        bytes32 indexed currentVersion,
        bytes32 indexed newVersion,
        uint256 activationBlock
    );

    event VersionActivated(
        bytes32 indexed oldVersion,
        bytes32 indexed newVersion,
        uint256 blockNumber
    );

    event VersionRolledBack(
        bytes32 indexed fromVersion,
        bytes32 indexed toVersion,
        string reason
    );

    // ============ Version Management Functions ============

    /**
     * @notice Validate a version hash against expected format
     * @param versionHash The version hash to validate
     * @return isValid Whether the version hash is valid
     */
    function validateVersionHash(bytes32 versionHash) internal pure returns (bool isValid) {
        // Version hash must not be zero
        if (versionHash == bytes32(0)) {
            return false;
        }

        // Additional validation logic can be added here
        // For example, checking specific byte patterns or prefixes
        return true;
    }

    /**
     * @notice Check if a version transition is ready
     * @param transition The version transition to check
     * @return isReady Whether the transition can be executed
     */
    function isTransitionReady(
        VersionTransition memory transition
    ) internal view returns (bool isReady) {
        return !transition.completed &&
               block.number >= transition.transitionBlock &&
               transition.fromVersion != bytes32(0) &&
               transition.toVersion != bytes32(0);
    }

    /**
     * @notice Calculate version compatibility
     * @param currentVersion Current WASM version
     * @param targetVersion Target WASM version
     * @return compatible Whether versions are compatible
     */
    function areVersionsCompatible(
        bytes32 currentVersion,
        bytes32 targetVersion
    ) internal pure returns (bool compatible) {
        // Implement version compatibility logic
        // For now, we'll use a simple approach where all versions are compatible
        // In production, this might check major/minor version numbers
        return currentVersion != bytes32(0) && targetVersion != bytes32(0);
    }

    /**
     * @notice Generate a version transition hash
     * @param fromVersion Starting version
     * @param toVersion Target version
     * @param nonce Unique nonce for the transition
     * @return transitionHash The unique transition identifier
     */
    function generateTransitionHash(
        bytes32 fromVersion,
        bytes32 toVersion,
        uint256 nonce
    ) internal pure returns (bytes32 transitionHash) {
        return keccak256(abi.encode(fromVersion, toVersion, nonce));
    }

    /**
     * @notice Verify WASM binary integrity
     * @param wasmData The WASM binary data
     * @param expectedHash Expected hash of the WASM binary
     * @return valid Whether the WASM data matches expected hash
     */
    function verifyWASMIntegrity(
        bytes memory wasmData,
        bytes32 expectedHash
    ) internal pure returns (bool valid) {
        bytes32 actualHash = keccak256(wasmData);
        return actualHash == expectedHash;
    }

    /**
     * @notice Calculate activation delay based on version change magnitude
     * @param isMinorUpdate Whether this is a minor update
     * @param isCriticalFix Whether this is a critical security fix
     * @return delay Blocks to wait before activation
     */
    function calculateActivationDelay(
        bool isMinorUpdate,
        bool isCriticalFix
    ) internal pure returns (uint256 delay) {
        if (isCriticalFix) {
            // Critical fixes can be activated faster
            return 300; // ~1 hour at 12s blocks
        } else if (isMinorUpdate) {
            // Minor updates need moderate delay
            return 3600; // ~12 hours at 12s blocks
        } else {
            // Major updates need longer delay
            return 7200; // ~24 hours at 12s blocks
        }
    }

    /**
     * @notice Check if a version is within valid activation window
     * @param version The version to check
     * @param currentBlock Current block number
     * @return inWindow Whether version is in activation window
     */
    function isInActivationWindow(
        Version memory version,
        uint256 currentBlock
    ) internal pure returns (bool inWindow) {
        return currentBlock >= version.activationBlock &&
               (version.deprecationBlock == 0 || currentBlock < version.deprecationBlock);
    }

    /**
     * @notice Encode version metadata for storage
     * @param version Version struct to encode
     * @return encoded Encoded version data
     */
    function encodeVersion(
        Version memory version
    ) internal pure returns (bytes memory encoded) {
        return abi.encode(
            version.hash,
            version.ipfsCID,
            version.arweaveTxId,
            version.activationBlock,
            version.deprecationBlock,
            version.isActive
        );
    }

    /**
     * @notice Decode version metadata from storage
     * @param data Encoded version data
     * @return version Decoded version struct
     */
    function decodeVersion(
        bytes memory data
    ) internal pure returns (Version memory version) {
        (
            version.hash,
            version.ipfsCID,
            version.arweaveTxId,
            version.activationBlock,
            version.deprecationBlock,
            version.isActive
        ) = abi.decode(data, (bytes32, string, string, uint256, uint256, bool));
    }

    /**
     * @notice Generate storage key for version data
     * @param versionHash Version hash
     * @param suffix Additional identifier
     * @return key Storage key
     */
    function generateStorageKey(
        bytes32 versionHash,
        string memory suffix
    ) internal pure returns (bytes32 key) {
        return keccak256(abi.encodePacked(versionHash, suffix));
    }

    /**
     * @notice Check if version update requires validator consensus
     * @param fromVersion Current version
     * @param toVersion Target version
     * @return requiresConsensus Whether validator consensus is needed
     */
    function requiresValidatorConsensus(
        bytes32 fromVersion,
        bytes32 toVersion
    ) internal pure returns (bool requiresConsensus) {
        // Major version changes require validator consensus
        // This is a simplified check - in production would parse version numbers
        return fromVersion != toVersion;
    }

    /**
     * @notice Calculate version priority for conflict resolution
     * @param version1 First version
     * @param version2 Second version
     * @return priority Which version has priority (1 or 2, 0 if equal)
     */
    function getVersionPriority(
        Version memory version1,
        Version memory version2
    ) internal pure returns (uint8 priority) {
        // Newer activation blocks have priority
        if (version1.activationBlock > version2.activationBlock) {
            return 1;
        } else if (version2.activationBlock > version1.activationBlock) {
            return 2;
        }
        return 0;
    }
}