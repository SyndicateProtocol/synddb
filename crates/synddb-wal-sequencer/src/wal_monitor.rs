use core::panic;
use std::{
    fs,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use rusqlite::{Connection, Transaction};
use tracing::trace;

pub const JOURNAL_MODE: &str = "journal_mode";
pub const WAL_AUTOCHECKPOINT: &str = "wal_autocheckpoint";
pub const WAL_CHECKPOINT: &str = "wal_checkpoint";
pub const TRUNCATE: &str = "TRUNCATE";
pub const WAL: &str = "WAL";

pub fn monitor_wal<P: AsRef<Path>>(db_path: P, wal_backups_dir: P, checkpoint_interval: Duration) {
    let wal_path = db_path.as_ref().with_extension("db-wal");
    // reconcile storage layer state with DB / WAL /backup files

    // assert correct DB configuration is in place:
    // jornal_mode = WAL
    // wal_checkpoint = 0
    let mut conn = if db_path.as_ref().exists() {
        // existing DB
        let conn = Connection::open(db_path).unwrap();
        let journal_mode: String = conn
            .pragma_query_value(None, JOURNAL_MODE, |row| row.get(0))
            .unwrap();
        assert_eq!(
            journal_mode.to_uppercase(),
            WAL,
            "Database must be in WAL mode"
        );
        conn
    } else {
        // new DB
        let conn = Connection::open(db_path).unwrap();
        conn.pragma_update(None, JOURNAL_MODE, WAL).unwrap();
        conn
    };

    // wal_checkpoint is per-connection
    conn.pragma_update(None, WAL_AUTOCHECKPOINT, 0).unwrap();

    trace!(?wal_path, "monitor_wal started");

    // NOTE: this read lock is necessary to prevent other applications from checkpointing
    #[allow(clippy::collection_is_never_read)]
    let mut _read_lock = acquire_read_lock(&mut conn);

    loop {
        std::thread::sleep(checkpoint_interval);

        let wal_size = fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0);
        if wal_size == 0 {
            trace!("WAL is empty.");
            continue;
        }
        trace!(wal_size, "WAL contents found.");

        _read_lock = None; // rollbacks the read_tx and releases the lock

        // NOTE: to avoid changes while the WAL data is collected, we temporarily lock db access with this tx (notice the locking mode)
        // https://www.sqlite.org/lockingv3.html#locking
        // this is how litestream does it for reference: https://github.com/benbjohnson/litestream/blob/e1d5aad75bc67735732b54a252d98685c502288b/db.go#L544
        // TODO I think Immediate is enough here, but maybe Exclusive is warranted?
        // TODO make some tests that assert this actually properly locks the DB as expected
        let lock_tx = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();

        // TODO using timestamp for simplicity, but a sequence number would  be better (should come
        // from the storage layer)

        let start = Instant::now();
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let wal_backup = wal_backups_dir.as_ref().join(format!("{ts}.db-wal"));
        fs::copy(&wal_path, &wal_backup)
            .unwrap_or_else(|e| panic!("failed to backup WAL file: {e}"));

        trace!(
            ?wal_backup,
            "WAL backup complete, took {} ns",
            start.elapsed().as_nanos()
        );

        // release the lock
        lock_tx.rollback().expect("failed to rollback lock_tx");

        // TODO address this:
        // NOTE: there is a race condition where the application could write to the db
        // between the release of the read lock and the WAL_CHECKPOINT being applied
        // litestream protects against this by allowing the inconsistent state to occur, then
        // applying a snapshot to catch up: https://github.com/benbjohnson/litestream/blob/92fc139923d2b13909ba8b0e5df8b63d45a91648/db.go#L940-L942

        // checkpoint
        // https://www.sqlite.org/pragma.html#pragma_wal_checkpoint
        conn.pragma_update(None, WAL_CHECKPOINT, TRUNCATE)
            .unwrap_or_else(|e| panic!("failed to initiate WAL checkpoint: {e}"));

        // re-aquire the read-lock immediately
        _read_lock = acquire_read_lock(&mut conn);
    }
}

fn acquire_read_lock(conn: &mut Connection) -> Option<Transaction<'_>> {
    let read_lock = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .unwrap();
    let _: i32 = read_lock
        .query_row("SELECT COUNT(1) FROM sqlite_schema", [], |row| row.get(0))
        .unwrap_or_else(|e| panic!("failed to get read_lock tx: {e}"));
    Some(read_lock)
}

#[cfg(test)]
mod tests {
    // use super::*;

    #[test]
    fn todo_test() {}
}
