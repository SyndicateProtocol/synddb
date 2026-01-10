//! Withdrawal submission for the Bridge contract
//!
//! This module handles submitting withdrawal messages to the Bridge contract.
//! It receives withdrawal data from the price oracle or other clients,
//! fetches the validator signature, and calls `initializeAndHandleMessage`.

use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, B256, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, error, info, warn};
use url::Url;

/// Request to submit a withdrawal to the Bridge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WithdrawalSubmissionRequest {
    /// Message ID (bytes32 hex)
    pub message_id: String,
    /// Target contract address
    pub target_address: String,
    /// Calldata for the target contract (hex encoded)
    pub payload: String,
    /// Native token amount (as string, usually "0" for pure calldata execution)
    pub native_token_amount: String,
    /// Sequencer signature (65 bytes hex)
    pub sequencer_signature: String,
    /// Sequence number from sequencer
    pub sequence: u64,
    /// Timestamp from sequencer
    pub timestamp: u64,
}

/// Response from withdrawal submission
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct WithdrawalSubmissionResponse {
    /// Transaction hash if successful
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Status: "submitted", "confirmed", or "failed"
    pub status: String,
}

/// Signature from validator
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ValidatorSignature {
    pub message_id: String,
    pub signature: String,
    pub signer: Address,
    pub signed_at: u64,
}

// Bridge contract interface
sol! {
    #[sol(rpc)]
    interface IBridge {
        /// `SequencerSignature` struct
        struct SequencerSignature {
            bytes signature;
            uint64 sequence;
            uint64 timestamp;
        }

        /// Initialize and handle a message in one transaction
        function initializeAndHandleMessage(
            bytes32 messageId,
            address targetAddress,
            bytes calldata payload,
            SequencerSignature calldata sequencerSignature,
            bytes[] calldata validatorSignatures,
            uint256 nativeTokenAmount
        ) external;
    }
}

/// Withdrawal submitter
pub(crate) struct WithdrawalSubmitter {
    rpc_url: String,
    bridge_address: Address,
    signer: PrivateKeySigner,
    validator_url: Option<String>,
    tx_confirmation_timeout: Duration,
}

impl std::fmt::Debug for WithdrawalSubmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WithdrawalSubmitter")
            .field("rpc_url", &self.rpc_url)
            .field("bridge_address", &self.bridge_address)
            .field("validator_url", &self.validator_url)
            .field("relayer_address", &self.signer.address())
            .finish()
    }
}

impl WithdrawalSubmitter {
    /// Create a new withdrawal submitter
    pub(crate) fn new(
        rpc_url: String,
        bridge_address: Address,
        private_key: &str,
        validator_url: Option<String>,
        tx_confirmation_timeout: Duration,
    ) -> anyhow::Result<Self> {
        let key_bytes = hex::decode(private_key.trim_start_matches("0x"))?;
        let signer = PrivateKeySigner::from_slice(&key_bytes)?;

        info!(
            bridge = %bridge_address,
            validator_url = ?validator_url,
            relayer = %signer.address(),
            "Withdrawal submitter initialized"
        );

        Ok(Self {
            rpc_url,
            bridge_address,
            signer,
            validator_url,
            tx_confirmation_timeout,
        })
    }

    /// Submit a withdrawal to the Bridge contract
    pub(crate) async fn submit_withdrawal(
        &self,
        request: &WithdrawalSubmissionRequest,
    ) -> anyhow::Result<WithdrawalSubmissionResponse> {
        // Parse message_id
        let message_id_hex = request
            .message_id
            .strip_prefix("0x")
            .unwrap_or(&request.message_id);
        let message_id_bytes = hex::decode(message_id_hex)?;
        if message_id_bytes.len() != 32 {
            anyhow::bail!("Invalid message_id length: expected 32 bytes");
        }
        let message_id = B256::from_slice(&message_id_bytes);

        // Parse target address
        let target_address: Address = request.target_address.parse()?;

        // Parse payload
        let payload_hex = request
            .payload
            .strip_prefix("0x")
            .unwrap_or(&request.payload);
        let payload = Bytes::from(hex::decode(payload_hex)?);

        // Parse amount
        let native_token_amount: u128 = request.native_token_amount.parse()?;
        let amount_u256 = U256::from(native_token_amount);

        // Parse sequencer signature
        let seq_sig_hex = request
            .sequencer_signature
            .strip_prefix("0x")
            .unwrap_or(&request.sequencer_signature);
        let seq_sig_bytes = Bytes::from(hex::decode(seq_sig_hex)?);

        // Fetch validator signature if validator URL is configured
        let validator_signatures = if let Some(validator_url) = &self.validator_url {
            match self
                .fetch_validator_signature(validator_url, &request.message_id)
                .await
            {
                Ok(sig) => vec![sig],
                Err(e) => {
                    warn!(error = %e, message_id = %request.message_id, "Failed to fetch validator signature, proceeding without it");
                    vec![]
                }
            }
        } else {
            debug!("No validator URL configured, submitting without validator signatures");
            vec![]
        };

        // Build sequencer signature struct
        let sequencer_signature = IBridge::SequencerSignature {
            signature: seq_sig_bytes,
            sequence: request.sequence,
            timestamp: request.timestamp,
        };

        info!(
            message_id = %message_id,
            target = %target_address,
            payload_len = payload.len(),
            validator_sig_count = validator_signatures.len(),
            "Submitting withdrawal to Bridge"
        );

        // Submit to Bridge
        let wallet = EthereumWallet::from(self.signer.clone());
        let url = Url::parse(&self.rpc_url)?;
        let provider = ProviderBuilder::new().wallet(wallet).connect_http(url);

        let contract = IBridge::new(self.bridge_address, &provider);

        let tx = contract.initializeAndHandleMessage(
            message_id,
            target_address,
            payload,
            sequencer_signature,
            validator_signatures,
            amount_u256,
        );

        match tx.send().await {
            Ok(pending) => {
                let tx_hash = *pending.tx_hash();
                info!(tx_hash = %tx_hash, message_id = %message_id, "Withdrawal transaction submitted");

                // Wait for confirmation
                match self.wait_for_confirmation(&provider, tx_hash).await {
                    Ok(()) => Ok(WithdrawalSubmissionResponse {
                        tx_hash: Some(format!("{tx_hash:#x}")),
                        error: None,
                        status: "confirmed".to_string(),
                    }),
                    Err(e) => Ok(WithdrawalSubmissionResponse {
                        tx_hash: Some(format!("{tx_hash:#x}")),
                        error: Some(e.to_string()),
                        status: "submitted".to_string(),
                    }),
                }
            }
            Err(e) => {
                error!(error = %e, message_id = %message_id, "Failed to submit withdrawal");
                Ok(WithdrawalSubmissionResponse {
                    tx_hash: None,
                    error: Some(e.to_string()),
                    status: "failed".to_string(),
                })
            }
        }
    }

    /// Fetch validator signature from the validator's signature API
    async fn fetch_validator_signature(
        &self,
        validator_url: &str,
        message_id: &str,
    ) -> anyhow::Result<Bytes> {
        let url = format!(
            "{}/signature/{}",
            validator_url.trim_end_matches('/'),
            message_id
        );

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .timeout(Duration::from_secs(10))
            .send()
            .await?;

        if !response.status().is_success() {
            anyhow::bail!("Validator returned status {}", response.status());
        }

        let sig: ValidatorSignature = response.json().await?;
        let sig_hex = sig.signature.strip_prefix("0x").unwrap_or(&sig.signature);
        let sig_bytes = hex::decode(sig_hex)?;

        debug!(
            message_id = %message_id,
            signer = %sig.signer,
            "Fetched validator signature"
        );

        Ok(Bytes::from(sig_bytes))
    }

    /// Wait for transaction confirmation
    async fn wait_for_confirmation<P: Provider>(
        &self,
        provider: &P,
        tx_hash: B256,
    ) -> anyhow::Result<()> {
        let poll_interval = Duration::from_secs(2);
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > self.tx_confirmation_timeout {
                anyhow::bail!(
                    "Timeout waiting for tx confirmation after {:?}: {}",
                    self.tx_confirmation_timeout,
                    tx_hash
                );
            }

            match provider.get_transaction_receipt(tx_hash).await {
                Ok(Some(receipt)) => {
                    if receipt.status() {
                        info!(tx_hash = %tx_hash, "Transaction confirmed");
                        return Ok(());
                    }
                    anyhow::bail!("Transaction reverted: {}", tx_hash);
                }
                Ok(None) => {
                    debug!(tx_hash = %tx_hash, "Transaction pending...");
                }
                Err(e) => {
                    warn!(error = %e, "Error checking receipt");
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}
