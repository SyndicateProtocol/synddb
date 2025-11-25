//! Ethereum client for interacting with blockchain RPC endpoints.
//!
//! This client provides robust blockchain interaction with automatic retry logic,
//! WebSocket subscriptions, and smart error handling.

use alloy::{
    eips::{BlockId, BlockNumberOrTag},
    providers::{Provider as _, ProviderBuilder, RootProvider, WsConnect},
    pubsub::Subscription,
    rpc::types::{Block, Filter, FilterBlockOption, Log},
    transports::{ws::WebSocketConfig, RpcError, TransportErrorKind},
};
use anyhow::{anyhow, Result};
use std::time::Duration;
use tokio::time::timeout;
use tracing::{error, info, warn};
use url::Url;

/// A client for interacting with an Ethereum-like blockchain.
///
/// This client is designed to retrieve blockchain data such as blocks and logs
/// by interacting with an Ethereum JSON-RPC endpoint via WebSocket.
///
/// # Features
///
/// - **Automatic retry logic**: All operations retry indefinitely until successful
/// - **WebSocket subscriptions**: Real-time event monitoring
/// - **Smart range splitting**: Automatically splits log queries that exceed provider limits
/// - **Configurable timeouts**: Per-operation timeout configuration
#[derive(Debug, Clone)]
pub struct EthClient {
    /// The underlying Alloy provider for Ethereum-like chains
    pub client: RootProvider,
    timeout: Duration,
    get_logs_timeout: Duration,
    retry_interval: Duration,
}

fn handle_rpc_error(name: &str, err: &RpcError<TransportErrorKind>) {
    error!("{}: {}", name, err);
    if let RpcError::Transport(err) = err {
        assert!(
            err.is_retry_err(),
            "{}: {}: {}",
            name,
            "fatal transport error",
            err
        );
    }
}

impl EthClient {
    /// Creates a new [`EthClient`] instance. Retries indefinitely until it is able to connect.
    ///
    /// # Arguments
    ///
    /// * `ws_urls` - List of WebSocket RPC URLs to try (will fallback through list)
    /// * `timeout` - Timeout for individual RPC requests
    /// * `get_logs_timeout` - Timeout specifically for `eth_getLogs` requests
    /// * `channel_size` - Size of the subscription channel buffer
    /// * `retry_interval` - Duration to wait between retry attempts
    ///
    /// # Returns
    ///
    /// A connected `EthClient` instance. This function will retry indefinitely
    /// until a connection is established.
    pub async fn new(
        ws_urls: Vec<Url>,
        timeout: Duration,
        get_logs_timeout: Duration,
        channel_size: usize,
        retry_interval: Duration,
    ) -> Self {
        loop {
            // Fallback to next ws url if the current one fails
            for ws_url in ws_urls.clone() {
                match tokio::time::timeout(
                    timeout,
                    ProviderBuilder::default().connect_ws(
                        WsConnect::new(ws_url).with_config(
                            WebSocketConfig::default()
                                .max_message_size(None)
                                .max_frame_size(None),
                        ),
                    ),
                )
                .await
                {
                    Err(_) => {
                        error!("timed out connecting to websocket");
                    }
                    Ok(Err(err)) => {
                        handle_rpc_error("failed to connect to websocket", &err);
                    }
                    Ok(Ok(client)) => {
                        client
                            .client()
                            .expect_pubsub_frontend()
                            .set_channel_size(channel_size);
                        return Self {
                            client,
                            timeout,
                            get_logs_timeout,
                            retry_interval,
                        };
                    }
                }
            }
            tokio::time::sleep(retry_interval).await;
        }
    }

    /// Retrieves latest block on the chain. Retries indefinitely until the request succeeds.
    pub async fn get_latest_block(&self) -> Option<Block> {
        loop {
            match timeout(self.timeout, self.client.get_block(BlockId::latest())).await {
                Err(_) => {
                    warn!("get_latest_block request timed out");
                }
                Ok(Err(err)) => {
                    handle_rpc_error("failed to fetch latest block", &err);
                }
                Ok(Ok(block)) => {
                    return block;
                }
            }
            tokio::time::sleep(self.retry_interval).await;
        }
    }

    /// Retrieves a specific block with a timeout. Retries indefinitely until the request
    /// succeeds.
    pub async fn get_block(&self, block_identifier: BlockNumberOrTag) -> Option<Block> {
        loop {
            match timeout(
                self.timeout,
                self.client.get_block(BlockId::Number(block_identifier)),
            )
            .await
            {
                Err(_) => {
                    warn!(%block_identifier, "get_block request timed out");
                }
                Ok(Err(err)) => {
                    handle_rpc_error("failed to fetch block", &err);
                }
                Ok(Ok(block)) => {
                    return block;
                }
            }
            tokio::time::sleep(self.retry_interval).await;
        }
    }

    /// Tries once to create a subscription to check if the client supports it.
    ///
    /// This is useful for detecting whether the RPC provider supports WebSocket
    /// subscriptions, allowing graceful fallback to polling mode.
    pub async fn try_subscribe_logs(&self, filter: &Filter) -> Result<Subscription<Log>> {
        self.client
            .subscribe_logs(filter)
            .await
            .map_err(|err| anyhow!("Failed to subscribe to logs: {:?}", err))
    }

    /// Subscribes to logs with a given filter over the websocket connection with a timeout.
    /// Retries indefinitely until the request succeeds.
    ///
    /// # Arguments
    ///
    /// * `filter` - The log filter specifying which events to subscribe to
    ///
    /// # Returns
    ///
    /// A `Subscription<Log>` that yields logs as they are emitted by the blockchain.
    pub async fn subscribe_logs(&self, filter: &Filter) -> Subscription<Log> {
        loop {
            match timeout(self.timeout, self.client.subscribe_logs(filter)).await {
                Err(_) => {
                    error!("eth_subscribe request timed out");
                }
                Ok(Err(err)) => {
                    handle_rpc_error("failed to subscribe to logs", &err);
                }
                Ok(Ok(sub)) => return sub,
            }
            tokio::time::sleep(self.retry_interval).await;
        }
    }

    /// Gets the chain id. Retries indefinitely until the request succeeds.
    pub async fn get_chain_id(&self) -> u64 {
        loop {
            match timeout(self.timeout, self.client.get_chain_id()).await {
                Err(_) => {
                    error!("eth_chainId request timed out");
                }
                Ok(Err(err)) => {
                    handle_rpc_error("failed to get chain id", &err);
                }
                Ok(Ok(chain_id)) => return chain_id,
            }
            tokio::time::sleep(self.retry_interval).await;
        }
    }

    /// Get logs, splitting the range in half if the request fails.
    ///
    /// This method handles provider limitations on log query ranges by automatically
    /// splitting large ranges into smaller chunks when the provider rejects the request.
    ///
    /// # Arguments
    ///
    /// * `filter` - The log filter specifying which logs to retrieve
    ///
    /// # Returns
    ///
    /// A vector of logs matching the filter, or an error if the request ultimately fails.
    #[allow(clippy::cognitive_complexity)]
    pub async fn get_logs(
        &self,
        filter: &Filter,
    ) -> Result<Vec<Log>, RpcError<TransportErrorKind>> {
        match timeout(self.get_logs_timeout, self.client.get_logs(filter)).await {
            Err(_) => {
                warn!(
                    "eth_getLogs request timed out. Attempting to split range: {:?}",
                    filter
                );
                self.handle_split_range(
                    filter,
                    TransportErrorKind::Custom("request timed out".into()).into(),
                )
                .await
            }
            Ok(Ok(x)) => Ok(x),
            Ok(Err(RpcError::ErrorResp(err))) => {
                warn!(
                    "eth_getLogs request failed. Attempting to split range: {:?}",
                    filter
                );
                self.handle_split_range(filter, RpcError::ErrorResp(err))
                    .await
            }
            Ok(Err(err)) => {
                handle_rpc_error("failed to get logs", &err);
                Err(err)
            }
        }
    }

    async fn handle_split_range(
        &self,
        filter: &Filter,
        err: RpcError<TransportErrorKind>,
    ) -> Result<Vec<Log>, RpcError<TransportErrorKind>> {
        // Only attempt to split the range if we have a valid block range
        let (from_block, to_block) = match filter.block_option {
            FilterBlockOption::Range {
                from_block: Some(BlockNumberOrTag::Number(from)),
                to_block: Some(BlockNumberOrTag::Number(to)),
            } => (from, to),
            _ => (0, 0),
        };

        // Error if the range is too small
        if to_block <= from_block {
            error!("failed to get logs ({:?})", filter);
            return Err(err);
        }

        // Split range in half and recursively fetch logs
        info!(
            "splitting eth_getLogs range ({} to {})",
            from_block, to_block
        );
        let mid = (from_block + to_block) / 2;
        let lower_range =
            Box::pin(self.get_logs(&filter.clone().from_block(from_block).to_block(mid))).await?;
        let upper_range =
            Box::pin(self.get_logs(&filter.clone().from_block(mid + 1).to_block(to_block))).await?;
        Ok([lower_range, upper_range].concat())
    }
}
