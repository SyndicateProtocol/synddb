use std::{path::PathBuf, thread, time::Duration};

use clap::Parser;
use synddb_wal_sequencer::{
    storage_layer::{watch_and_sync_to_storage, StorageLayer},
    wal_monitor::monitor_wal,
};
use tracing::info;

fn parse_duration(s: &str) -> Result<Duration, humantime::DurationError> {
    humantime::parse_duration(s)
}

#[derive(Debug, Clone, Parser)]
#[command(name = "synddb-wal-sequencer")]
#[command(about = "WAL sequencer for SQLite replication")]
pub struct Config {
    #[arg(long, env = "DB_PATH")]
    pub db_path: PathBuf,

    #[arg(long, env = "WAL_BACKUPS_DIR")]
    pub wal_backups_dir: PathBuf,

    #[allow(clippy::doc_markdown)]
    /// the storage layer to be used. examples:
    /// - filesystem:/path/to/dir
    #[arg(long, env = "STORAGE_LAYER")]
    pub storage_layer: StorageLayer,

    /// interval between WAL checkpoints (e.g., "1s", "500ms")
    #[arg(long, env = "CHECKPOINT_INTERVAL", default_value = "1s", value_parser = parse_duration)]
    pub checkpoint_interval: Duration,

    /// interval between storage sync checks (e.g., "1s", "500ms")
    #[arg(long, env = "STORAGE_SYNC_INTERVAL", default_value = "1s", value_parser = parse_duration)]
    pub storage_sync_interval: Duration,
}

fn main() {
    tracing_subscriber::fmt::init();
    let config = Box::leak(Box::new(Config::parse()));
    start(config);
}

fn start(config: &'static Config) {
    let wal_monitor_handle = thread::spawn(move || {
        info!("starting WAL monitor");
        monitor_wal(
            &config.db_path,
            &config.wal_backups_dir,
            config.checkpoint_interval,
        );
    });

    let storage_handle = thread::spawn(|| {
        info!("starting Storage service");
        watch_and_sync_to_storage(
            &config.wal_backups_dir,
            &config.storage_layer,
            config.storage_sync_interval,
        );
    });

    wal_monitor_handle.join().expect("monitor thread panicked");
    storage_handle.join().expect("uploader thread panicked");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path, thread, time::Duration};

    use rusqlite::Connection;
    use synddb_shared::utils::tmp_dir;
    use synddb_wal_sequencer::storage_layer::StorageLayer;

    use crate::Config;

    fn apply_wal<P: AsRef<Path>>(wal_path: P, db_path: P) {
        let db_path = db_path.as_ref();
        let wal_dest = db_path.with_extension("db-wal");

        fs::copy(&wal_path, &wal_dest).expect("failed to copy WAL file");

        let conn = Connection::open(db_path).expect("failed to open db");
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")
            .expect("failed to checkpoint");
    }

    #[test]
    fn basic_functionality() {
        let db_dir = tmp_dir("db", None);
        let wal_backups_dir = tmp_dir("db_backup", None);
        let storage_dir = tmp_dir("storage", None);
        let new_db_dir = tmp_dir("new_db", None);

        let db_path = db_dir.join("test.db");
        let new_db_path = new_db_dir.join("test.db");

        // create source DB and write test data
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.pragma_update(None, "journal_mode", "WAL").unwrap();
            conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();

            conn.execute(
                "CREATE TABLE syndicate (id INTEGER PRIMARY KEY, value TEXT)",
                [],
            )
            .unwrap();

            for i in 0..10 {
                conn.execute(
                    "INSERT INTO syndicate (value) VALUES (?1)",
                    [format!("value_{i}")],
                )
                .unwrap();
            }
        }

        // start monitor and storage sync in background threads
        let config = Box::leak(Box::new(Config {
            db_path,
            wal_backups_dir: (&wal_backups_dir).into(),
            storage_layer: StorageLayer::Filesystem((&storage_dir).into()),
            checkpoint_interval: Duration::from_secs(1),
            storage_sync_interval: Duration::from_secs(1),
        }));
        start(config);

        thread::sleep(Duration::from_millis(1100));

        // // wait for WAL files to appear in storage
        // let mut attempts = 0;
        // loop {
        //     let files: Vec<_> = fs::read_dir(&*storage_dir)
        //         .unwrap()
        //         .filter_map(|e| e.ok())
        //         .collect();
        //     if !files.is_empty() {
        //         break;
        //     }
        //     attempts += 1;
        //     if attempts > 50 {
        //         panic!("timeout waiting for WAL files in storage");
        //     }
        //     thread::sleep(Duration::from_millis(100));
        // }
        //
        // collect and sort WAL files from storage
        let mut wal_files: Vec<_> = fs::read_dir(&*storage_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .collect();
        wal_files.sort();

        // apply WAL files to new database
        for wal_file in &wal_files {
            apply_wal(wal_file, &new_db_path);
        }

        // verify data consistency
        let new_conn = Connection::open(&new_db_path).unwrap();
        let mut stmt = new_conn
            .prepare("SELECT id, value FROM syndicate ORDER BY id")
            .unwrap();
        let rows: Vec<(i64, String)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(rows.len(), 10);
        for (i, (id, value)) in rows.iter().enumerate() {
            assert_eq!(*id, (i + 1) as i64);
            assert_eq!(*value, format!("value_{i}"));
        }
    }
}
