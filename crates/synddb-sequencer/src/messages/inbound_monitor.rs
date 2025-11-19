//! Blockchain event monitoring for inbound messages

use super::InboundMessage;
use anyhow::Result;
use tokio::sync::mpsc::Sender;

pub struct InboundMonitor {
    _chain_rpc_url: String,
    _bridge_contract: String,
    _message_tx: Sender<InboundMessage>,
}

impl InboundMonitor {
    pub fn new(
        chain_rpc_url: String,
        bridge_contract: String,
        message_tx: Sender<InboundMessage>,
    ) -> Self {
        Self {
            _chain_rpc_url: chain_rpc_url,
            _bridge_contract: bridge_contract,
            _message_tx: message_tx,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        // TODO: Monitor blockchain for deposit events
        // TODO: Use alloy to watch contract events
        // TODO: Send inbound messages to channel

        Ok(())
    }
}
