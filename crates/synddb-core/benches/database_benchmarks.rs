//! Database performance benchmarks for SyndDB Core
//!
//! Run with: cargo bench --package synddb-core

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use synddb_core::{
    database::{SqliteDatabase, SyndDatabase},
    types::SqlValue,
};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::runtime::Runtime;

// ============================================================================
// Benchmark Setup
// ============================================================================

struct BenchmarkSetup {
    _temp_dir: TempDir,
    database: Arc<SqliteDatabase>,
    runtime: Runtime,
}

impl BenchmarkSetup {
    fn new() -> Self {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("bench.db");
        let database = Arc::new(SqliteDatabase::new(db_path, 16).unwrap());
        let runtime = Runtime::new().unwrap();

        // Initialize test schema
        runtime.block_on(async {
            database
                .execute(
                    r#"
                    CREATE TABLE IF NOT EXISTS test_table (
                        id INTEGER PRIMARY KEY,
                        name TEXT NOT NULL,
                        value INTEGER NOT NULL,
                        data BLOB,
                        created_at INTEGER NOT NULL
                    )
                    "#,
                    vec![],
                )
                .await
                .unwrap();

            database
                .execute(
                    "CREATE INDEX IF NOT EXISTS idx_test_value ON test_table(value)",
                    vec![],
                )
                .await
                .unwrap();
        });

        Self {
            _temp_dir: temp_dir,
            database,
            runtime,
        }
    }
}

// ============================================================================
// Insert Benchmarks
// ============================================================================

fn bench_single_insert(c: &mut Criterion) {
    let setup = BenchmarkSetup::new();

    c.bench_function("single_insert", |b| {
        b.to_async(&setup.runtime).iter(|| async {
            let db = setup.database.clone();
            db.execute(
                "INSERT INTO test_table (name, value, created_at) VALUES (?1, ?2, ?3)",
                vec![
                    SqlValue::Text("test".to_string()),
                    SqlValue::Integer(42),
                    SqlValue::Integer(1000000),
                ],
            )
            .await
            .unwrap();
        });
    });
}

fn bench_batch_insert(c: &mut Criterion) {
    let setup = BenchmarkSetup::new();
    let mut group = c.benchmark_group("batch_insert");

    for batch_size in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*batch_size as u64));
        group.bench_with_input(
            format!("batch_{}", batch_size),
            batch_size,
            |b, &size| {
                b.to_async(&setup.runtime).iter(|| async {
                    let db = setup.database.clone();

                    // Build batch of operations
                    let operations: Vec<_> = (0..size)
                        .map(|i| synddb_core::types::SqlOperation {
                            sql: "INSERT INTO test_table (name, value, created_at) VALUES (?1, ?2, ?3)"
                                .to_string(),
                            params: vec![
                                SqlValue::Text(format!("test_{}", i)),
                                SqlValue::Integer(i as i64),
                                SqlValue::Integer(1000000 + i as i64),
                            ],
                        })
                        .collect();

                    db.execute_batch(operations).await.unwrap();
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// Query Benchmarks
// ============================================================================

fn bench_simple_query(c: &mut Criterion) {
    let setup = BenchmarkSetup::new();

    // Pre-populate with data
    setup.runtime.block_on(async {
        for i in 0..1000 {
            setup
                .database
                .execute(
                    "INSERT INTO test_table (name, value, created_at) VALUES (?1, ?2, ?3)",
                    vec![
                        SqlValue::Text(format!("test_{}", i)),
                        SqlValue::Integer(i),
                        SqlValue::Integer(1000000 + i),
                    ],
                )
                .await
                .unwrap();
        }
    });

    c.bench_function("query_by_primary_key", |b| {
        b.to_async(&setup.runtime).iter(|| async {
            let db = setup.database.clone();
            let result = db
                .query(
                    "SELECT * FROM test_table WHERE id = ?1",
                    vec![SqlValue::Integer(black_box(500))],
                )
                .await
                .unwrap();

            black_box(result);
        });
    });
}

fn bench_indexed_query(c: &mut Criterion) {
    let setup = BenchmarkSetup::new();

    // Pre-populate with data
    setup.runtime.block_on(async {
        for i in 0..1000 {
            setup
                .database
                .execute(
                    "INSERT INTO test_table (name, value, created_at) VALUES (?1, ?2, ?3)",
                    vec![
                        SqlValue::Text(format!("test_{}", i)),
                        SqlValue::Integer(i),
                        SqlValue::Integer(1000000 + i),
                    ],
                )
                .await
                .unwrap();
        }
    });

    c.bench_function("query_by_indexed_column", |b| {
        b.to_async(&setup.runtime).iter(|| async {
            let db = setup.database.clone();
            let result = db
                .query(
                    "SELECT * FROM test_table WHERE value = ?1",
                    vec![SqlValue::Integer(black_box(500))],
                )
                .await
                .unwrap();

            black_box(result);
        });
    });
}

fn bench_range_query(c: &mut Criterion) {
    let setup = BenchmarkSetup::new();

    // Pre-populate with data
    setup.runtime.block_on(async {
        for i in 0..1000 {
            setup
                .database
                .execute(
                    "INSERT INTO test_table (name, value, created_at) VALUES (?1, ?2, ?3)",
                    vec![
                        SqlValue::Text(format!("test_{}", i)),
                        SqlValue::Integer(i),
                        SqlValue::Integer(1000000 + i),
                    ],
                )
                .await
                .unwrap();
        }
    });

    c.bench_function("query_range_100_rows", |b| {
        b.to_async(&setup.runtime).iter(|| async {
            let db = setup.database.clone();
            let result = db
                .query(
                    "SELECT * FROM test_table WHERE value BETWEEN ?1 AND ?2",
                    vec![SqlValue::Integer(400), SqlValue::Integer(500)],
                )
                .await
                .unwrap();

            black_box(result);
        });
    });
}

// ============================================================================
// Update Benchmarks
// ============================================================================

fn bench_simple_update(c: &mut Criterion) {
    let setup = BenchmarkSetup::new();

    // Pre-populate with data
    setup.runtime.block_on(async {
        for i in 0..1000 {
            setup
                .database
                .execute(
                    "INSERT INTO test_table (name, value, created_at) VALUES (?1, ?2, ?3)",
                    vec![
                        SqlValue::Text(format!("test_{}", i)),
                        SqlValue::Integer(i),
                        SqlValue::Integer(1000000 + i),
                    ],
                )
                .await
                .unwrap();
        }
    });

    c.bench_function("single_update", |b| {
        b.to_async(&setup.runtime).iter(|| async {
            let db = setup.database.clone();
            db.execute(
                "UPDATE test_table SET value = ?1 WHERE id = ?2",
                vec![
                    SqlValue::Integer(black_box(999)),
                    SqlValue::Integer(black_box(500)),
                ],
            )
            .await
            .unwrap();
        });
    });
}

// ============================================================================
// Transaction Benchmarks
// ============================================================================

fn bench_transaction_throughput(c: &mut Criterion) {
    let setup = BenchmarkSetup::new();
    let mut group = c.benchmark_group("transaction_throughput");

    group.throughput(Throughput::Elements(1000));
    group.bench_function("1000_inserts_in_transaction", |b| {
        b.to_async(&setup.runtime).iter(|| async {
            let db = setup.database.clone();

            let operations: Vec<_> = (0..1000)
                .map(|i| synddb_core::types::SqlOperation {
                    sql: "INSERT INTO test_table (name, value, created_at) VALUES (?1, ?2, ?3)"
                        .to_string(),
                    params: vec![
                        SqlValue::Text(format!("test_{}", i)),
                        SqlValue::Integer(i as i64),
                        SqlValue::Integer(1000000 + i as i64),
                    ],
                })
                .collect();

            db.execute_batch(operations).await.unwrap();
        });
    });

    group.finish();
}

// ============================================================================
// Mixed Workload Benchmarks
// ============================================================================

fn bench_mixed_workload(c: &mut Criterion) {
    let setup = BenchmarkSetup::new();

    // Pre-populate with data
    setup.runtime.block_on(async {
        for i in 0..1000 {
            setup
                .database
                .execute(
                    "INSERT INTO test_table (name, value, created_at) VALUES (?1, ?2, ?3)",
                    vec![
                        SqlValue::Text(format!("test_{}", i)),
                        SqlValue::Integer(i),
                        SqlValue::Integer(1000000 + i),
                    ],
                )
                .await
                .unwrap();
        }
    });

    use std::sync::atomic::{AtomicU64, Ordering};
    let counter = Arc::new(AtomicU64::new(0));

    c.bench_function("mixed_read_write", |b| {
        let counter = counter.clone();
        b.to_async(&setup.runtime).iter(|| {
            let db = setup.database.clone();
            let counter = counter.clone();

            async move {
                let count = counter.fetch_add(1, Ordering::Relaxed);

                // 70% reads, 30% writes
                if count % 10 < 7 {
                    // Read
                    let result = db
                        .query(
                            "SELECT * FROM test_table WHERE value = ?1",
                            vec![SqlValue::Integer((count % 1000) as i64)],
                        )
                        .await
                        .unwrap();
                    black_box(result);
                } else {
                    // Write
                    db.execute(
                        "INSERT INTO test_table (name, value, created_at) VALUES (?1, ?2, ?3)",
                        vec![
                            SqlValue::Text(format!("test_{}", count)),
                            SqlValue::Integer(count as i64),
                            SqlValue::Integer(1000000 + count as i64),
                        ],
                    )
                    .await
                    .unwrap();
                }
            }
        });
    });
}

// ============================================================================
// Criterion Configuration
// ============================================================================

criterion_group!(
    benches,
    bench_single_insert,
    bench_batch_insert,
    bench_simple_query,
    bench_indexed_query,
    bench_range_query,
    bench_simple_update,
    bench_transaction_throughput,
    bench_mixed_workload,
);

criterion_main!(benches);
