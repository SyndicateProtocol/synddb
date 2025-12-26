//! Bridge HTTP client with retry logic and status polling.
//!
//! This demonstrates the complexity of message-passing compared to SQLite:
//! - Must handle transient failures with retry
//! - Must poll for completion status
//! - Must classify errors as retryable vs permanent
//! - Must manage nonces to prevent replay

use std::time::Duration;

use alloy::primitives::Address;
use anyhow::{Context, Result};
use serde::Deserialize;
use tracing::{debug, info, warn};

use super::encoding::{self, signatures};
use super::types::{is_retryable_error, stage_to_status, MessageStatus, PushResult};
use crate::types::Outcome;

/// Bridge client for submitting messages to the validator.
#[allow(missing_debug_implementations)] // Contains AtomicU64 which doesn't implement Debug
pub struct BridgeClient {
    /// Validator HTTP URL.
    validator_url: String,
    /// Application domain (32 bytes, hex).
    domain: String,
    /// HTTP client.
    http_client: reqwest::Client,
    /// Maximum retry attempts.
    max_retries: u32,
    /// Initial retry delay.
    retry_delay: Duration,
    /// Current nonce.
    nonce: std::sync::atomic::AtomicU64,
}

impl BridgeClient {
    /// Create a new Bridge client.
    pub fn new(
        validator_url: &str,
        domain: &str,
        max_retries: u32,
        retry_delay: Duration,
    ) -> Self {
        Self {
            validator_url: validator_url.trim_end_matches('/').to_string(),
            domain: domain.to_string(),
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            max_retries,
            retry_delay,
            nonce: std::sync::atomic::AtomicU64::new(1),
        }
    }

    /// Get the next nonce.
    fn next_nonce(&self) -> u64 {
        self.nonce
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    /// Build a message payload.
    fn build_message(
        &self,
        message_type: &str,
        calldata: Vec<u8>,
        metadata: serde_json::Value,
    ) -> serde_json::Value {
        let nonce = self.next_nonce();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("Time went backwards")
            .as_secs();

        serde_json::json!({
            "messageType": message_type,
            "calldata": format!("0x{}", hex::encode(&calldata)),
            "metadata": metadata,
            "nonce": nonce,
            "timestamp": timestamp,
            "domain": self.domain,
        })
    }

    /// Submit a message to the validator.
    async fn submit_message(&self, message: serde_json::Value) -> PushResult {
        let url = format!("{}/messages", self.validator_url);

        match self.http_client.post(&url).json(&message).send().await {
            Ok(response) => {
                let status = response.status();
                match response.json::<ValidatorResponse>().await {
                    Ok(resp) if status.is_success() && resp.status == "accepted" => {
                        PushResult::success(
                            resp.message_id.unwrap_or_default(),
                            resp.signature,
                        )
                    }
                    Ok(resp) => {
                        let error = resp.error.unwrap_or_default();
                        let error_code = error
                            .get("code")
                            .and_then(|v| v.as_str())
                            .unwrap_or("UNKNOWN");
                        let error_message = error
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("Unknown error");
                        PushResult::failure(error_code, error_message, is_retryable_error(error_code))
                    }
                    Err(e) => {
                        PushResult::failure("PARSE_ERROR", &e.to_string(), true)
                    }
                }
            }
            Err(e) if e.is_timeout() => {
                PushResult::failure("TIMEOUT", "Request timed out", true)
            }
            Err(e) if e.is_connect() => {
                PushResult::failure("CONNECTION_ERROR", "Failed to connect to validator", true)
            }
            Err(e) => {
                PushResult::failure("HTTP_ERROR", &e.to_string(), true)
            }
        }
    }

    /// Submit with retry logic.
    async fn submit_with_retry(&self, message_type: &str, calldata: Vec<u8>, metadata: serde_json::Value) -> PushResult {
        let mut attempts = 0u32;
        let mut delay = self.retry_delay;

        while attempts < self.max_retries {
            attempts += 1;

            // Build fresh message with new nonce for each attempt
            let message = self.build_message(message_type, calldata.clone(), metadata.clone());
            let mut result = self.submit_message(message).await;
            result.attempts = attempts;

            if result.success {
                return result;
            }

            if !result.is_retryable {
                warn!(
                    error_code = ?result.error_code,
                    "Non-retryable error after {} attempt(s)",
                    attempts
                );
                return result;
            }

            if attempts < self.max_retries {
                info!(
                    error_code = ?result.error_code,
                    attempts,
                    max_retries = self.max_retries,
                    delay_ms = delay.as_millis(),
                    "Retryable error, waiting before retry"
                );
                tokio::time::sleep(delay).await;
                delay *= 2; // Exponential backoff
            }
        }

        PushResult {
            success: false,
            message_id: None,
            signature: None,
            error_code: Some("MAX_RETRIES_EXCEEDED".to_string()),
            error_message: Some(format!("Failed after {} attempts", attempts)),
            is_retryable: false,
            attempts,
        }
    }

    // =========================================================================
    // Market operations
    // =========================================================================

    /// Create a new prediction market.
    pub async fn create_market(
        &self,
        market_id: &str,
        question: &str,
        resolution_time: u64,
    ) -> Result<PushResult> {
        let market_id_bytes = encoding::market_id_to_bytes32(market_id)?;
        let calldata = encoding::encode_create_market(market_id_bytes, question, resolution_time);

        let metadata = serde_json::json!({
            "reason": "create_market",
            "market_id": market_id,
            "question": question,
            "resolution_time": resolution_time,
            "source": "prediction-market-bridge",
        });

        Ok(self.submit_with_retry(signatures::CREATE_MARKET, calldata, metadata).await)
    }

    /// Deposit funds for a user.
    pub async fn deposit(&self, user: &str, amount: u64) -> Result<PushResult> {
        let user_addr: Address = user.parse().context("Invalid user address")?;
        let calldata = encoding::encode_deposit(user_addr, amount);

        let metadata = serde_json::json!({
            "reason": "deposit",
            "user": user,
            "amount": amount,
            "source": "prediction-market-bridge",
        });

        Ok(self.submit_with_retry(signatures::DEPOSIT, calldata, metadata).await)
    }

    /// Buy shares in a market with retry.
    pub async fn buy_shares(
        &self,
        market_id: &str,
        user: &str,
        outcome: Outcome,
        shares: u64,
    ) -> Result<PushResult> {
        let market_id_bytes = encoding::market_id_to_bytes32(market_id)?;
        let user_addr: Address = user.parse().context("Invalid user address")?;
        let calldata = encoding::encode_buy_shares(market_id_bytes, user_addr, outcome.as_u8(), shares);

        let metadata = serde_json::json!({
            "reason": "buy_shares",
            "market_id": market_id,
            "user": user,
            "outcome": outcome.to_string(),
            "shares": shares,
            "source": "prediction-market-bridge",
        });

        Ok(self.submit_with_retry(signatures::BUY_SHARES, calldata, metadata).await)
    }

    /// Sell shares in a market with retry.
    pub async fn sell_shares(
        &self,
        market_id: &str,
        user: &str,
        outcome: Outcome,
        shares: u64,
    ) -> Result<PushResult> {
        let market_id_bytes = encoding::market_id_to_bytes32(market_id)?;
        let user_addr: Address = user.parse().context("Invalid user address")?;
        let calldata = encoding::encode_sell_shares(market_id_bytes, user_addr, outcome.as_u8(), shares);

        let metadata = serde_json::json!({
            "reason": "sell_shares",
            "market_id": market_id,
            "user": user,
            "outcome": outcome.to_string(),
            "shares": shares,
            "source": "prediction-market-bridge",
        });

        Ok(self.submit_with_retry(signatures::SELL_SHARES, calldata, metadata).await)
    }

    /// Resolve a market with retry.
    pub async fn resolve_market(&self, market_id: &str, outcome: Outcome) -> Result<PushResult> {
        let market_id_bytes = encoding::market_id_to_bytes32(market_id)?;
        let calldata = encoding::encode_resolve_market(market_id_bytes, outcome.as_u8());

        let metadata = serde_json::json!({
            "reason": "resolve_market",
            "market_id": market_id,
            "outcome": outcome.to_string(),
            "source": "prediction-market-bridge",
        });

        Ok(self.submit_with_retry(signatures::RESOLVE_MARKET, calldata, metadata).await)
    }

    // =========================================================================
    // Status polling
    // =========================================================================

    /// Get the current status of a message.
    pub async fn get_message_status(&self, message_id: &str) -> Result<MessageStatus> {
        let url = format!("{}/messages/{}", self.validator_url, message_id);

        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .context("Failed to get message status")?;

        if response.status().is_success() {
            let data: serde_json::Value = response.json().await?;
            let stage = data.get("stage").and_then(|v| v.as_u64()).unwrap_or(0) as u8;

            Ok(MessageStatus {
                message_id: message_id.to_string(),
                stage,
                status: stage_to_status(stage).to_string(),
                executed: data.get("executed").and_then(|v| v.as_bool()).unwrap_or(false),
                signatures_collected: data
                    .get("signaturesCollected")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32,
                signature_threshold: data
                    .get("signatureThreshold")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32,
                block_number: data.get("blockNumber").and_then(|v| v.as_u64()),
                tx_hash: data.get("txHash").and_then(|v| v.as_str()).map(String::from),
            })
        } else {
            Ok(MessageStatus {
                message_id: message_id.to_string(),
                stage: 0,
                status: "not_found".to_string(),
                executed: false,
                signatures_collected: 0,
                signature_threshold: 1,
                block_number: None,
                tx_hash: None,
            })
        }
    }

    /// Wait for a message to reach a terminal state.
    ///
    /// This demonstrates the polling pattern required for message-passing:
    /// - No push notifications, must poll
    /// - Must handle timeout
    /// - Multiple terminal states possible
    pub async fn wait_for_completion(
        &self,
        message_id: &str,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<MessageStatus> {
        let start = std::time::Instant::now();
        let mut last_stage = None;

        while start.elapsed() < timeout {
            let status = self.get_message_status(message_id).await?;

            // Log stage transitions
            if last_stage != Some(status.stage) {
                debug!(
                    message_id,
                    stage = status.stage,
                    status = %status.status,
                    "Message status update"
                );
                last_stage = Some(status.stage);
            }

            if status.is_terminal() {
                return Ok(status);
            }

            tokio::time::sleep(poll_interval).await;
        }

        // Timeout - return last known status
        let status = self.get_message_status(message_id).await?;
        warn!(
            message_id,
            stage = status.stage,
            elapsed_ms = start.elapsed().as_millis(),
            "Timeout waiting for message completion"
        );
        Ok(status)
    }
}

/// Response from the validator.
#[derive(Debug, Deserialize)]
struct ValidatorResponse {
    status: String,
    #[serde(rename = "messageId")]
    message_id: Option<String>,
    signature: Option<String>,
    error: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_message() {
        let client = BridgeClient::new(
            "http://localhost:8080",
            "0x0000000000000000000000000000000000000000000000000000000000000001",
            3,
            Duration::from_millis(100),
        );

        let message = client.build_message(
            "test(uint256)",
            vec![1, 2, 3, 4],
            serde_json::json!({"key": "value"}),
        );

        assert_eq!(message["messageType"], "test(uint256)");
        assert_eq!(message["calldata"], "0x01020304");
        assert_eq!(message["domain"], client.domain);
        assert!(message["nonce"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_nonce_increments() {
        let client = BridgeClient::new(
            "http://localhost:8080",
            "0x01",
            3,
            Duration::from_millis(100),
        );

        let n1 = client.next_nonce();
        let n2 = client.next_nonce();
        let n3 = client.next_nonce();

        assert_eq!(n2, n1 + 1);
        assert_eq!(n3, n2 + 1);
    }
}
