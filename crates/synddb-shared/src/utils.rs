use alloy::primitives::keccak256;
use std::{
    fs::{self},
    ops::Deref,
    panic,
    path::{Path, PathBuf},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
pub struct TmpDir(PathBuf);

impl Deref for TmpDir {
    type Target = Path;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

impl From<&TmpDir> for PathBuf {
    fn from(tmp: &TmpDir) -> Self {
        tmp.0.clone()
    }
}

/// Returns a unique temporary path for tests.
///
/// The path is constructed by:
/// 1. Getting the caller's source location (file and line)
/// 2. Appending the current timestamp in nanoseconds, process ID, and thread ID
/// 3. Hashing the combined string
/// 4. Creating a path in the system temp directory with format `"{prefix}_{hash}"`
///
/// This ensures unique paths for concurrent tests by including both the test location,
/// process ID, and thread ID for debugging.
pub fn tmp_dir(prefix: &str, dir: Option<PathBuf>) -> TmpDir {
    let location = panic::Location::caller();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let thread_id = thread::current().id();
    let process_id = std::process::id();

    let input = format!("{location}:{timestamp}:{process_id}:{thread_id:?}");
    let hash = keccak256(input.as_bytes());
    let hash_hex = alloy::hex::encode(hash);

    let dir = dir
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("{prefix}_{hash_hex}"));
    fs::create_dir_all(&dir).unwrap();
    TmpDir(dir)
}
