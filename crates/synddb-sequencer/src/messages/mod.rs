//! Message passing module for inbound/outbound messages

pub mod alerts;
pub mod api;
pub mod consistency;
pub mod degradation;
pub mod inbound_monitor;
pub mod outbound_monitor;
pub mod queue;
pub mod recovery;
pub mod state_commitments;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Inbound(InboundMessage),
    Outbound(OutboundMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub id: u64,
    pub source_tx_hash: String,
    pub message_type: String,
    pub payload: Vec<u8>,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub id: u64,
    pub target_chain: String,
    pub message_type: String,
    pub payload: Vec<u8>,
    pub status: MessageStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageStatus {
    Pending,
    Processing,
    Published,
    Failed,
}
