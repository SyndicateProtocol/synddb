use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};
use thiserror::Error;
use zerocopy::{byteorder::Order, FromBytes, Immutable, KnownLayout, BE, LE, U32};

#[derive(Debug, Error)]
pub enum WalError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),
    #[error("invalid WAL header bytes: {0}")]
    InvalidHeaderBytes(String),
    #[error("invalid WAL frame header bytes: {0}")]
    InvalidFrameHeaderBytes(String),
}

// https://www.sqlite.org/fileformat.html#the_write_ahead_log
#[derive(Debug)]
pub struct WalState {
    header: WalHeader,
    frames: Vec<WalFrame>,
}

const WAL_HEADER_SIZE: usize = 32;
const WAL_FRAME_HEADER_SIZE: usize = 24;
const FILE_FORMAT_VERSION: u32 = 3007000;

pub fn read_wal_file<P: AsRef<Path>>(path: P) -> Result<WalState, WalError> {
    let mut file = File::open(path)?;

    let mut header_bytes = [0u8; WAL_HEADER_SIZE];
    file.read_exact(&mut header_bytes)?;

    let header = WalHeader::read_from_bytes(&header_bytes)
        .map_err(|e| WalError::InvalidHeaderBytes(e.to_string()))?;

    let checksum_byte_order = header.byte_order();

    let (mut checksum_1, mut checksum_2) =
        wal_checksum(&checksum_byte_order, 0, 0, &header_bytes[..24]);

    assert_eq!(
        checksum_1,
        header.checksum_1.get(),
        "WAL header checksum 1 mismatch"
    );
    assert_eq!(
        checksum_2,
        header.checksum_2.get(),
        "WAL header checksum 2 mismatch"
    );

    assert_eq!(
        header.file_format_version, FILE_FORMAT_VERSION,
        "invalid file format version on WAL header"
    );

    let page_size: usize = header.page_size.get().try_into().unwrap();
    let mut frames = vec![];
    loop {
        let mut frame_header_bytes = [0u8; WAL_FRAME_HEADER_SIZE];
        match file.read_exact(&mut frame_header_bytes) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(e.into()),
        }
        let frame_header = WalFrameHeader::read_from_bytes(&frame_header_bytes)
            .map_err(|e| WalError::InvalidFrameHeaderBytes(e.to_string()))?;

        let mut data = vec![0u8; page_size];
        file.read_exact(&mut data)?;

        (checksum_1, checksum_2) = wal_checksum(
            &checksum_byte_order,
            checksum_1,
            checksum_2,
            &frame_header_bytes[..8],
        );
        (checksum_1, checksum_2) =
            wal_checksum(&checksum_byte_order, checksum_1, checksum_2, &data);
        assert_eq!(
            checksum_1,
            frame_header.checksum_1.get(),
            "WAL frame checksum 1 mismatch"
        );
        assert_eq!(
            checksum_2,
            frame_header.checksum_2.get(),
            "WAL frame checksum 2 mismatch"
        );

        frames.push(WalFrame {
            header: frame_header,
            data,
        });
    }

    Ok(WalState { header, frames })
}

fn wal_checksum(byte_order: &Order, mut s0: u32, mut s1: u32, data: &[u8]) -> (u32, u32) {
    assert!(data.len().is_multiple_of(8), "checksum bad data length");

    for chunk in data.chunks_exact(8) {
        let (d0, d1) = match byte_order {
            Order::BigEndian => {
                let vals = <[U32<BE>; 2]>::read_from_bytes(chunk).unwrap();
                (vals[0].get(), vals[1].get())
            }
            Order::LittleEndian => {
                let vals = <[U32<LE>; 2]>::read_from_bytes(chunk).unwrap();
                (vals[0].get(), vals[1].get())
            }
        };
        s0 = s0.wrapping_add(d0).wrapping_add(s1);
        s1 = s1.wrapping_add(d1).wrapping_add(s0);
    }

    (s0, s1)
}

#[derive(Debug, FromBytes, Immutable, KnownLayout)]
#[repr(C)]
struct WalHeader {
    magic_number: U32<BE>,
    file_format_version: U32<BE>,
    page_size: U32<BE>,
    checkpoint_seq: U32<BE>,
    salt_1: U32<BE>,
    salt_2: U32<BE>,
    checksum_1: U32<BE>,
    checksum_2: U32<BE>,
}

impl WalHeader {
    fn byte_order(&self) -> Order {
        match self.magic_number.get() {
            0x377f0682 => Order::LittleEndian,
            0x377f0683 => Order::BigEndian,
            magic => panic!("invalid WAL magic number: {:#010x}", magic),
        }
    }
}

#[derive(Debug, FromBytes, Immutable, KnownLayout)]
#[repr(C)]
struct WalFrameHeader {
    page_number: U32<BE>,
    /// expressed in pages after commit (0 for non-commit frames)
    db_size_after_commit: U32<BE>,
    salt_1: U32<BE>,
    salt_2: U32<BE>,
    checksum_1: U32<BE>,
    checksum_2: U32<BE>,
}

#[derive(Debug)]
struct WalFrame {
    header: WalFrameHeader,
    data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::{
        ops::Deref,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Self {
            let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
            let dir = std::env::temp_dir().join("synddb-wal-tests").join(format!(
                "{}-{}-{}",
                name,
                std::process::id(),
                counter
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Deref for TestDir {
        type Target = Path;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn test_wal_checksum_little_endian() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let (s0, s1) = wal_checksum(&Order::LittleEndian, 0, 0, &data);

        // d0 = 0x04030201, d1 = 0x08070605
        // s0 = 0 + 0x04030201 + 0 = 0x04030201
        // s1 = 0 + 0x08070605 + 0x04030201 = 0x0c0a0806
        assert_eq!(s0, 0x04030201);
        assert_eq!(s1, 0x0c0a0806);
    }

    #[test]
    fn test_wal_checksum_big_endian() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let (s0, s1) = wal_checksum(&Order::BigEndian, 0, 0, &data);

        // d0 = 0x01020304, d1 = 0x05060708
        // s0 = 0 + 0x01020304 + 0 = 0x01020304
        // s1 = 0 + 0x05060708 + 0x01020304 = 0x06080a0c
        assert_eq!(s0, 0x01020304);
        assert_eq!(s1, 0x06080a0c);
    }

    #[test]
    fn test_wal_checksum_chained() {
        let data1 = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let data2 = [0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80];

        let (s0, s1) = wal_checksum(&Order::LittleEndian, 0, 0, &data1);
        let (s0, s1) = wal_checksum(&Order::LittleEndian, s0, s1, &data2);

        // Verify chaining works by computing the same thing in one call
        let mut combined = [0u8; 16];
        combined[..8].copy_from_slice(&data1);
        combined[8..].copy_from_slice(&data2);
        let (expected_s0, expected_s1) = wal_checksum(&Order::LittleEndian, 0, 0, &combined);

        assert_eq!(s0, expected_s0);
        assert_eq!(s1, expected_s1);
    }

    #[test]
    #[should_panic(expected = "checksum bad data length")]
    fn test_wal_checksum_bad_length() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05];
        wal_checksum(&Order::LittleEndian, 0, 0, &data);
    }

    #[test]
    fn test_read_wal_file_with_sqlite() {
        let dir = TestDir::new("read_wal_file_with_sqlite");
        let db_path = dir.join("test.db");
        let wal_path = dir.join("test.db-wal");

        let conn = Connection::open(&db_path).unwrap();

        conn.pragma_update(None, "journal_mode", "WAL").unwrap();

        conn.execute("CREATE TABLE test (id INTEGER PRIMARY KEY, value TEXT)", [])
            .unwrap();

        for i in 0..10 {
            conn.execute(
                "INSERT INTO test (value) VALUES (?1)",
                [format!("value_{}", i)],
            )
            .unwrap();
        }

        let wal_state = read_wal_file(&wal_path).unwrap();
        assert!(!wal_state.frames.is_empty())
    }
}
