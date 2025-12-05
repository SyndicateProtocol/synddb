//! DA fetch tests

use anyhow::ensure;
use synddb_shared::types::message::MessageType;

use crate::result::{TestCase, TestCaseResult};
use crate::runner::TestRunner;

impl TestRunner {
    /// Test: Can fetch messages from the DA layer
    pub(crate) async fn test_da_fetch(&self) -> TestCaseResult {
        TestCase::new("da_fetch", "DA message fetch works")
            .run(|| async {
                // Get the latest sequence from DA
                let latest = self.sequencer.da_latest().await?;
                ensure!(latest.sequence.is_some(), "No messages in DA layer");

                // Fetch message at sequence 0 (first message)
                let message = self.sequencer.fetch_da_message(0).await?;

                // Verify basic message structure
                ensure!(
                    message.sequence == 0,
                    "Expected sequence 0, got {}",
                    message.sequence
                );
                ensure!(
                    message.message_type == MessageType::Changeset,
                    "Expected Changeset type"
                );
                ensure!(
                    message.signer.starts_with("0x"),
                    "Signer should be hex address"
                );
                ensure!(
                    message.signature.starts_with("0x"),
                    "Signature should be hex"
                );
                ensure!(
                    message.message_hash.starts_with("0x"),
                    "Message hash should be hex"
                );

                Ok(())
            })
            .await
    }
}
