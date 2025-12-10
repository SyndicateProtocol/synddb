//! Batch index for efficient sequential sync
//!
//! The `BatchIndex` caches batch metadata from the storage layer, enabling efficient
//! sequential fetching without repeated list operations per message.
//!
//! # Usage
//!
//! ```rust,ignore
//! use synddb_validator::sync::batch_index::BatchIndex;
//!
//! let mut index = BatchIndex::build(&fetcher).await?;
//!
//! // Find batch containing a specific sequence
//! if let Some(info) = index.find_batch_containing(42) {
//!     let batch = fetcher.get_batch_by_path(&info.path).await?;
//! }
//!
//! // Detect gaps in sequence coverage
//! let gaps = index.detect_gaps(0);
//! ```

use crate::sync::fetcher::{BatchInfo, StorageFetcher};
use anyhow::Result;
use std::sync::Arc;
use tracing::{debug, info};

/// Cached index of batch metadata for efficient sequential sync
///
/// Maintains a sorted list of `BatchInfo` entries from the storage layer,
/// supporting binary search for sequence lookups and gap detection.
#[derive(Debug)]
pub struct BatchIndex {
    /// Sorted list of batch metadata (by `start_sequence`)
    batches: Vec<BatchInfo>,
}

impl BatchIndex {
    /// Build a batch index by listing all batches from the fetcher
    pub async fn build(fetcher: &Arc<dyn StorageFetcher>) -> Result<Self> {
        let batches = fetcher.list_batches().await?;
        info!(
            batch_count = batches.len(),
            fetcher = fetcher.name(),
            "Built batch index"
        );
        Ok(Self { batches })
    }

    /// Create an empty batch index
    pub const fn empty() -> Self {
        Self { batches: vec![] }
    }

    /// Number of batches in the index
    pub const fn len(&self) -> usize {
        self.batches.len()
    }

    /// Check if index is empty
    pub const fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }

    /// Get all batches (for iteration)
    pub fn batches(&self) -> &[BatchInfo] {
        &self.batches
    }

    /// Find the batch containing a specific sequence number
    ///
    /// Uses binary search for O(log n) lookups because batches are sorted by sequence number.
    /// Returns `None` if no batch contains the sequence.
    pub fn find_batch_containing(&self, sequence: u64) -> Option<&BatchInfo> {
        // Binary search for a batch where start_sequence <= sequence
        let idx = self
            .batches
            .partition_point(|b| b.start_sequence <= sequence);

        if idx == 0 {
            // sequence is before all batches
            return None;
        }

        // Check the batch before the partition point
        let batch = &self.batches[idx - 1];
        batch.contains(sequence).then_some(batch)
    }

    /// Find the first batch that starts after or at a given sequence
    ///
    /// Useful for finding the next batch to sync when resuming.
    pub fn find_first_batch_starting_at_or_after(&self, sequence: u64) -> Option<&BatchInfo> {
        let idx = self
            .batches
            .partition_point(|b| b.start_sequence < sequence);

        self.batches.get(idx)
    }

    /// Find the first batch that starts strictly after a given sequence
    pub fn find_first_batch_after(&self, sequence: u64) -> Option<&BatchInfo> {
        let idx = self
            .batches
            .partition_point(|b| b.start_sequence <= sequence);

        self.batches.get(idx)
    }

    /// Get the batch at a specific index
    pub fn get_batch_at_index(&self, index: usize) -> Option<&BatchInfo> {
        self.batches.get(index)
    }

    /// Find the index of a batch by its start sequence
    pub fn find_batch_index(&self, start_sequence: u64) -> Option<usize> {
        self.batches
            .binary_search_by_key(&start_sequence, |b| b.start_sequence)
            .ok()
    }

    /// Detect gaps in sequence coverage starting from a given sequence
    ///
    /// Returns a list of (`expected_start`, `actual_start`) tuples where gaps exist.
    /// A gap means sequences are missing between batches.
    pub fn detect_gaps(&self, from_sequence: u64) -> Vec<(u64, u64)> {
        let mut gaps = Vec::new();
        let mut expected = from_sequence;

        for batch in &self.batches {
            if batch.end_sequence < from_sequence {
                // Batch is entirely before our starting point
                continue;
            }

            if batch.start_sequence > expected {
                // Gap detected: expected sequence is missing
                gaps.push((expected, batch.start_sequence));
            }

            // Update expected to be one after this batch's end
            expected = batch.end_sequence + 1;
        }

        gaps
    }

    /// Get the latest sequence number covered by the index
    ///
    /// Returns `None` if the index is empty.
    pub fn latest_sequence(&self) -> Option<u64> {
        self.batches.last().map(|b| b.end_sequence)
    }

    /// Get the earliest sequence number covered by the index
    ///
    /// Returns `None` if the index is empty.
    pub fn earliest_sequence(&self) -> Option<u64> {
        self.batches.first().map(|b| b.start_sequence)
    }

    /// Refresh the index by re-listing batches from the fetcher
    ///
    /// Returns the number of new batches discovered.
    pub async fn refresh(&mut self, fetcher: &Arc<dyn StorageFetcher>) -> Result<usize> {
        let old_count = self.batches.len();
        self.batches = fetcher.list_batches().await?;
        let new_count = self.batches.len();

        if new_count > old_count {
            debug!(
                old_count,
                new_count,
                new_batches = new_count - old_count,
                "Refreshed batch index"
            );
        }

        Ok(new_count.saturating_sub(old_count))
    }

    /// Check if a sequence is covered by any batch in the index
    pub fn contains(&self, sequence: u64) -> bool {
        self.find_batch_containing(sequence).is_some()
    }

    /// Get total message count across all batches
    pub fn total_messages(&self) -> u64 {
        self.batches.iter().map(|b| b.len()).sum()
    }
}

/// Iterator state for batch-based sync
///
/// Tracks position within the batch index for sequential iteration.
#[derive(Debug)]
pub struct BatchIterator {
    /// Current batch index position
    batch_idx: usize,
    /// Current message index within the current batch (0-indexed relative to batch)
    message_offset: u64,
}

impl BatchIterator {
    /// Create a new iterator starting at the first batch
    pub const fn new() -> Self {
        Self {
            batch_idx: 0,
            message_offset: 0,
        }
    }

    /// Create an iterator positioned to start at a specific sequence
    ///
    /// Finds the batch containing the sequence and positions within it.
    pub fn starting_at(index: &BatchIndex, sequence: u64) -> Self {
        // Find the batch containing this sequence
        for (idx, batch) in index.batches().iter().enumerate() {
            if batch.contains(sequence) {
                return Self {
                    batch_idx: idx,
                    message_offset: sequence - batch.start_sequence,
                };
            }
            if batch.start_sequence > sequence {
                // Sequence is in a gap before this batch
                // Position at the start of this batch
                return Self {
                    batch_idx: idx,
                    message_offset: 0,
                };
            }
        }

        // Sequence is beyond all batches
        Self {
            batch_idx: index.len(),
            message_offset: 0,
        }
    }

    /// Get the current batch info, if any
    pub fn current_batch<'a>(&self, index: &'a BatchIndex) -> Option<&'a BatchInfo> {
        index.get_batch_at_index(self.batch_idx)
    }

    /// Get the current sequence number being processed
    pub fn current_sequence(&self, index: &BatchIndex) -> Option<u64> {
        self.current_batch(index)
            .map(|b| b.start_sequence + self.message_offset)
    }

    /// Check if we've exhausted all batches in the index
    pub const fn is_exhausted(&self, index: &BatchIndex) -> bool {
        self.batch_idx >= index.len()
    }

    /// Advance to the next message
    ///
    /// Returns `true` if there's another message, `false` if exhausted.
    pub fn advance(&mut self, index: &BatchIndex) -> bool {
        if let Some(batch) = self.current_batch(index) {
            let messages_in_batch = batch.len();
            self.message_offset += 1;

            if self.message_offset >= messages_in_batch {
                // Move to next batch
                self.batch_idx += 1;
                self.message_offset = 0;
            }

            !self.is_exhausted(index)
        } else {
            false
        }
    }

    /// Move to the next batch (skip remaining messages in current batch)
    pub const fn advance_to_next_batch(&mut self) {
        self.batch_idx += 1;
        self.message_offset = 0;
    }

    /// Current batch index
    pub const fn batch_index(&self) -> usize {
        self.batch_idx
    }

    /// Current message offset within batch
    pub const fn message_offset(&self) -> u64 {
        self.message_offset
    }

    /// Reset to the beginning
    pub const fn reset(&mut self) {
        self.batch_idx = 0;
        self.message_offset = 0;
    }
}

impl Default for BatchIterator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_batch_info(start: u64, end: u64) -> BatchInfo {
        BatchInfo::new(start, end, format!("batch_{start}_{end}.json"))
    }

    fn make_index(ranges: &[(u64, u64)]) -> BatchIndex {
        let batches: Vec<BatchInfo> = ranges
            .iter()
            .map(|(start, end)| make_batch_info(*start, *end))
            .collect();
        BatchIndex { batches }
    }

    #[test]
    fn test_empty_index() {
        let index = BatchIndex::empty();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert!(index.find_batch_containing(0).is_none());
        assert!(index.latest_sequence().is_none());
        assert_eq!(index.total_messages(), 0);
    }

    #[test]
    fn test_find_batch_containing() {
        let index = make_index(&[(1, 50), (51, 100), (101, 150)]);

        // Before all batches
        assert!(index.find_batch_containing(0).is_none());

        // In first batch
        assert_eq!(
            index.find_batch_containing(1).map(|b| b.start_sequence),
            Some(1)
        );
        assert_eq!(
            index.find_batch_containing(25).map(|b| b.start_sequence),
            Some(1)
        );
        assert_eq!(
            index.find_batch_containing(50).map(|b| b.start_sequence),
            Some(1)
        );

        // In second batch
        assert_eq!(
            index.find_batch_containing(51).map(|b| b.start_sequence),
            Some(51)
        );
        assert_eq!(
            index.find_batch_containing(75).map(|b| b.start_sequence),
            Some(51)
        );

        // In third batch
        assert_eq!(
            index.find_batch_containing(150).map(|b| b.start_sequence),
            Some(101)
        );

        // After all batches
        assert!(index.find_batch_containing(151).is_none());
    }

    #[test]
    fn test_find_batch_with_gaps() {
        // Batches with gaps: [1-50], [100-150], [200-250]
        let index = make_index(&[(1, 50), (100, 150), (200, 250)]);

        // In first batch
        assert_eq!(
            index.find_batch_containing(25).map(|b| b.start_sequence),
            Some(1)
        );

        // In gap between first and second
        assert!(index.find_batch_containing(75).is_none());

        // In second batch
        assert_eq!(
            index.find_batch_containing(125).map(|b| b.start_sequence),
            Some(100)
        );

        // In gap between second and third
        assert!(index.find_batch_containing(175).is_none());

        // In third batch
        assert_eq!(
            index.find_batch_containing(225).map(|b| b.start_sequence),
            Some(200)
        );
    }

    #[test]
    fn test_find_first_batch_starting_at_or_after() {
        let index = make_index(&[(1, 50), (51, 100), (101, 150)]);

        // Exact match
        assert_eq!(
            index
                .find_first_batch_starting_at_or_after(51)
                .map(|b| b.start_sequence),
            Some(51)
        );

        // Before first batch
        assert_eq!(
            index
                .find_first_batch_starting_at_or_after(0)
                .map(|b| b.start_sequence),
            Some(1)
        );

        // In middle of first batch
        assert_eq!(
            index
                .find_first_batch_starting_at_or_after(25)
                .map(|b| b.start_sequence),
            Some(51)
        );

        // After all batches
        assert!(index.find_first_batch_starting_at_or_after(200).is_none());
    }

    #[test]
    fn test_detect_gaps() {
        // Contiguous batches - no gaps
        let index = make_index(&[(1, 50), (51, 100), (101, 150)]);
        assert_eq!(index.detect_gaps(1), vec![]);

        // Batches with gaps
        let index = make_index(&[(1, 50), (100, 150), (200, 250)]);
        assert_eq!(index.detect_gaps(1), vec![(51, 100), (151, 200)]);

        // Starting after first gap
        let index = make_index(&[(1, 50), (100, 150), (200, 250)]);
        assert_eq!(index.detect_gaps(100), vec![(151, 200)]);

        // Starting from 0 with first batch at 1
        let index = make_index(&[(1, 50)]);
        assert_eq!(index.detect_gaps(0), vec![(0, 1)]);
    }

    #[test]
    fn test_latest_and_earliest_sequence() {
        let index = make_index(&[(10, 50), (51, 100), (101, 150)]);

        assert_eq!(index.earliest_sequence(), Some(10));
        assert_eq!(index.latest_sequence(), Some(150));
    }

    #[test]
    fn test_total_messages() {
        let index = make_index(&[(1, 50), (51, 100)]);
        // First batch: 50 messages, second batch: 50 messages
        assert_eq!(index.total_messages(), 100);

        let index = make_index(&[(1, 1), (2, 2), (3, 3)]);
        assert_eq!(index.total_messages(), 3);
    }

    #[test]
    fn test_contains() {
        let index = make_index(&[(1, 50), (100, 150)]);

        assert!(!index.contains(0));
        assert!(index.contains(1));
        assert!(index.contains(25));
        assert!(index.contains(50));
        assert!(!index.contains(51));
        assert!(!index.contains(99));
        assert!(index.contains(100));
        assert!(index.contains(150));
        assert!(!index.contains(151));
    }

    // BatchIterator tests

    #[test]
    fn test_iterator_new() {
        let iter = BatchIterator::new();
        assert_eq!(iter.batch_index(), 0);
        assert_eq!(iter.message_offset(), 0);
    }

    #[test]
    fn test_iterator_starting_at() {
        let index = make_index(&[(1, 10), (11, 20), (21, 30)]);

        // Start at beginning of first batch
        let iter = BatchIterator::starting_at(&index, 1);
        assert_eq!(iter.batch_index(), 0);
        assert_eq!(iter.message_offset(), 0);

        // Start in middle of first batch
        let iter = BatchIterator::starting_at(&index, 5);
        assert_eq!(iter.batch_index(), 0);
        assert_eq!(iter.message_offset(), 4); // sequence 5 is at offset 4 (0-indexed)

        // Start at beginning of second batch
        let iter = BatchIterator::starting_at(&index, 11);
        assert_eq!(iter.batch_index(), 1);
        assert_eq!(iter.message_offset(), 0);

        // Start in middle of third batch
        let iter = BatchIterator::starting_at(&index, 25);
        assert_eq!(iter.batch_index(), 2);
        assert_eq!(iter.message_offset(), 4);
    }

    #[test]
    fn test_iterator_starting_at_gap() {
        // Batches with gap: [1-10], [21-30]
        let index = make_index(&[(1, 10), (21, 30)]);

        // Start in gap - should position at next batch
        let iter = BatchIterator::starting_at(&index, 15);
        assert_eq!(iter.batch_index(), 1);
        assert_eq!(iter.message_offset(), 0);
    }

    #[test]
    fn test_iterator_advance() {
        let index = make_index(&[(1, 3), (4, 6)]); // 3 messages each

        let mut iter = BatchIterator::new();

        // First batch, first message
        assert_eq!(iter.current_sequence(&index), Some(1));
        assert!(!iter.is_exhausted(&index));

        // Advance through first batch
        assert!(iter.advance(&index));
        assert_eq!(iter.current_sequence(&index), Some(2));

        assert!(iter.advance(&index));
        assert_eq!(iter.current_sequence(&index), Some(3));

        // Move to second batch
        assert!(iter.advance(&index));
        assert_eq!(iter.batch_index(), 1);
        assert_eq!(iter.current_sequence(&index), Some(4));

        // Continue through second batch
        assert!(iter.advance(&index));
        assert_eq!(iter.current_sequence(&index), Some(5));

        assert!(iter.advance(&index));
        assert_eq!(iter.current_sequence(&index), Some(6));

        // Exhaust
        assert!(!iter.advance(&index));
        assert!(iter.is_exhausted(&index));
    }

    #[test]
    fn test_iterator_advance_to_next_batch() {
        let index = make_index(&[(1, 10), (11, 20), (21, 30)]);

        let mut iter = BatchIterator::starting_at(&index, 5);
        assert_eq!(iter.batch_index(), 0);

        iter.advance_to_next_batch();
        assert_eq!(iter.batch_index(), 1);
        assert_eq!(iter.message_offset(), 0);
        assert_eq!(iter.current_sequence(&index), Some(11));
    }

    #[test]
    fn test_iterator_reset() {
        let index = make_index(&[(1, 10)]);

        let mut iter = BatchIterator::starting_at(&index, 5);
        assert_eq!(iter.batch_index(), 0);
        assert_eq!(iter.message_offset(), 4);

        iter.reset();
        assert_eq!(iter.batch_index(), 0);
        assert_eq!(iter.message_offset(), 0);
    }

    #[test]
    fn test_iterator_empty_index() {
        let index = BatchIndex::empty();
        let iter = BatchIterator::new();

        assert!(iter.is_exhausted(&index));
        assert!(iter.current_batch(&index).is_none());
        assert!(iter.current_sequence(&index).is_none());
    }
}
