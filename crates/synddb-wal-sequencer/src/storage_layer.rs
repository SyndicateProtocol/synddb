use std::{
    fs::{self, DirEntry},
    io::Error,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use tracing::trace;

#[derive(Debug, Clone)]
pub enum StorageLayer {
    Noop,
    Filesystem(PathBuf),
}

impl std::str::FromStr for StorageLayer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "noop" {
            Ok(Self::Noop)
        } else if let Some(path) = s.strip_prefix("filesystem:") {
            Ok(Self::Filesystem(PathBuf::from(path)))
        } else {
            Err(format!(
                "invalid storage layer: {s}. Expected 'noop' or 'filesystem:/path'"
            ))
        }
    }
}

/// watches a wal `backups_dir` and uploads files to a `storage_layer`
pub fn watch_and_sync_to_storage<P: AsRef<Path>>(
    backups_dir: P,
    storage_layer: &StorageLayer,
    sync_interval: Duration,
) {
    match storage_layer {
        StorageLayer::Filesystem(target_dir) => {
            do_watch_and_sync(backups_dir, FsStorage::new(target_dir), sync_interval);
        }
        StorageLayer::Noop => {
            do_watch_and_sync(backups_dir, NoopStorage {}, sync_interval);
        }
    };
}

fn do_watch_and_sync<P, S>(dir: P, storage: S, sync_interval: Duration)
where
    P: AsRef<Path>,
    S: StorageAdapter,
{
    trace!(dir = ?dir.as_ref(), "starting watch_and_sync");
    loop {
        let wal_files = wal_files_in_dir(&dir)
            .unwrap_or_else(|e| panic!("failed to obtain wal_files in dir: {e}"));

        for wal in wal_files {
            let filename = wal.file_name().to_string_lossy().into_owned();
            if !storage.has_file(&filename) {
                // TODO this is a naive implementation that reads the entire file to RAM ,can be
                // improved
                let start = Instant::now();
                let contents = fs::read(wal.path())
                    .unwrap_or_else(|e| panic!("failed to read file {filename}: {e}"));
                storage
                    .upload(&filename, &contents)
                    .unwrap_or_else(|e| panic!("failed to upload file {filename}: {e}"));
                trace!(%filename, "uploaded, took {} ns", start.elapsed().as_nanos());
            }
            fs::remove_file(wal.path())
                .unwrap_or_else(|e| panic!("failed to remove file: {wal:?}, error: {e}"));
        }

        std::thread::sleep(sync_interval);
    }
}

/// returns the list of files in the dir sorted alphabetically
fn wal_files_in_dir<P: AsRef<Path>>(dir: P) -> Result<Vec<DirEntry>, Error> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.path());
    Ok(entries)
}

pub trait StorageAdapter {
    fn has_file(&self, filename: &str) -> bool;
    fn upload(&self, filename: &str, contents: &[u8]) -> Result<(), String>;
}

// ------------------- Storage Adapter implementations

/// TODO remove
struct NoopStorage {}
impl StorageAdapter for NoopStorage {
    fn has_file(&self, _filename: &str) -> bool {
        false
    }

    fn upload(&self, _filename: &str, _contents: &[u8]) -> Result<(), String> {
        // noop
        Ok(())
    }
}

/// Simple filesystem storage (moves files to another dir), useful for tests
#[derive(Debug)]
pub struct FsStorage {
    target_dir: PathBuf,
}

impl FsStorage {
    pub fn new<P: AsRef<Path>>(target_dir: P) -> Self {
        let target_dir = target_dir.as_ref().to_path_buf();
        fs::create_dir_all(&target_dir)
            .unwrap_or_else(|e| panic!("unable to create dir {target_dir:?}: {e}"));
        Self { target_dir }
    }
}

impl StorageAdapter for FsStorage {
    fn has_file(&self, filename: &str) -> bool {
        self.target_dir.join(filename).exists()
    }

    fn upload(&self, filename: &str, contents: &[u8]) -> Result<(), String> {
        let path = self.target_dir.join(filename);
        fs::write(&path, contents).map_err(|e| format!("failed to write {}: {e}", path.display()))
    }
}
