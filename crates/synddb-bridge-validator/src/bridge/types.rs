use alloy::sol;

sol! {
    #[derive(Debug)]
    struct ApplicationConfigSol {
        address primaryValidator;
        uint64 expirationSeconds;
        bool requireWitnessSignatures;
        bool active;
    }

    #[derive(Debug)]
    struct MessageTypeConfigSol {
        address target;
        bytes32 schemaHash;
        string schemaUri;
        bool active;
    }

    #[sol(rpc)]
    interface IMessageBridge {
        function computeMessageId(
            string calldata messageType,
            bytes calldata calldata_,
            bytes32 metadataHash,
            uint64 nonce,
            uint64 timestamp,
            bytes32 domain
        ) external pure returns (bytes32);

        function initializeMessage(
            bytes32 messageId,
            string calldata messageType,
            bytes calldata calldata_,
            bytes32 metadataHash,
            string calldata storageRef,
            uint64 nonce,
            uint64 timestamp,
            bytes32 domain
        ) external payable;

        function signMessage(bytes32 messageId, bytes calldata signature) external;

        function rejectProposal(
            bytes32 messageId,
            string calldata messageType,
            bytes calldata calldata_,
            bytes32 metadataHash,
            string calldata storageRef,
            uint64 nonce,
            uint64 timestamp,
            bytes32 domain,
            string calldata reason
        ) external;

        function rejectMessage(bytes32 messageId, string calldata reason) external;

        function getLastNonce(bytes32 domain) external view returns (uint64);

        function getApplicationConfig(bytes32 domain) external view returns (ApplicationConfigSol memory);

        function getMessageTypeConfig(string calldata messageType) external view returns (MessageTypeConfigSol memory);

        function DOMAIN_SEPARATOR() external view returns (bytes32);

        event MessageInitialized(
            bytes32 indexed messageId,
            bytes32 indexed domain,
            string messageType,
            address indexed primaryValidator,
            string storageRef
        );

        event MessageSigned(
            bytes32 indexed messageId,
            address indexed validator,
            uint256 signatureCount
        );

        event MessageReady(
            bytes32 indexed messageId,
            uint256 signatureCount
        );
    }
}
