// SPDX-License-Identifier: MIT
pragma solidity ^0.8.30;

/**
 * @title MerkleUtils
 * @notice Utility library for Merkle tree operations
 */
library MerkleUtils {
    /**
     * @notice Calculate Merkle root from leaves
     * @param leaves Array of leaf hashes
     * @return The Merkle root
     */
    function calculateMerkleRoot(bytes32[] memory leaves) internal pure returns (bytes32) {
        if (leaves.length == 0) return bytes32(0);
        if (leaves.length == 1) return leaves[0];

        // Create a working array
        uint256 levelLength = leaves.length;
        bytes32[] memory currentLevel = new bytes32[](levelLength);

        // Copy leaves to working array
        for (uint256 i = 0; i < levelLength; i++) {
            currentLevel[i] = leaves[i];
        }

        // Build tree level by level
        while (levelLength > 1) {
            uint256 nextLevelLength = (levelLength + 1) / 2;
            bytes32[] memory nextLevel = new bytes32[](nextLevelLength);

            for (uint256 i = 0; i < nextLevelLength; i++) {
                uint256 leftIndex = i * 2;
                uint256 rightIndex = leftIndex + 1;

                if (rightIndex < levelLength) {
                    // Hash pair of nodes
                    nextLevel[i] = hashPair(currentLevel[leftIndex], currentLevel[rightIndex]);
                } else {
                    // Odd number of nodes, promote the last one
                    nextLevel[i] = currentLevel[leftIndex];
                }
            }

            currentLevel = nextLevel;
            levelLength = nextLevelLength;
        }

        return currentLevel[0];
    }

    /**
     * @notice Verify a Merkle proof
     * @param leaf The leaf to verify
     * @param proof Array of sibling hashes
     * @param root The expected Merkle root
     * @param index The leaf index in the tree
     * @return Whether the proof is valid
     */
    function verifyMerkleProof(bytes32 leaf, bytes32[] memory proof, bytes32 root, uint256 index)
        internal
        pure
        returns (bool)
    {
        bytes32 computedHash = leaf;

        for (uint256 i = 0; i < proof.length; i++) {
            bytes32 proofElement = proof[i];

            if (index % 2 == 0) {
                // Left node
                computedHash = hashPair(computedHash, proofElement);
            } else {
                // Right node
                computedHash = hashPair(proofElement, computedHash);
            }

            index = index / 2;
        }

        return computedHash == root;
    }

    /**
     * @notice Hash a pair of nodes
     * @param left The left node
     * @param right The right node
     * @return The combined hash
     */
    function hashPair(bytes32 left, bytes32 right) internal pure returns (bytes32) {
        return left < right ? keccak256(abi.encodePacked(left, right)) : keccak256(abi.encodePacked(right, left));
    }

    /**
     * @notice Calculate the Merkle root for balance updates
     * @param accounts Array of account addresses
     * @param tokens Array of token addresses
     * @param deltas Array of balance deltas
     * @return The Merkle root of balance updates
     */
    function calculateBalanceUpdateRoot(address[] memory accounts, address[] memory tokens, int256[] memory deltas)
        internal
        pure
        returns (bytes32)
    {
        require(accounts.length == tokens.length && tokens.length == deltas.length, "Array length mismatch");

        bytes32[] memory leaves = new bytes32[](accounts.length);

        for (uint256 i = 0; i < accounts.length; i++) {
            leaves[i] = keccak256(abi.encode(accounts[i], tokens[i], deltas[i]));
        }

        return calculateMerkleRoot(leaves);
    }

    /**
     * @notice Generate a Merkle proof for a specific leaf
     * @param leaves All leaves in the tree
     * @param leafIndex The index of the leaf to prove
     * @return proof Array of sibling hashes for the proof
     */
    function generateMerkleProof(bytes32[] memory leaves, uint256 leafIndex)
        internal
        pure
        returns (bytes32[] memory proof)
    {
        require(leafIndex < leaves.length, "Leaf index out of bounds");

        // Calculate proof size (log2 of leaves rounded up)
        uint256 proofSize = 0;
        uint256 n = leaves.length - 1;
        while (n > 0) {
            proofSize++;
            n /= 2;
        }

        proof = new bytes32[](proofSize);
        uint256 proofIndex = 0;

        // Build the proof
        uint256 currentLevelSize = leaves.length;
        uint256 currentIndex = leafIndex;
        bytes32[] memory currentLevel = leaves;

        while (currentLevelSize > 1) {
            uint256 siblingIndex = (currentIndex % 2 == 0) ? currentIndex + 1 : currentIndex - 1;

            if (siblingIndex < currentLevelSize) {
                proof[proofIndex++] = currentLevel[siblingIndex];
            }

            // Build next level
            uint256 nextLevelSize = (currentLevelSize + 1) / 2;
            bytes32[] memory nextLevel = new bytes32[](nextLevelSize);

            for (uint256 i = 0; i < nextLevelSize; i++) {
                uint256 leftIndex = i * 2;
                uint256 rightIndex = leftIndex + 1;

                if (rightIndex < currentLevelSize) {
                    nextLevel[i] = hashPair(currentLevel[leftIndex], currentLevel[rightIndex]);
                } else {
                    nextLevel[i] = currentLevel[leftIndex];
                }
            }

            currentLevel = nextLevel;
            currentLevelSize = nextLevelSize;
            currentIndex = currentIndex / 2;
        }

        // Resize proof array to actual size
        assembly {
            mstore(proof, proofIndex)
        }
    }
}
