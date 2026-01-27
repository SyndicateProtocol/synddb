//! Property-based tests for E2E scenarios
//!
//! These tests use proptest to generate random scenarios and verify
//! that invariants hold across the `SyndDB` pipeline.

#[cfg(test)]
use crate::{
    harness::E2EHarness,
    invariants::{
        check_all_invariants, ConsistencyInvariants, InvariantChecker, SequenceInvariants,
        SignatureInvariants,
    },
    scenarios::{
        execute_scenario, high_volume_scenario_strategy, multi_sync_scenario_strategy,
        simple_scenario_strategy,
    },
};

#[cfg(test)]
use proptest::prelude::*;

#[cfg(test)]
proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50),
        max_shrink_iters: 100,
        ..ProptestConfig::default()
    })]

    /// Test that simple scenarios maintain state consistency after sync
    #[test]
    fn prop_simple_scenario_consistency(scenario in simple_scenario_strategy()) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");

        let mut harness = E2EHarness::new().expect("Failed to create harness");

        // Execute scenario
        execute_scenario(&mut harness, &scenario).expect("Failed to execute scenario");

        // Final sync
        rt.block_on(harness.sync_validator()).expect("Failed to sync");

        // Check consistency
        let violations = ConsistencyInvariants.check(&harness);
        prop_assert!(violations.is_empty(), "Consistency violations: {:?}", violations);
    }

    /// Test that sequence invariants hold
    #[test]
    fn prop_sequence_invariants(scenario in simple_scenario_strategy()) {
        let mut harness = E2EHarness::new().expect("Failed to create harness");

        execute_scenario(&mut harness, &scenario).expect("Failed to execute scenario");

        let violations = SequenceInvariants.check(&harness);
        prop_assert!(violations.is_empty(), "Sequence violations: {:?}", violations);
    }

    /// Test that signature invariants hold
    #[test]
    fn prop_signature_invariants(scenario in simple_scenario_strategy()) {
        let mut harness = E2EHarness::new().expect("Failed to create harness");

        execute_scenario(&mut harness, &scenario).expect("Failed to execute scenario");

        let violations = SignatureInvariants.check(&harness);
        prop_assert!(violations.is_empty(), "Signature violations: {:?}", violations);
    }

    /// Test multi-sync scenarios maintain consistency at each sync point
    #[test]
    fn prop_multi_sync_consistency(scenario in multi_sync_scenario_strategy()) {
        let mut harness = E2EHarness::new().expect("Failed to create harness");

        execute_scenario(&mut harness, &scenario).expect("Failed to execute scenario");

        // After scenario execution (which includes syncs), check consistency
        let violations = ConsistencyInvariants.check(&harness);
        prop_assert!(violations.is_empty(), "Consistency violations: {:?}", violations);
    }

    /// Test all invariants together
    #[test]
    fn prop_all_invariants(scenario in simple_scenario_strategy()) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");

        let mut harness = E2EHarness::new().expect("Failed to create harness");

        execute_scenario(&mut harness, &scenario).expect("Failed to execute scenario");

        // Final sync to ensure consistency
        rt.block_on(harness.sync_validator()).expect("Failed to sync");

        let violations = check_all_invariants(&harness);
        prop_assert!(violations.is_empty(), "Invariant violations: {:?}", violations);
    }
}

// High-volume tests are expensive, so we run fewer cases
#[cfg(test)]
proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|s| s.parse().ok())
            .map_or(10, |c: u32| c / 5), // Run 1/5 as many high-volume tests
        max_shrink_iters: 50,
        ..ProptestConfig::default()
    })]

    /// Test high-volume scenarios
    #[test]
    fn prop_high_volume_consistency(scenario in high_volume_scenario_strategy()) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create runtime");

        let mut harness = E2EHarness::new().expect("Failed to create harness");

        execute_scenario(&mut harness, &scenario).expect("Failed to execute scenario");

        // Sync all messages
        rt.block_on(harness.sync_validator()).expect("Failed to sync");

        // Check consistency
        let violations = ConsistencyInvariants.check(&harness);
        prop_assert!(violations.is_empty(), "Consistency violations: {:?}", violations);
    }
}

// Stress tests - expensive, run with: cargo test -- --ignored
#[cfg(test)]
proptest! {
    #![proptest_config(ProptestConfig {
        cases: std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|s| s.parse().ok())
            .map_or(3, |c: u32| c / 30), // Very few stress tests
        max_shrink_iters: 10,
        ..ProptestConfig::default()
    })]

    /// Stress test with 200-500 operations
    #[test]
    #[ignore] // Enable with: cargo test -- --ignored
    fn prop_stress_scenario(scenario in crate::scenarios::stress_scenario_strategy()) {
        let mut harness = E2EHarness::new().expect("Failed to create harness");

        execute_scenario(&mut harness, &scenario).expect("Failed to execute scenario");

        let violations = check_all_invariants(&harness);
        prop_assert!(violations.is_empty(), "Stress test violations: {:?}", violations);
    }

    /// Test with large TEXT payloads
    #[test]
    #[ignore]
    fn prop_large_payload_scenario(scenario in crate::scenarios::large_payload_scenario_strategy()) {
        let mut harness = E2EHarness::new().expect("Failed to create harness");

        execute_scenario(&mut harness, &scenario).expect("Failed to execute scenario");

        let violations = ConsistencyInvariants.check(&harness);
        prop_assert!(violations.is_empty(), "Large payload violations: {:?}", violations);
    }

    /// Test rapid insert/delete churn
    #[test]
    #[ignore]
    fn prop_churn_scenario(scenario in crate::scenarios::churn_scenario_strategy()) {
        let mut harness = E2EHarness::new().expect("Failed to create harness");

        execute_scenario(&mut harness, &scenario).expect("Failed to execute scenario");

        let violations = check_all_invariants(&harness);
        prop_assert!(violations.is_empty(), "Churn scenario violations: {:?}", violations);
    }
}

#[cfg(test)]
mod targeted_tests {
    use super::*;
    use crate::faults::{inject_fault, tamper_signature, Fault};
    use synddb_validator::sync::fetcher::StorageFetcher;

    /// Test that signature tampering is detected
    #[tokio::test]
    async fn test_signature_tampering_detected() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")
            .expect("Failed to setup schema");

        harness
            .source()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();
        harness
            .replica()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();

        // Sequence a message
        harness
            .execute_and_sequence("UPDATE t SET v = 1 WHERE id = 1")
            .unwrap();

        // Get the message and tamper with its signature
        let mut message = harness.fetcher().get(0).await.unwrap().unwrap();
        message.signature = tamper_signature(&message.signature);

        // Clear the fetcher and add tampered message
        harness.fetcher().clear();
        harness.fetcher().add_message(message);

        // Sync should fail due to signature verification
        let result = harness.sync_validator().await;

        // The sync should either fail or not sync any messages
        // (depending on how the validator handles invalid signatures)
        match result {
            Ok(synced) => {
                // If it "succeeded", it should have synced 0 messages
                // because the signature is invalid
                assert_eq!(synced, 0, "Should not sync message with invalid signature");
            }
            Err(e) => {
                // Expected: signature verification failed
                let err_str = e.to_string();
                assert!(
                    err_str.contains("signature")
                        || err_str.contains("Signer")
                        || err_str.contains("verify"),
                    "Expected signature error, got: {}",
                    err_str
                );
            }
        }
    }

    /// Test that fetch failures are handled
    #[tokio::test]
    async fn test_fetch_failure_handling() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")
            .expect("Failed to setup schema");

        harness
            .source()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();
        harness
            .replica()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();

        // Sequence a message
        harness
            .execute_and_sequence("UPDATE t SET v = 1 WHERE id = 1")
            .unwrap();

        // Inject a fetch failure
        inject_fault(
            &harness,
            &Fault::MessageFetchFailure {
                sequence: 0,
                error_message: "network timeout".to_string(),
            },
        );

        // Sync should fail or handle gracefully
        let _result = harness.sync_validator().await;

        // Clear the failure and retry
        inject_fault(&harness, &Fault::ClearFailures);
        let synced = harness
            .sync_validator()
            .await
            .expect("Should sync after clearing failure");
        assert_eq!(synced, 1);
    }

    /// Test multiple operations without sync still maintains sequencing
    #[tokio::test]
    async fn test_multiple_operations_sequencing() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")
            .expect("Failed to setup schema");

        harness
            .source()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();
        harness
            .replica()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();

        // Execute many operations
        for i in 1..=10 {
            harness
                .execute_and_sequence(&format!("UPDATE t SET v = {} WHERE id = 1", i))
                .unwrap();
        }

        // Check sequence invariants
        let violations = SequenceInvariants.check(&harness);
        assert!(
            violations.is_empty(),
            "Sequence violations: {:?}",
            violations
        );

        // Sync all
        let synced = harness.sync_validator().await.expect("Failed to sync");
        assert_eq!(synced, 10);

        // Check final value
        let value: i64 = harness
            .replica()
            .query_row("SELECT v FROM t WHERE id = 1", [], |row| row.get(0))
            .expect("Failed to query");
        assert_eq!(value, 10);
    }

    /// Test empty changeset handling
    #[tokio::test]
    async fn test_empty_changeset() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")
            .expect("Failed to setup schema");

        harness
            .source()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();
        harness
            .replica()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();

        // Try to update with same value - should produce empty changeset
        let result = harness.execute_and_sequence("UPDATE t SET v = 0 WHERE id = 1");

        // Empty changesets may be rejected - this is expected behavior
        // The important thing is it doesn't crash
        match result {
            Ok(_) => {
                // If it succeeded, we should be able to sync
                harness.sync_validator().await.expect("Failed to sync");
            }
            Err(e) => {
                // Empty changeset error is acceptable
                assert!(
                    e.to_string().contains("empty") || e.to_string().contains("no changes"),
                    "Unexpected error: {}",
                    e
                );
            }
        }
    }

    /// Test insert, update, delete cycle
    #[tokio::test]
    async fn test_insert_update_delete_cycle() {
        let mut harness = E2EHarness::new().expect("Failed to create harness");
        harness
            .setup_schema("CREATE TABLE t (id INTEGER PRIMARY KEY, v INTEGER)")
            .expect("Failed to setup schema");

        // Initial sync point
        harness
            .source()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();
        harness
            .replica()
            .execute("INSERT INTO t VALUES (1, 0)", [])
            .unwrap();

        // Insert new row
        harness
            .execute_and_sequence("INSERT INTO t VALUES (2, 100)")
            .unwrap();
        harness.sync_validator().await.unwrap();
        harness
            .verify_consistency()
            .expect("Should be consistent after insert");

        // Update the row
        harness
            .execute_and_sequence("UPDATE t SET v = 200 WHERE id = 2")
            .unwrap();
        harness.sync_validator().await.unwrap();
        harness
            .verify_consistency()
            .expect("Should be consistent after update");

        // Delete the row
        harness
            .execute_and_sequence("DELETE FROM t WHERE id = 2")
            .unwrap();
        harness.sync_validator().await.unwrap();
        harness
            .verify_consistency()
            .expect("Should be consistent after delete");

        // Verify row is gone
        let count: i64 = harness
            .replica()
            .query_row("SELECT COUNT(*) FROM t WHERE id = 2", [], |row| row.get(0))
            .expect("Failed to query");
        assert_eq!(count, 0);
    }
}
