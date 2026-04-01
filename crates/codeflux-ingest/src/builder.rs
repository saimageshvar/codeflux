use anyhow::{Result, Context};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

use codeflux_core::intern::StringTable;
use codeflux_core::graph::{InvertedIndex, ForwardIndex, FileMethodMap};
use codeflux_core::{MethodId, TestId};
use crate::parser::{self, TraceFile};

/// Statistics from ingestion.
#[derive(Debug)]
pub struct IngestStats {
    pub files_processed: usize,
    pub files_skipped: usize,
    pub files_empty: usize,
    pub total_methods: usize,
    pub total_tests: usize,
}

/// Result of building the index (in-memory, not yet serialized).
pub struct BuiltIndex {
    pub strings: StringTable,
    pub inverted: InvertedIndex,
    pub forward: ForwardIndex,
    pub file_methods: FileMethodMap,
    pub commit_sha: String,
    pub stats: IngestStats,
}

/// Discover all .cft files in a directory.
pub fn discover_cft_files(traces_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !traces_dir.exists() {
        return Ok(files);
    }
    for entry in std::fs::read_dir(traces_dir)
        .with_context(|| format!("reading {}", traces_dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("cft") {
            files.push(path);
        }
    }
    Ok(files)
}

/// Build an in-memory index from .cft files.
pub fn build_index(traces_dir: &Path) -> Result<BuiltIndex> {
    let cft_paths = discover_cft_files(traces_dir)?;

    // Parse all files in parallel
    let parse_results: Vec<(PathBuf, Result<Option<TraceFile>>)> = cft_paths
        .into_par_iter()
        .map(|path| {
            let result = parser::parse_cft(&path);
            (path, result)
        })
        .collect();

    let mut strings = StringTable::new();
    let mut inverted = InvertedIndex::new();
    let mut forward = ForwardIndex::new();
    let mut file_methods = FileMethodMap::new();

    let mut files_processed = 0usize;
    let mut files_skipped = 0usize;
    let mut files_empty = 0usize;
    let mut commit_sha = String::new();

    for (path, result) in parse_results {
        match result {
            Ok(Some(trace)) => {
                if commit_sha.is_empty() {
                    commit_sha = trace.commit_sha.clone();
                }

                let test_id = TestId(strings.intern(&trace.test_id));

                for method in &trace.methods {
                    let method_id = MethodId(strings.intern(&method.qualified_name));
                    let file_id = strings.intern(&method.file_path);
                    let _compound = strings.intern(
                        &format!("{}@{}", method.qualified_name, method.file_path)
                    );

                    inverted.add(method_id, test_id);
                    forward.add(test_id, method_id);
                    file_methods.add(file_id, method_id);
                }

                files_processed += 1;
            }
            Ok(None) => {
                files_empty += 1;
            }
            Err(e) => {
                eprintln!("warning: skipping {}: {}", path.display(), e);
                files_skipped += 1;
            }
        }
    }

    inverted.finalize();
    forward.finalize();
    file_methods.finalize();

    Ok(BuiltIndex {
        stats: IngestStats {
            files_processed,
            files_skipped,
            files_empty,
            total_methods: inverted.method_count(),
            total_tests: forward.test_count(),
        },
        strings,
        inverted,
        forward,
        file_methods,
        commit_sha,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
    }

    #[test]
    fn test_build_index_from_fixtures() {
        let result = build_index(&fixtures_dir()).unwrap();

        // simple.cft has 1 test with 3 methods
        // corrupt.cft has 1 test with 1 valid method
        // empty.cft is skipped
        assert_eq!(result.stats.files_processed, 2);
        assert_eq!(result.stats.files_empty, 1);
        assert_eq!(result.stats.total_tests, 2);
        assert!(result.stats.total_methods > 0);
    }

    #[test]
    fn test_discover_cft_files() {
        let files = discover_cft_files(&fixtures_dir()).unwrap();
        assert_eq!(files.len(), 3); // simple.cft, corrupt.cft, empty.cft
    }

    #[test]
    fn test_discover_missing_dir() {
        let files = discover_cft_files(Path::new("/nonexistent")).unwrap();
        assert!(files.is_empty());
    }

    /// Full round-trip: build from fixtures → write .cfx → read back → verify.
    #[test]
    fn test_write_and_read_roundtrip() {
        use codeflux_core::index::{write_cfx, CfxReader};

        let built = build_index(&fixtures_dir()).unwrap();

        let tmp = tempfile::NamedTempFile::new().unwrap();
        write_cfx(
            tmp.path(),
            &built.strings,
            &built.inverted,
            &built.forward,
            &built.file_methods,
            &built.commit_sha,
        )
        .unwrap();

        let reader = CfxReader::open(tmp.path()).unwrap();

        assert_eq!(reader.method_count(), built.stats.total_methods);
        assert_eq!(reader.test_count(), built.stats.total_tests);
        assert_eq!(reader.commit_sha(), built.commit_sha);

        // Verify a known method lookup from simple.cft
        let tests = reader.lookup_method("User#deactivate!");
        assert!(!tests.is_empty(), "expected at least one test for User#deactivate!");
        assert!(
            tests.iter().any(|t| t.contains("test_deactivate")),
            "expected a test containing 'test_deactivate', got: {:?}",
            tests
        );
    }
}
