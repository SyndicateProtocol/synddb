//! Gas drain utilities for key rotation
//!
//! When rotating to a new key, the old key should drain its remaining
//! gas balance to the treasury before being deactivated.

use crate::{BootstrapError, ContractSubmitter};
use alloy::{
    primitives::{Address, U256},
    signers::local::PrivateKeySigner,
};
use std::time::Duration;
use tracing::{info, warn};

/// Gas cost buffer for ETH transfer (21000 gas units)
const ETH_TRANSFER_GAS: u128 = 21_000;

/// Drain remaining ETH balance from an old key to the treasury
///
/// This should be called before the key is deactivated on-chain.
/// The function:
/// 1. Checks the current balance
/// 2. Estimates gas cost for transfer
/// 3. Sends remaining balance minus gas cost to treasury
/// 4. Waits for confirmation
///
/// # Arguments
///
/// * `submitter` - Contract submitter for RPC calls
/// * `old_key` - The key to drain (requires private key access)
/// * `treasury_address` - Address to send funds to
///
/// # Returns
///
/// The amount of ETH drained (in wei), or 0 if balance was too low
pub async fn drain_to_treasury(
    submitter: &ContractSubmitter,
    old_key: &PrivateKeySigner,
    treasury_address: Address,
) -> Result<U256, BootstrapError> {
    let old_address = old_key.address();

    // Get current balance
    let balance = submitter.get_balance(old_address).await?;

    if balance == 0 {
        info!(address = %old_address, "Old key has no balance to drain");
        return Ok(U256::ZERO);
    }

    // Get current gas price
    let gas_price = submitter.get_gas_price().await?;
    let gas_cost = ETH_TRANSFER_GAS * gas_price;

    if balance <= gas_cost {
        warn!(
            address = %old_address,
            balance_wei = balance,
            gas_cost_wei = gas_cost,
            "Balance too low to cover gas for drain transaction"
        );
        return Ok(U256::ZERO);
    }

    let amount_to_send = balance - gas_cost;

    info!(
        from = %old_address,
        to = %treasury_address,
        balance_wei = balance,
        gas_cost_wei = gas_cost,
        amount_to_send_wei = amount_to_send,
        "Draining old key balance to treasury"
    );

    // Submit the transfer
    let tx_hash = submitter
        .send_eth(old_key, treasury_address, amount_to_send)
        .await?;

    // Wait for confirmation (60 second timeout for simple transfer)
    submitter
        .wait_for_confirmation(tx_hash, Duration::from_secs(60))
        .await?;

    info!(
        tx_hash = %tx_hash,
        amount_wei = amount_to_send,
        "Successfully drained old key to treasury"
    );

    Ok(U256::from(amount_to_send))
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_eth_transfer_gas_constant() {
        // ETH simple transfer is always 21000 gas
        assert_eq!(super::ETH_TRANSFER_GAS, 21_000);
    }
}
