//! Blockchain event monitoring for inbound messages

use super::InboundMessage;
use anyhow::Result;
use tokio::sync::mpsc::Sender;

pub struct InboundMonitor {
    chain_rpc_url: String,
    bridge_contract: String,
    message_tx: Sender<InboundMessage>,
}

impl InboundMonitor {
    pub fn new(
        chain_rpc_url: String,
        bridge_contract: String,
        message_tx: Sender<InboundMessage>,
    ) -> Self {
        Self {
            chain_rpc_url,
            bridge_contract,
            message_tx,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        // TODO: Monitor blockchain for deposit events
        // TODO: Use alloy to watch contract events
        // TODO: Send inbound messages to channel

        Ok(())
    }
}
