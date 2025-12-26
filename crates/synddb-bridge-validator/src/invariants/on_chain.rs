use alloy::{
    primitives::{Address, U256},
    sol,
};
use async_trait::async_trait;

use super::{Invariant, InvariantContext};
use crate::{error::ValidationError, types::Message};

sol! {
    #[sol(rpc)]
    interface IERC20 {
        function totalSupply() external view returns (uint256);
        function balanceOf(address account) external view returns (uint256);
    }
}

pub struct SupplyCapInvariant {
    token_address: Address,
    max_supply: U256,
}

impl SupplyCapInvariant {
    pub fn new(token_address: Address, max_supply: U256) -> Self {
        Self {
            token_address,
            max_supply,
        }
    }
}

#[async_trait]
impl Invariant for SupplyCapInvariant {
    fn name(&self) -> &str {
        "supply_cap"
    }

    async fn check(
        &self,
        message: &Message,
        ctx: &InvariantContext,
    ) -> Result<(), ValidationError> {
        let Some(provider) = ctx.create_provider() else {
            return Err(ValidationError::InvariantViolated {
                invariant: self.name().to_string(),
                message: "No RPC provider configured for on-chain invariant check".to_string(),
            });
        };

        // Extract mint amount from calldata if this is a mint message
        let mint_amount = extract_mint_amount(message)?;

        // Query current total supply
        let contract = IERC20::new(self.token_address, provider);
        let current_supply = contract.totalSupply().call().await.map_err(|e| {
            ValidationError::InvariantViolated {
                invariant: self.name().to_string(),
                message: format!("Failed to query totalSupply: {}", e),
            }
        })?;

        // Check if mint would exceed cap
        let new_supply = current_supply.saturating_add(mint_amount);
        if new_supply > self.max_supply {
            return Err(ValidationError::InvariantViolated {
                invariant: self.name().to_string(),
                message: format!(
                    "Mint would exceed supply cap: current={}, mint={}, max={}",
                    current_supply, mint_amount, self.max_supply
                ),
            });
        }

        Ok(())
    }
}

pub struct BalanceCheckInvariant {
    token_address: Address,
}

impl BalanceCheckInvariant {
    pub fn new(token_address: Address) -> Self {
        Self { token_address }
    }
}

#[async_trait]
impl Invariant for BalanceCheckInvariant {
    fn name(&self) -> &str {
        "balance_check"
    }

    async fn check(
        &self,
        message: &Message,
        ctx: &InvariantContext,
    ) -> Result<(), ValidationError> {
        let Some(provider) = ctx.create_provider() else {
            return Err(ValidationError::InvariantViolated {
                invariant: self.name().to_string(),
                message: "No RPC provider configured for on-chain invariant check".to_string(),
            });
        };

        // Extract transfer amount and sender from calldata if this is a transfer
        let (sender, amount) = match extract_transfer_details(message) {
            Ok(details) => details,
            Err(_) => return Ok(()), // Not a transfer message, skip
        };

        // Query sender's balance
        let contract = IERC20::new(self.token_address, provider);
        let balance = contract.balanceOf(sender).call().await.map_err(|e| {
            ValidationError::InvariantViolated {
                invariant: self.name().to_string(),
                message: format!("Failed to query balanceOf: {}", e),
            }
        })?;

        // Check if sender has sufficient balance
        if balance < amount {
            return Err(ValidationError::InvariantViolated {
                invariant: self.name().to_string(),
                message: format!(
                    "Insufficient balance: sender={}, balance={}, required={}",
                    sender, balance, amount
                ),
            });
        }

        Ok(())
    }
}

fn extract_mint_amount(message: &Message) -> Result<U256, ValidationError> {
    // Mint messages typically have signature: mint(address,uint256)
    // Calldata: 4-byte selector + 32-byte address + 32-byte amount
    if message.calldata.len() < 68 {
        return Err(ValidationError::InvariantViolated {
            invariant: "supply_cap".to_string(),
            message: "Calldata too short for mint".to_string(),
        });
    }

    // Extract amount from bytes 36-68 (after selector + address)
    let amount_bytes: [u8; 32] =
        message.calldata[36..68]
            .try_into()
            .map_err(|_| ValidationError::InvariantViolated {
                invariant: "supply_cap".to_string(),
                message: "Failed to extract amount from calldata".to_string(),
            })?;

    Ok(U256::from_be_bytes(amount_bytes))
}

fn extract_transfer_details(message: &Message) -> Result<(Address, U256), ValidationError> {
    // Transfer messages typically have signature: transfer(address,uint256)
    // or transferFrom(address,address,uint256)
    // For simplicity, we handle transfer(address,uint256) where sender is msg.sender

    // Check if this looks like a transfer
    if !message.message_type.starts_with("transfer") {
        return Err(ValidationError::InvariantViolated {
            invariant: "balance_check".to_string(),
            message: "Not a transfer message".to_string(),
        });
    }

    if message.calldata.len() < 68 {
        return Err(ValidationError::InvariantViolated {
            invariant: "balance_check".to_string(),
            message: "Calldata too short for transfer".to_string(),
        });
    }

    // For transferFrom(address from, address to, uint256 amount)
    if message.message_type.starts_with("transferFrom") && message.calldata.len() >= 100 {
        // Extract from address (bytes 4-36)
        let from_bytes: [u8; 32] =
            message.calldata[4..36]
                .try_into()
                .map_err(|_| ValidationError::InvariantViolated {
                    invariant: "balance_check".to_string(),
                    message: "Failed to extract from address".to_string(),
                })?;
        let from = Address::from_slice(&from_bytes[12..32]);

        // Extract amount (bytes 68-100)
        let amount_bytes: [u8; 32] = message.calldata[68..100].try_into().map_err(|_| {
            ValidationError::InvariantViolated {
                invariant: "balance_check".to_string(),
                message: "Failed to extract amount".to_string(),
            }
        })?;

        return Ok((from, U256::from_be_bytes(amount_bytes)));
    }

    // For transfer(address to, uint256 amount), we need sender from context
    // Since we don't have the sender in this context, we skip this check
    Err(ValidationError::InvariantViolated {
        invariant: "balance_check".to_string(),
        message: "Cannot determine sender for transfer message".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_mint_message(amount: U256) -> Message {
        // Selector for mint(address,uint256): 0x40c10f19
        let mut calldata = vec![0x40, 0xc1, 0x0f, 0x19];
        // Recipient address (32 bytes, padded)
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(&[
            0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc,
            0xde, 0xf0, 0x12, 0x34, 0x56, 0x78,
        ]);
        // Amount (32 bytes)
        calldata.extend_from_slice(&amount.to_be_bytes::<32>());

        Message {
            id: [0u8; 32],
            message_type: "mint(address,uint256)".to_string(),
            calldata,
            metadata: json!({}),
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1234567890,
            domain: [0u8; 32],
            value: None,
        }
    }

    #[test]
    fn test_extract_mint_amount() {
        let amount = U256::from(1000000000000000000u128); // 1 ether
        let message = make_mint_message(amount);
        let extracted = extract_mint_amount(&message).unwrap();
        assert_eq!(extracted, amount);
    }

    #[test]
    fn test_extract_mint_amount_short_calldata() {
        let message = Message {
            id: [0u8; 32],
            message_type: "mint(address,uint256)".to_string(),
            calldata: vec![0u8; 10], // Too short
            metadata: json!({}),
            metadata_hash: [0u8; 32],
            nonce: 1,
            timestamp: 1234567890,
            domain: [0u8; 32],
            value: None,
        };

        assert!(extract_mint_amount(&message).is_err());
    }
}
