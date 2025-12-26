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
        bytes4 selector;
        address target;
        bytes32 schemaHash;
        string schemaUri;
        bool enabled;
        uint64 updatedAt;
    }

    #[derive(Debug)]
    struct MessageStateV2Sol {
        uint8 stage;
        string messageType;
        bytes calldata_;
        bytes32 metadataHash;
        string storageRef;
        uint256 value;
        uint64 nonce;
        uint64 timestamp;
        bytes32 domain;
        address primaryValidator;
        uint256 signaturesCollected;
        uint256 rejectionsCollected;
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

        function initializeAndSign(
            bytes32 messageId,
            string calldata messageType,
            bytes calldata calldata_,
            bytes32 metadataHash,
            string calldata storageRef,
            uint64 nonce,
            uint64 timestamp,
            bytes32 domain,
            bytes calldata signature
        ) external payable;

        function signMessage(bytes32 messageId, bytes calldata signature) external;

        function executeMessage(bytes32 messageId) external;

        function rejectProposal(
            bytes32 messageId,
            string calldata messageType,
            bytes32 domain,
            uint64 nonce,
            bytes32 reasonHash,
            string calldata reasonRef
        ) external;

        function rejectMessage(
            bytes32 messageId,
            bytes32 reasonHash,
            string calldata reasonRef
        ) external;

        function getLastNonce(bytes32 domain) external view returns (uint64);

        function getApplicationConfig(bytes32 domain) external view returns (ApplicationConfigSol memory);

        function getMessageTypeConfig(string calldata messageType) external view returns (MessageTypeConfigSol memory);

        function getMessageState(bytes32 messageId) external view returns (MessageStateV2Sol memory);

        function getMessageStage(bytes32 messageId) external view returns (uint8);

        function getSignatureCount(bytes32 messageId) external view returns (uint256);

        function hasValidatorSigned(bytes32 messageId, address validator) external view returns (bool);

        function signatureThreshold() external view returns (uint256);

        function DOMAIN_SEPARATOR() external view returns (bytes32);

        event MessageInitialized(
            bytes32 indexed messageId,
            bytes32 indexed domain,
            address indexed primaryValidator,
            string messageType,
            string storageRef
        );

        event SignatureSubmitted(
            bytes32 indexed messageId,
            address indexed validator,
            uint256 signaturesCollected
        );

        event ThresholdReached(
            bytes32 indexed messageId,
            uint256 signaturesCollected
        );

        event MessageExecuted(
            bytes32 indexed messageId,
            address indexed target,
            bool success
        );
    }
}
