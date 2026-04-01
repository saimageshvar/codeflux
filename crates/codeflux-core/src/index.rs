use std::io::{self, Write};
use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::graph::{FileMethodMap, ForwardIndex, InvertedIndex};
use crate::intern::StringTable;
use crate::{MethodId, TestId, CFX_MAGIC, CFX_MIN_READER_VERSION, CFX_VERSION};

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Write a built index to a `.cfx` binary file (little-endian).
pub fn write_cfx(
    path: &Path,
    strings: &StringTable,
    inverted: &InvertedIndex,
    forward: &ForwardIndex,
    file_methods: &FileMethodMap,
    commit_sha: &str,
) -> Result<()> {
    let file = std::fs::File::create(path)
        .with_context(|| format!("creating {}", path.display()))?;
    let mut w = io::BufWriter::new(file);

    // --- Header (32 bytes) ---
    // magic: [u8; 4]
    w.write_all(CFX_MAGIC)?;
    // version: u16
    w.write_all(&CFX_VERSION.to_le_bytes())?;
    // min_reader_version: u16
    w.write_all(&CFX_MIN_READER_VERSION.to_le_bytes())?;
    // commit_sha: [u8; 20] (zero-padded)
    let mut sha_bytes = [0u8; 20];
    let sha_src = commit_sha.as_bytes();
    let copy_len = sha_src.len().min(20);
    sha_bytes[..copy_len].copy_from_slice(&sha_src[..copy_len]);
    w.write_all(&sha_bytes)?;
    // method_count: u32
    let method_count = inverted.method_count() as u32;
    w.write_all(&method_count.to_le_bytes())?;

    // --- String Table Section ---
    let entries = strings.entries();
    let blob = strings.blob();
    // entry_count: u32
    w.write_all(&(entries.len() as u32).to_le_bytes())?;
    // entries: [(offset: u32, len: u32); entry_count]
    for &(offset, len) in entries {
        w.write_all(&offset.to_le_bytes())?;
        w.write_all(&len.to_le_bytes())?;
    }
    // blob_len: u32
    w.write_all(&(blob.len() as u32).to_le_bytes())?;
    // blob: [u8; blob_len]
    w.write_all(blob)?;

    // --- Inverted Index Section ---
    // posting_count: u32
    let inverted_postings: Vec<(&MethodId, &[TestId])> = inverted.iter().collect();
    w.write_all(&(inverted_postings.len() as u32).to_le_bytes())?;
    for (&method_id, tests) in &inverted_postings {
        w.write_all(&method_id.0.to_le_bytes())?;
        w.write_all(&(tests.len() as u32).to_le_bytes())?;
        for t in *tests {
            w.write_all(&t.0.to_le_bytes())?;
        }
    }

    // --- Forward Index Section ---
    // posting_count: u32
    let forward_postings: Vec<(&TestId, &[MethodId])> =
        forward.iter().collect();
    w.write_all(&(forward_postings.len() as u32).to_le_bytes())?;
    for (&test_id, methods) in &forward_postings {
        w.write_all(&test_id.0.to_le_bytes())?;
        w.write_all(&(methods.len() as u32).to_le_bytes())?;
        for m in *methods {
            w.write_all(&m.0.to_le_bytes())?;
        }
    }

    // --- FileMethod Map Section ---
    // entry_count: u32
    let fm_entries: Vec<(&u32, &[MethodId])> = file_methods.iter().collect();
    w.write_all(&(fm_entries.len() as u32).to_le_bytes())?;
    for (&file_id, methods) in &fm_entries {
        w.write_all(&file_id.to_le_bytes())?;
        w.write_all(&(methods.len() as u32).to_le_bytes())?;
        for m in *methods {
            w.write_all(&m.0.to_le_bytes())?;
        }
    }

    w.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Reader helpers
// ---------------------------------------------------------------------------

/// Read a u32 little-endian from a byte slice at position `pos`, advancing pos.
fn read_u32(data: &[u8], pos: &mut usize) -> Result<u32> {
    let end = *pos + 4;
    if end > data.len() {
        return Err(anyhow!("unexpected end of file reading u32 at offset {}", pos));
    }
    let val = u32::from_le_bytes(data[*pos..end].try_into().unwrap());
    *pos = end;
    Ok(val)
}

fn read_u16(data: &[u8], pos: &mut usize) -> Result<u16> {
    let end = *pos + 2;
    if end > data.len() {
        return Err(anyhow!("unexpected end of file reading u16 at offset {}", pos));
    }
    let val = u16::from_le_bytes(data[*pos..end].try_into().unwrap());
    *pos = end;
    Ok(val)
}

fn read_bytes<'a>(data: &'a [u8], pos: &mut usize, len: usize) -> Result<&'a [u8]> {
    let end = *pos + len;
    if end > data.len() {
        return Err(anyhow!(
            "unexpected end of file reading {} bytes at offset {}",
            len,
            pos
        ));
    }
    let slice = &data[*pos..end];
    *pos = end;
    Ok(slice)
}

// ---------------------------------------------------------------------------
// CfxReader
// ---------------------------------------------------------------------------

pub struct CfxReader {
    strings: StringTable,
    inverted: InvertedIndex,
    forward: ForwardIndex,
    file_methods: FileMethodMap,
    commit_sha: String,
    method_count: u32,
}

impl CfxReader {
    /// Parse a `.cfx` file from disk and reconstruct all in-memory structures.
    pub fn open(path: &Path) -> Result<Self> {
        let data = std::fs::read(path)
            .with_context(|| format!("reading {}", path.display()))?;

        let mut pos = 0usize;

        // --- Header ---
        let magic = read_bytes(&data, &mut pos, 4)?;
        if magic != CFX_MAGIC.as_ref() {
            return Err(anyhow!(
                "invalid magic bytes: expected {:?}, got {:?}",
                CFX_MAGIC,
                magic
            ));
        }
        let version = read_u16(&data, &mut pos)?;
        if version < CFX_MIN_READER_VERSION {
            return Err(anyhow!(
                "index version {} is too old (min {})",
                version,
                CFX_MIN_READER_VERSION
            ));
        }
        let _min_reader_version = read_u16(&data, &mut pos)?;
        let sha_bytes = read_bytes(&data, &mut pos, 20)?;
        // Trim trailing zero bytes to reconstruct the original SHA string.
        let sha_trimmed = sha_bytes.iter().rposition(|&b| b != 0)
            .map(|last| &sha_bytes[..=last])
            .unwrap_or(&[]);
        let commit_sha = std::str::from_utf8(sha_trimmed)
            .context("commit_sha is not valid UTF-8")?
            .to_owned();
        let method_count = read_u32(&data, &mut pos)?;

        // --- String Table ---
        let entry_count = read_u32(&data, &mut pos)? as usize;
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let offset = read_u32(&data, &mut pos)?;
            let len = read_u32(&data, &mut pos)?;
            entries.push((offset, len));
        }
        let blob_len = read_u32(&data, &mut pos)? as usize;
        let blob_slice = read_bytes(&data, &mut pos, blob_len)?;
        let strings = StringTable::from_parts(entries, blob_slice.to_vec());

        // --- Inverted Index ---
        let mut inverted = InvertedIndex::new();
        let inv_posting_count = read_u32(&data, &mut pos)? as usize;
        for _ in 0..inv_posting_count {
            let method_id = MethodId(read_u32(&data, &mut pos)?);
            let test_count = read_u32(&data, &mut pos)? as usize;
            for _ in 0..test_count {
                let test_id = TestId(read_u32(&data, &mut pos)?);
                inverted.add(method_id, test_id);
            }
        }
        // Already sorted when written; finalize just deduplicates (safe to call again).
        inverted.finalize();

        // --- Forward Index ---
        let mut forward = ForwardIndex::new();
        let fwd_posting_count = read_u32(&data, &mut pos)? as usize;
        for _ in 0..fwd_posting_count {
            let test_id = TestId(read_u32(&data, &mut pos)?);
            let method_count_fwd = read_u32(&data, &mut pos)? as usize;
            for _ in 0..method_count_fwd {
                let method_id = MethodId(read_u32(&data, &mut pos)?);
                forward.add(test_id, method_id);
            }
        }
        forward.finalize();

        // --- FileMethod Map ---
        let mut file_methods = FileMethodMap::new();
        let fm_entry_count = read_u32(&data, &mut pos)? as usize;
        for _ in 0..fm_entry_count {
            let file_id = read_u32(&data, &mut pos)?;
            let method_count_fm = read_u32(&data, &mut pos)? as usize;
            for _ in 0..method_count_fm {
                let method_id = MethodId(read_u32(&data, &mut pos)?);
                file_methods.add(file_id, method_id);
            }
        }
        file_methods.finalize();

        Ok(Self {
            strings,
            inverted,
            forward,
            file_methods,
            commit_sha,
            method_count,
        })
    }

    /// Look up tests that cover a method by its string name.
    /// Returns a list of test name strings.
    pub fn lookup_method(&self, method_name: &str) -> Vec<String> {
        match self.strings.lookup(method_name) {
            None => vec![],
            Some(mid) => {
                let method_id = MethodId(mid);
                self.inverted
                    .get(method_id)
                    .iter()
                    .map(|tid| self.strings.resolve(tid.0).to_owned())
                    .collect()
            }
        }
    }

    /// Look up methods defined in a file by its path string.
    pub fn lookup_file(&self, file_path: &str) -> Vec<String> {
        match self.strings.lookup(file_path) {
            None => vec![],
            Some(fid) => self
                .file_methods
                .get(fid)
                .iter()
                .map(|mid| self.strings.resolve(mid.0).to_owned())
                .collect(),
        }
    }

    /// Get all method names present in the inverted index.
    pub fn all_methods(&self) -> Vec<String> {
        self.inverted
            .iter()
            .map(|(&mid, _)| self.strings.resolve(mid.0).to_owned())
            .collect()
    }

    pub fn method_count(&self) -> usize {
        self.method_count as usize
    }

    pub fn test_count(&self) -> usize {
        self.forward.test_count()
    }

    pub fn commit_sha(&self) -> &str {
        &self.commit_sha
    }

    pub fn strings(&self) -> &StringTable {
        &self.strings
    }

    pub fn inverted(&self) -> &InvertedIndex {
        &self.inverted
    }

    pub fn file_methods(&self) -> &FileMethodMap {
        &self.file_methods
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a small in-memory index that mirrors the data in simple.cft.
    ///
    /// simple.cft:
    ///   test: test/unit/models/user_test.rb::UserTest#test_deactivate
    ///   commit: fdd907a7cd4b
    ///   methods: User#deactivate!, User#update_status, ActiveRecord::Base#save
    fn make_test_index() -> (StringTable, InvertedIndex, ForwardIndex, FileMethodMap, String) {
        let commit_sha = "fdd907a7cd4b".to_owned();

        let mut strings = StringTable::new();
        let test_id = TestId(strings.intern(
            "test/unit/models/user_test.rb::UserTest#test_deactivate",
        ));
        let m1 = MethodId(strings.intern("User#deactivate!"));
        let m2 = MethodId(strings.intern("User#update_status"));
        let m3 = MethodId(strings.intern("ActiveRecord::Base#save"));
        let f1 = strings.intern("app/models/user.rb");
        let f2 = strings.intern(
            "gems/activerecord-7.2.2/lib/active_record/persistence.rb",
        );

        let mut inverted = InvertedIndex::new();
        inverted.add(m1, test_id);
        inverted.add(m2, test_id);
        inverted.add(m3, test_id);
        inverted.finalize();

        let mut forward = ForwardIndex::new();
        forward.add(test_id, m1);
        forward.add(test_id, m2);
        forward.add(test_id, m3);
        forward.finalize();

        let mut file_methods = FileMethodMap::new();
        file_methods.add(f1, m1);
        file_methods.add(f1, m2);
        file_methods.add(f2, m3);
        file_methods.finalize();

        (strings, inverted, forward, file_methods, commit_sha)
    }

    #[test]
    fn test_write_and_read_roundtrip() {
        let (strings, inverted, forward, file_methods, commit_sha) = make_test_index();

        let tmp = tempfile::NamedTempFile::new().unwrap();
        write_cfx(
            tmp.path(),
            &strings,
            &inverted,
            &forward,
            &file_methods,
            &commit_sha,
        )
        .unwrap();

        let reader = CfxReader::open(tmp.path()).unwrap();

        // Verify counts match
        assert_eq!(reader.method_count(), inverted.method_count());
        assert_eq!(reader.test_count(), forward.test_count());
        assert_eq!(reader.commit_sha(), commit_sha);

        // Verify a known method lookup
        let tests = reader.lookup_method("User#deactivate!");
        assert!(!tests.is_empty(), "expected at least one test for User#deactivate!");
        assert!(
            tests.iter().any(|t| t.contains("test_deactivate")),
            "expected a test containing 'test_deactivate', got: {:?}",
            tests
        );

        // Verify file lookup
        let methods = reader.lookup_file("app/models/user.rb");
        assert!(methods.contains(&"User#deactivate!".to_owned()));
        assert!(methods.contains(&"User#update_status".to_owned()));

        // Verify all_methods returns something
        let all = reader.all_methods();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_invalid_magic() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"NOPE").unwrap();
        assert!(CfxReader::open(tmp.path()).is_err());
    }

    #[test]
    fn test_commit_sha_roundtrip() {
        let (strings, inverted, forward, file_methods, _) = make_test_index();
        // The header field is 20 bytes; use a short commit SHA (≤ 20 chars).
        let sha = "fdd907a7cd4b";

        let tmp = tempfile::NamedTempFile::new().unwrap();
        write_cfx(tmp.path(), &strings, &inverted, &forward, &file_methods, sha).unwrap();

        let reader = CfxReader::open(tmp.path()).unwrap();
        assert_eq!(reader.commit_sha(), sha);
    }
}
