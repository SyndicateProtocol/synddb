//! Batch metadata and utilities shared between sequencer and validator
//!
//! This module provides common types for working with message batches across
//! the `SyndDB` system. Both the sequencer (publishing) and validator (fetching)
//! use these types for consistent batch handling.

use serde::{Deserialize, Serialize};

/// Metadata about a batch of messages in storage
///
/// Used by both the sequencer (for tracking published batches) and the validator
/// (for building an in-memory index of available batches).
///
/// The `path` field contains a transport-specific reference:
/// - GCS: `gs://bucket/prefix/batches/000000000001_000000000050.cbor.zst`
/// - Local: `local://1`
/// - HTTP: `http://sequencer:8433/storage/batches/1`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchInfo {
    /// First sequence number in this batch (inclusive)
    pub start_sequence: u64,
    /// Last sequence number in this batch (inclusive)
    pub end_sequence: u64,
    /// Transport-specific path or reference to fetch this batch
    pub path: String,
    /// Content hash of the batch (SHA-256), if known
    ///
    /// This may be `[0u8; 32]` if the hash wasn't computed during listing
    /// (e.g., when listing from GCS without downloading each batch).
    #[serde(default, skip_serializing_if = "is_zero_hash")]
    pub content_hash: [u8; 32],
}

fn is_zero_hash(hash: &[u8; 32]) -> bool {
    hash.iter().all(|&b| b == 0)
}

impl BatchInfo {
    /// Create a new `BatchInfo` without a content hash
    pub fn new(start_sequence: u64, end_sequence: u64, path: impl Into<String>) -> Self {
        Self {
            start_sequence,
            end_sequence,
            path: path.into(),
            content_hash: [0u8; 32],
        }
    }

    /// Create a new `BatchInfo` with a content hash
    pub fn with_hash(
        start_sequence: u64,
        end_sequence: u64,
        path: impl Into<String>,
        content_hash: [u8; 32],
    ) -> Self {
        Self {
            start_sequence,
            end_sequence,
            path: path.into(),
            content_hash,
        }
    }

    /// Check if this batch contains the given sequence number
    pub const fn contains(&self, sequence: u64) -> bool {
        sequence >= self.start_sequence && sequence <= self.end_sequence
    }

    /// Number of messages in this batch
    pub const fn len(&self) -> u64 {
        if self.is_empty() {
            0
        } else {
            self.end_sequence - self.start_sequence + 1
        }
    }

    /// Check if batch is empty (should never happen in practice)
    pub const fn is_empty(&self) -> bool {
        self.end_sequence < self.start_sequence
    }

    /// Check if the content hash is known (non-zero)
    pub fn has_content_hash(&self) -> bool {
        !is_zero_hash(&self.content_hash)
    }
}

/// Parse a batch filename to extract start and end sequence numbers
///
/// Expected format: `{start:012}_{end:012}.cbor.zst`
///
/// # Examples
///
/// ```
/// use synddb_shared::types::batch::parse_batch_filename;
///
/// assert_eq!(parse_batch_filename("000000000001_000000000050.cbor.zst"), Some((1, 50)));
/// assert_eq!(parse_batch_filename("invalid.txt"), None);
/// ```
pub fn parse_batch_filename(filename: &str) -> Option<(u64, u64)> {
    let without_ext = filename.strip_suffix(".cbor.zst")?;
    let mut parts = without_ext.split('_');
    let start = parts.next()?.parse::<u64>().ok()?;
    let end = parts.next()?.parse::<u64>().ok()?;
    // Ensure no extra parts
    if parts.next().is_some() {
        return None;
    }
    Some((start, end))
}

/// Format a batch filename from start and end sequence numbers
///
/// Creates the standard batch filename format: `{start:012}_{end:012}.cbor.zst`
///
/// # Examples
///
/// ```
/// use synddb_shared::types::batch::format_batch_filename;
///
/// assert_eq!(format_batch_filename(1, 50), "000000000001_000000000050.cbor.zst");
/// ```
pub fn format_batch_filename(start_sequence: u64, end_sequence: u64) -> String {
    format!("{start_sequence:012}_{end_sequence:012}.cbor.zst")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_info_contains() {
        let info = BatchInfo::new(10, 20, "batch.cbor.zst");

        assert!(!info.contains(9));
        assert!(info.contains(10));
        assert!(info.contains(15));
        assert!(info.contains(20));
        assert!(!info.contains(21));
    }

    #[test]
    fn test_batch_info_len() {
        let info = BatchInfo::new(1, 50, "batch.cbor.zst");
        assert_eq!(info.len(), 50);

        let single = BatchInfo::new(42, 42, "single.cbor.zst");
        assert_eq!(single.len(), 1);
    }

    #[test]
    fn test_batch_info_is_empty() {
        let normal = BatchInfo::new(1, 10, "batch.cbor.zst");
        assert!(!normal.is_empty());

        let single = BatchInfo::new(5, 5, "single.cbor.zst");
        assert!(!single.is_empty());
    }

    #[test]
    fn test_batch_info_with_hash() {
        let hash = [1u8; 32];
        let info = BatchInfo::with_hash(1, 50, "batch.cbor.zst", hash);

        assert!(info.has_content_hash());
        assert_eq!(info.content_hash, hash);
    }

    #[test]
    fn test_batch_info_no_hash() {
        let info = BatchInfo::new(1, 50, "batch.cbor.zst");

        assert!(!info.has_content_hash());
    }

    #[test]
    fn test_parse_batch_filename_valid() {
        assert_eq!(
            parse_batch_filename("000000000001_000000000050.cbor.zst"),
            Some((1, 50))
        );
        assert_eq!(
            parse_batch_filename("000000001000_000000002000.cbor.zst"),
            Some((1000, 2000))
        );
        assert_eq!(
            parse_batch_filename("000000000042_000000000042.cbor.zst"),
            Some((42, 42))
        );
    }

    #[test]
    fn test_parse_batch_filename_invalid() {
        // Wrong extension
        assert_eq!(parse_batch_filename("000000000001_000000000050.json"), None);

        // Missing extension
        assert_eq!(parse_batch_filename("000000000001_000000000050"), None);

        // Extra underscore
        assert_eq!(
            parse_batch_filename("000000000001_000000000050_extra.cbor.zst"),
            None
        );

        // Non-numeric
        assert_eq!(parse_batch_filename("abcdef_ghijkl.cbor.zst"), None);

        // Empty
        assert_eq!(parse_batch_filename(""), None);

        // Just .zst (not .cbor.zst)
        assert_eq!(parse_batch_filename("000000000001_000000000050.zst"), None);
    }

    #[test]
    fn test_format_batch_filename() {
        assert_eq!(
            format_batch_filename(1, 50),
            "000000000001_000000000050.cbor.zst"
        );
        assert_eq!(
            format_batch_filename(0, 0),
            "000000000000_000000000000.cbor.zst"
        );
        assert_eq!(
            format_batch_filename(999_999_999_999, 999_999_999_999),
            "999999999999_999999999999.cbor.zst"
        );
    }

    #[test]
    fn test_batch_filename_roundtrip() {
        let start = 12345;
        let end = 67890;
        let filename = format_batch_filename(start, end);
        let (parsed_start, parsed_end) = parse_batch_filename(&filename).unwrap();
        assert_eq!(parsed_start, start);
        assert_eq!(parsed_end, end);
    }

    #[test]
    fn test_batch_filename_sorting() {
        let mut filenames = vec![
            format_batch_filename(51, 100),
            format_batch_filename(1, 50),
            format_batch_filename(101, 150),
        ];
        filenames.sort();

        assert_eq!(
            filenames,
            vec![
                "000000000001_000000000050.cbor.zst",
                "000000000051_000000000100.cbor.zst",
                "000000000101_000000000150.cbor.zst",
            ]
        );
    }
}
