//! Message queue management

use super::{InboundMessage, OutboundMessage};
use std::collections::VecDeque;

#[derive(Debug)]
pub struct MessageQueue {
    inbound: VecDeque<InboundMessage>,
    outbound: VecDeque<OutboundMessage>,
}

impl MessageQueue {
    pub const fn new() -> Self {
        Self {
            inbound: VecDeque::new(),
            outbound: VecDeque::new(),
        }
    }

    pub fn push_inbound(&mut self, message: InboundMessage) {
        self.inbound.push_back(message);
    }

    pub fn push_outbound(&mut self, message: OutboundMessage) {
        self.outbound.push_back(message);
    }

    pub fn pop_inbound(&mut self) -> Option<InboundMessage> {
        self.inbound.pop_front()
    }

    pub fn pop_outbound(&mut self) -> Option<OutboundMessage> {
        self.outbound.pop_front()
    }
}

impl Default for MessageQueue {
    fn default() -> Self {
        Self::new()
    }
}
