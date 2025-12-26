//! Price oracle invariants for validating price update messages.
//!
//! These invariants ensure metadata consistency with calldata for price updates.

use alloy::{primitives::U256, sol, sol_types::SolCall};
use async_trait::async_trait;

use crate::{error::ValidationError, types::Message};

use super::{Invariant, InvariantContext};

sol! {
    /// Function signature for updatePrice(string,uint256,uint256)
    function updatePrice(string asset, uint256 priceScaled, uint256 timestamp);
}

/// Invariant that verifies price oracle metadata matches calldata.
///
/// This invariant ensures that for `updatePrice(string,uint256,uint256)` messages:
/// - The `asset` in metadata matches the asset parameter in calldata
/// - The `price_scaled` in metadata matches the priceScaled parameter
/// - The `timestamp` in metadata matches the timestamp parameter
#[derive(Debug)]
pub struct PriceMetadataConsistencyInvariant;

impl PriceMetadataConsistencyInvariant {
    pub const fn new() -> Self {
        Self
    }

    fn extract_metadata_values(message: &Message) -> Result<(String, U256, u64), ValidationError> {
        let metadata = &message.metadata;

        let asset = metadata
            .get("asset")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ValidationError::InvariantViolated {
                invariant: "price_metadata_consistency".to_string(),
                message: "metadata missing 'asset' field".to_string(),
            })?
            .to_string();

        let price_scaled_str = metadata
            .get("price_scaled")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ValidationError::InvariantViolated {
                invariant: "price_metadata_consistency".to_string(),
                message: "metadata missing 'price_scaled' field".to_string(),
            })?;

        let price_scaled: U256 =
            price_scaled_str
                .parse()
                .map_err(|_| ValidationError::InvariantViolated {
                    invariant: "price_metadata_consistency".to_string(),
                    message: format!("invalid price_scaled value: {}", price_scaled_str),
                })?;

        let timestamp = metadata
            .get("timestamp")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| ValidationError::InvariantViolated {
                invariant: "price_metadata_consistency".to_string(),
                message: "metadata missing 'timestamp' field".to_string(),
            })?;

        Ok((asset, price_scaled, timestamp))
    }

    fn decode_calldata(calldata: &[u8]) -> Result<(String, U256, U256), ValidationError> {
        let call = updatePriceCall::abi_decode(calldata).map_err(|e| {
            ValidationError::InvariantViolated {
                invariant: "price_metadata_consistency".to_string(),
                message: format!("failed to decode updatePrice calldata: {}", e),
            }
        })?;

        Ok((call.asset, call.priceScaled, call.timestamp))
    }
}

impl Default for PriceMetadataConsistencyInvariant {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Invariant for PriceMetadataConsistencyInvariant {
    fn name(&self) -> &str {
        "price_metadata_consistency"
    }

    async fn check(
        &self,
        message: &Message,
        _ctx: &InvariantContext,
    ) -> Result<(), ValidationError> {
        // Only apply to updatePrice messages
        if message.message_type != "updatePrice(string,uint256,uint256)" {
            return Ok(());
        }

        // Extract values from metadata
        let (meta_asset, meta_price, meta_timestamp) = Self::extract_metadata_values(message)?;

        // Decode calldata
        let (call_asset, call_price, call_timestamp) = Self::decode_calldata(&message.calldata)?;

        // Compare asset
        if meta_asset != call_asset {
            return Err(ValidationError::InvariantViolated {
                invariant: "price_metadata_consistency".to_string(),
                message: format!(
                    "asset mismatch: metadata='{}', calldata='{}'",
                    meta_asset, call_asset
                ),
            });
        }

        // Compare price
        if meta_price != call_price {
            return Err(ValidationError::InvariantViolated {
                invariant: "price_metadata_consistency".to_string(),
                message: format!(
                    "price_scaled mismatch: metadata={}, calldata={}",
                    meta_price, call_price
                ),
            });
        }

        // Compare timestamp (metadata is u64, calldata is U256)
        let call_timestamp_u64: u64 =
            call_timestamp
                .try_into()
                .map_err(|_| ValidationError::InvariantViolated {
                    invariant: "price_metadata_consistency".to_string(),
                    message: "calldata timestamp exceeds u64 range".to_string(),
                })?;

        if meta_timestamp != call_timestamp_u64 {
            return Err(ValidationError::InvariantViolated {
                invariant: "price_metadata_consistency".to_string(),
                message: format!(
                    "timestamp mismatch: metadata={}, calldata={}",
                    meta_timestamp, call_timestamp_u64
                ),
            });
        }

        Ok(())
    }
}

/// Invariant that validates price difference is within acceptable bounds.
///
/// This invariant ensures that for price updates with `price_diff_bps` metadata,
/// the difference is within the configured maximum.
#[derive(Debug)]
pub struct PriceDivergenceInvariant {
    max_diff_bps: u32,
}

impl PriceDivergenceInvariant {
    pub const fn new(max_diff_bps: u32) -> Self {
        Self { max_diff_bps }
    }
}

#[async_trait]
impl Invariant for PriceDivergenceInvariant {
    fn name(&self) -> &str {
        "price_divergence"
    }

    async fn check(
        &self,
        message: &Message,
        _ctx: &InvariantContext,
    ) -> Result<(), ValidationError> {
        // Only apply to updatePrice messages
        if message.message_type != "updatePrice(string,uint256,uint256)" {
            return Ok(());
        }

        // Check if price_diff_bps is present in metadata
        let Some(diff_bps) = message.metadata.get("price_diff_bps") else {
            // No divergence data, skip check
            return Ok(());
        };

        let diff_bps = diff_bps
            .as_u64()
            .ok_or_else(|| ValidationError::InvariantViolated {
                invariant: "price_divergence".to_string(),
                message: "price_diff_bps must be an integer".to_string(),
            })? as u32;

        if diff_bps > self.max_diff_bps {
            let asset = message
                .metadata
                .get("asset")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");

            return Err(ValidationError::InvariantViolated {
                invariant: "price_divergence".to_string(),
                message: format!(
                    "price divergence for '{}' is {} bps (max allowed: {} bps)",
                    asset, diff_bps, self.max_diff_bps
                ),
            });
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_update_price_message(asset: &str, price_scaled: &str, timestamp: u64) -> Message {
        // Encode the calldata using the sol! macro
        let price: U256 = price_scaled.parse().unwrap();
        let ts = U256::from(timestamp);
        let call = updatePriceCall {
            asset: asset.to_string(),
            priceScaled: price,
            timestamp: ts,
        };
        let calldata = call.abi_encode();

        Message {
            id: [0u8; 32],
            message_type: "updatePrice(string,uint256,uint256)".to_string(),
            calldata,
            metadata: json!({
                "reason": "price_update",
                "asset": asset,
                "price_scaled": price_scaled,
                "timestamp": timestamp,
                "source": "test"
            }),
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp,
            domain: [0u8; 32],
            value: None,
        }
    }

    #[tokio::test]
    async fn test_metadata_consistency_pass() {
        let invariant = PriceMetadataConsistencyInvariant::new();
        let ctx = InvariantContext::new();

        let message = make_update_price_message("bitcoin", "67196645000000000000000", 1735200000);

        let result = invariant.check(&message, &ctx).await;
        assert!(result.is_ok(), "Expected Ok, got {:?}", result);
    }

    #[tokio::test]
    async fn test_metadata_consistency_asset_mismatch() {
        let invariant = PriceMetadataConsistencyInvariant::new();
        let ctx = InvariantContext::new();

        // Create message with mismatched asset in metadata
        let mut message =
            make_update_price_message("bitcoin", "67196645000000000000000", 1735200000);
        message.metadata = json!({
            "reason": "price_update",
            "asset": "ethereum",  // Mismatch!
            "price_scaled": "67196645000000000000000",
            "timestamp": 1735200000,
            "source": "test"
        });

        let result = invariant.check(&message, &ctx).await;
        assert!(matches!(
            result,
            Err(ValidationError::InvariantViolated { .. })
        ));
    }

    #[tokio::test]
    async fn test_metadata_consistency_price_mismatch() {
        let invariant = PriceMetadataConsistencyInvariant::new();
        let ctx = InvariantContext::new();

        let mut message =
            make_update_price_message("bitcoin", "67196645000000000000000", 1735200000);
        message.metadata = json!({
            "reason": "price_update",
            "asset": "bitcoin",
            "price_scaled": "99999999999999999999999",  // Mismatch!
            "timestamp": 1735200000,
            "source": "test"
        });

        let result = invariant.check(&message, &ctx).await;
        assert!(matches!(
            result,
            Err(ValidationError::InvariantViolated { .. })
        ));
    }

    #[tokio::test]
    async fn test_price_divergence_within_limit() {
        let invariant = PriceDivergenceInvariant::new(100); // 1% max
        let ctx = InvariantContext::new();

        let mut message =
            make_update_price_message("bitcoin", "67196645000000000000000", 1735200000);
        message.metadata = json!({
            "reason": "price_update",
            "asset": "bitcoin",
            "price_scaled": "67196645000000000000000",
            "timestamp": 1735200000,
            "source": "test",
            "price_diff_bps": 50  // 0.5%, within limit
        });

        let result = invariant.check(&message, &ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_price_divergence_exceeds_limit() {
        let invariant = PriceDivergenceInvariant::new(100); // 1% max
        let ctx = InvariantContext::new();

        let mut message =
            make_update_price_message("bitcoin", "67196645000000000000000", 1735200000);
        message.metadata = json!({
            "reason": "price_update",
            "asset": "bitcoin",
            "price_scaled": "67196645000000000000000",
            "timestamp": 1735200000,
            "source": "test",
            "price_diff_bps": 500  // 5%, exceeds limit!
        });

        let result = invariant.check(&message, &ctx).await;
        assert!(matches!(
            result,
            Err(ValidationError::InvariantViolated { .. })
        ));
    }

    #[tokio::test]
    async fn test_skips_non_price_messages() {
        let invariant = PriceMetadataConsistencyInvariant::new();
        let ctx = InvariantContext::new();

        let message = Message {
            id: [0u8; 32],
            message_type: "setValue(uint256)".to_string(),
            calldata: vec![],
            metadata: json!({}), // Invalid metadata for price, but should be skipped
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1735200000,
            domain: [0u8; 32],
            value: None,
        };

        let result = invariant.check(&message, &ctx).await;
        assert!(result.is_ok());
    }
}
