//! Example contract event definitions.
//!
//! This module provides example event definitions for a typical Bridge contract
//! using Alloy's `sol!` macro. These serve as templates for your own contract events.

use alloy::sol;

// SAMPLE Bridge contract events
// TODO: Update with finalized Bridge contract events
// TODO: Test what happens if Bridge interacts with multiple contracts and which events are/aren't recorded
sol! {
    /// Emitted when a user deposits tokens into the bridge.
    ///
    /// # Event Parameters
    ///
    /// * `from` - The address depositing tokens (indexed)
    /// * `to` - The destination address on the L2 (indexed)
    /// * `amount` - Amount of tokens deposited
    /// * `data` - Optional additional data (e.g., memo, payload)
    #[derive(Debug)]
    event Deposit(
        address indexed from,
        address indexed to,
        uint256 amount,
        bytes data
    );

    /// Emitted when a user withdraws tokens from the bridge.
    ///
    /// # Event Parameters
    ///
    /// * `from` - The address withdrawing tokens (indexed)
    /// * `amount` - Amount of tokens withdrawn
    /// * `recipient` - The recipient address on L1
    /// * `data` - Optional additional data
    #[derive(Debug)]
    event Withdrawal(
        address indexed from,
        uint256 amount,
        address recipient,
        bytes data
    );

    /// Emitted when the bridge state is synchronized.
    ///
    /// # Event Parameters
    ///
    /// * `blockNumber` - The L2 block number being synchronized (indexed)
    /// * `stateRoot` - The state root at this block
    /// * `proof` - Merkle proof for verification
    #[derive(Debug)]
    event StateSync(
        uint256 indexed blockNumber,
        bytes32 stateRoot,
        bytes proof
    );

    /// Emitted when bridge ownership is transferred.
    ///
    /// # Event Parameters
    ///
    /// * `previousOwner` - Previous owner address (indexed)
    /// * `newOwner` - New owner address (indexed)
    #[derive(Debug)]
    event OwnershipTransferred(
        address indexed previousOwner,
        address indexed newOwner
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        primitives::{Address, Bytes, B256, U256},
        sol_types::SolEvent,
    };

    #[test]
    fn test_deposit_event_signature() {
        // Verify the event signature hash is computed correctly
        let signature = Deposit::SIGNATURE_HASH;
        assert_ne!(signature, B256::ZERO);

        // The signature should be consistent
        let signature2 = Deposit::SIGNATURE_HASH;
        assert_eq!(signature, signature2);
    }

    #[test]
    fn test_deposit_event_construction() {
        // Create a deposit event
        let from = Address::from([0x01; 20]);
        let to = Address::from([0x02; 20]);
        let amount = U256::from(1000);
        let data = Bytes::from(vec![0x42, 0x43]);

        let deposit = Deposit {
            from,
            to,
            amount,
            data,
        };

        assert_eq!(deposit.from, from);
        assert_eq!(deposit.to, to);
        assert_eq!(deposit.amount, amount);
    }

    #[test]
    fn test_withdrawal_event_signature() {
        let signature = Withdrawal::SIGNATURE_HASH;
        assert_ne!(signature, B256::ZERO);
    }

    #[test]
    fn test_state_sync_event_signature() {
        let signature = StateSync::SIGNATURE_HASH;
        assert_ne!(signature, B256::ZERO);
    }

    #[test]
    fn test_ownership_transferred_event_signature() {
        let signature = OwnershipTransferred::SIGNATURE_HASH;
        assert_ne!(signature, B256::ZERO);
    }

    #[test]
    fn test_all_signatures_are_unique() {
        // Verify all event signatures are unique
        let deposit_sig = Deposit::SIGNATURE_HASH;
        let withdrawal_sig = Withdrawal::SIGNATURE_HASH;
        let state_sync_sig = StateSync::SIGNATURE_HASH;
        let ownership_sig = OwnershipTransferred::SIGNATURE_HASH;

        assert_ne!(deposit_sig, withdrawal_sig);
        assert_ne!(deposit_sig, state_sync_sig);
        assert_ne!(deposit_sig, ownership_sig);
        assert_ne!(withdrawal_sig, state_sync_sig);
        assert_ne!(withdrawal_sig, ownership_sig);
        assert_ne!(state_sync_sig, ownership_sig);
    }
}
