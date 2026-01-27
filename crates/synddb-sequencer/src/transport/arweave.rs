//! Future: Arweave transport via ANS-104 bundles
//!
//! This module is a placeholder for future Arweave integration. When implemented,
//! it will wrap `CborBatch` data in ANS-104 `DataItems` for permanent storage on Arweave.
//!
//! # Architecture
//!
//! The `CborBatch` format is designed to be transport-agnostic:
//!
//! - **GCS:** Stores raw CBOR+zstd bytes directly
//! - **Arweave:** Wraps CBOR in ANS-104 `DataItem` with discovery tags
//!
//! The `content_hash` field in `CborBatch` (SHA-256 of serialized messages) enables
//! content-addressed lookup across storage systems, since Arweave uses signature-based
//! addressing (same content = different TX ID each upload).
//!
//! # Implementation Plan
//!
//! When implementing Arweave support:
//!
//! 1. **Add dependencies:**
//!    ```toml
//!    bundles-rs = "0.1"  # ANS-104 DataItem creation
//!    irys-sdk = "..."    # Or similar for bundler uploads
//!    ```
//!
//! 2. **Create ANS-104 `DataItem`:**
//!    - Data: `batch.to_cbor_zstd()` (compressed CBOR bytes)
//!    - Content-Type: `application/cbor+zstd`
//!    - Tags for discoverability:
//!      - `App-Name`: `SyndDB`
//!      - `App-Version`: `1`
//!      - `Schema-Version`: `1`
//!      - `Content-Type`: `application/cbor+zstd`
//!      - `Start-Sequence`: `{start_sequence}`
//!      - `End-Sequence`: `{end_sequence}`
//!      - `Content-SHA256`: `0x{hex(content_hash)}`
//!      - `Signer`: `0x{hex(signer_address)}`
//!      - `Created-At`: `{unix_timestamp}`
//!
//! 3. **Upload via Irys/Bundlr:**
//!    - Sign `DataItem` with Ethereum key (same as sequencer signer)
//!    - Upload to bundler for payment delegation
//!    - Store Arweave TX ID in response metadata
//!
//! 4. **Fetching batches:**
//!    - Query Arweave via GraphQL using tags
//!    - Example query to find batches by sequence range:
//!      ```graphql
//!      query {
//!        transactions(
//!          tags: [
//!            { name: "App-Name", values: ["SyndDB"] }
//!            { name: "Start-Sequence", values: ["1"] }
//!          ]
//!        ) {
//!          edges { node { id } }
//!        }
//!      }
//!      ```
//!    - Unwrap ANS-104, decompress, parse `CborBatch`
//!
//! 5. **Cross-referencing with GCS:**
//!    - Store Arweave TX ID in GCS object metadata
//!    - Content hash enables verification across systems
//!    - Validators can fetch from either source and verify content matches
//!
//! # ANS-104 Overhead
//!
//! ANS-104 `DataItems` add approximately:
//! - ~44 bytes for `DataItem` structure (signature type, owner, target, anchor, etc.)
//! - ~64 bytes for signature
//! - ~24 bytes per tag (name length + value length + overhead)
//!
//! With ~10 tags, expect ~300-400 bytes overhead per batch. This is acceptable
//! for batches containing multiple messages (amortized cost per message is low).
//!
//! # Example Implementation Sketch
//!
//! ```ignore
//! use crate::publish::transport::{TransportPublisher, TransportError, PublishMetadata, BatchInfo};
//! use synddb_shared::types::cbor::CborBatch;
//!
//! pub struct ArweaveTransport {
//!     irys_client: IrysClient,
//!     signer: EthereumSigner,
//! }
//!
//! #[async_trait]
//! impl TransportPublisher for ArweaveTransport {
//!     fn name(&self) -> &str { "arweave" }
//!
//!     async fn publish(&self, batch: &CborBatch) -> Result<PublishMetadata, TransportError> {
//!         let data = batch.to_cbor_zstd()?;
//!
//!         let tags = vec![
//!             ("App-Name", "SyndDB"),
//!             ("Schema-Version", "1"),
//!             ("Start-Sequence", &batch.start_sequence.to_string()),
//!             ("End-Sequence", &batch.end_sequence.to_string()),
//!             ("Content-SHA256", &batch.content_hash_hex()),
//!         ];
//!
//!         let data_item = DataItem::new(data)
//!             .with_tags(tags)
//!             .sign(&self.signer)?;
//!
//!         let tx_id = self.irys_client.upload(data_item).await?;
//!
//!         Ok(PublishMetadata {
//!             reference: format!("ar://{}", tx_id),
//!             content_hash: batch.content_hash,
//!             compressed_bytes: data.len(),
//!             uncompressed_bytes: batch.to_cbor()?.len(),
//!         })
//!     }
//!
//!     async fn fetch(&self, start_sequence: u64) -> Result<Option<CborBatch>, TransportError> {
//!         // Query Arweave GraphQL for matching transaction
//!         // Download data, unwrap ANS-104, decompress, parse
//!         todo!()
//!     }
//!
//!     async fn list_batches(&self) -> Result<Vec<BatchInfo>, TransportError> {
//!         // Query all SyndDB batches via GraphQL
//!         todo!()
//!     }
//!
//!     async fn get_latest_sequence(&self) -> Result<Option<u64>, TransportError> {
//!         // Query batches and find max end_sequence
//!         todo!()
//!     }
//! }
//! ```

// TODO: Implement ArweaveTransport - see module-level documentation above for detailed plan
#[derive(Debug)]
pub struct ArweaveTransport {
    _private: (),
}

impl ArweaveTransport {
    /// Create a new Arweave transport (not yet implemented)
    pub const fn new() -> Result<Self, &'static str> {
        Err("Arweave transport not yet implemented. See module documentation for implementation plan.")
    }
}
