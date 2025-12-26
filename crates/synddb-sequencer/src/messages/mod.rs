//! Message passing system for bidirectional communication with blockchain
//!
//! This module provides:
//! - Inbound message queue (blockchain -> application)
//! - Outbound message monitoring (application -> blockchain)
//! - HTTP API for message delivery to clients
//!
//! # Architecture
//!
//! ```text
//! Blockchain (Bridge.sol events)
//!     │
//!     ▼
//! InboundMonitor (captures events)
//!     │
//!     ▼
//! MessageQueue (in-memory, bounded)
//!     │
//!     ▼ GET /messages/inbound
//! Client Application
//!     │
//!     ▼ writes to message_log table
//! `SQLite` Database
//!     │
//!     ▼ (read-only)
//! OutboundMonitor (polls for new messages)
//!     │
//!     ▼
//! Bridge.sol (submits transactions)
//! ```

pub mod api;
pub mod outbound;
pub mod queue;

pub use api::{
    create_messages_router, GetMessagesQuery, InboundMessageResponse, MessageApiState,
    OutboundStatusResponse, PushInboundRequest, PushInboundResponse,
};
pub use outbound::{
    OutboundMonitor, OutboundMonitorConfig, OutboundMonitorHandle, OutboundStats, OutboundStatus,
    OutboundTracker, TrackedOutboundMessage,
};
pub use queue::{InboundMessage, MessageQueue, OutboundMessageStatus};
