//! ABI encoding for Bridge calldata.
//!
//! Uses alloy's sol! macro for type-safe encoding.

use alloy::primitives::{Address, FixedBytes, U256};
use alloy::sol;
use alloy::sol_types::SolCall;

// Define the contract interface using sol! macro
sol! {
    #[sol(rpc)]
    interface IPredictionMarket {
        function createMarket(bytes32 marketId, string calldata question, uint256 resolutionTime) external;
        function deposit(address user, uint256 amount) external;
        function buyShares(bytes32 marketId, address user, uint8 outcome, uint256 shares) external;
        function sellShares(bytes32 marketId, address user, uint8 outcome, uint256 shares) external;
        function resolveMarket(bytes32 marketId, uint8 outcome) external;

        event MarketCreated(bytes32 indexed marketId, string question, uint256 resolutionTime);
        event MarketResolved(bytes32 indexed marketId, uint8 outcome);
        event Deposit(address indexed user, uint256 amount);
        event SharesPurchased(bytes32 indexed marketId, address indexed user, uint8 outcome, uint256 shares);
        event SharesSold(bytes32 indexed marketId, address indexed user, uint8 outcome, uint256 shares);
    }
}

/// Function signatures for message types.
pub mod signatures {
    pub const CREATE_MARKET: &str = "createMarket(bytes32,string,uint256)";
    pub const DEPOSIT: &str = "deposit(address,uint256)";
    pub const BUY_SHARES: &str = "buyShares(bytes32,address,uint8,uint256)";
    pub const SELL_SHARES: &str = "sellShares(bytes32,address,uint8,uint256)";
    pub const RESOLVE_MARKET: &str = "resolveMarket(bytes32,uint8)";
}

/// Encode calldata for createMarket.
pub fn encode_create_market(market_id: [u8; 32], question: &str, resolution_time: u64) -> Vec<u8> {
    let call = IPredictionMarket::createMarketCall {
        marketId: FixedBytes::from(market_id),
        question: question.to_string(),
        resolutionTime: U256::from(resolution_time),
    };
    call.abi_encode()
}

/// Encode calldata for deposit.
pub fn encode_deposit(user: Address, amount: u64) -> Vec<u8> {
    let call = IPredictionMarket::depositCall {
        user,
        amount: U256::from(amount),
    };
    call.abi_encode()
}

/// Encode calldata for buyShares.
pub fn encode_buy_shares(market_id: [u8; 32], user: Address, outcome: u8, shares: u64) -> Vec<u8> {
    let call = IPredictionMarket::buySharesCall {
        marketId: FixedBytes::from(market_id),
        user,
        outcome,
        shares: U256::from(shares),
    };
    call.abi_encode()
}

/// Encode calldata for sellShares.
pub fn encode_sell_shares(market_id: [u8; 32], user: Address, outcome: u8, shares: u64) -> Vec<u8> {
    let call = IPredictionMarket::sellSharesCall {
        marketId: FixedBytes::from(market_id),
        user,
        outcome,
        shares: U256::from(shares),
    };
    call.abi_encode()
}

/// Encode calldata for resolveMarket.
pub fn encode_resolve_market(market_id: [u8; 32], outcome: u8) -> Vec<u8> {
    let call = IPredictionMarket::resolveMarketCall {
        marketId: FixedBytes::from(market_id),
        outcome,
    };
    call.abi_encode()
}

/// Convert a hex string market ID to bytes32.
pub fn market_id_to_bytes32(id: &str) -> anyhow::Result<[u8; 32]> {
    let id = id.strip_prefix("0x").unwrap_or(id);
    let bytes = hex::decode(id)?;

    if bytes.len() != 32 {
        anyhow::bail!("Market ID must be 32 bytes, got {}", bytes.len());
    }

    let mut result = [0u8; 32];
    result.copy_from_slice(&bytes);
    Ok(result)
}

/// Convert bytes32 to hex string.
pub fn bytes32_to_hex(bytes: &[u8; 32]) -> String {
    format!("0x{}", hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_create_market() {
        let market_id = [0u8; 32];
        let calldata = encode_create_market(market_id, "Will BTC hit 100k?", 1800000000);

        // Should start with function selector (4 bytes)
        assert!(calldata.len() > 4);

        // Verify it's valid ABI encoding by checking length
        // bytes32 (32) + string offset (32) + uint256 (32) + string length (32) + string data (rounded up)
        assert!(calldata.len() >= 4 + 32 + 32 + 32);
    }

    #[test]
    fn test_encode_buy_shares() {
        let market_id = [1u8; 32];
        let user = Address::ZERO;
        let calldata = encode_buy_shares(market_id, user, 1, 100);

        // Should have selector + 4 params
        assert!(calldata.len() >= 4 + 32 * 4);
    }

    #[test]
    fn test_market_id_conversion() {
        let hex_id = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let bytes = market_id_to_bytes32(hex_id).unwrap();
        let back = bytes32_to_hex(&bytes);
        assert_eq!(hex_id, back);
    }

    #[test]
    fn test_market_id_invalid_length() {
        let hex_id = "0x1234"; // Too short
        assert!(market_id_to_bytes32(hex_id).is_err());
    }
}
