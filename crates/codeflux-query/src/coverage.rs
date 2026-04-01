use codeflux_core::index::CfxReader;
use anyhow::Result;

pub struct CoverageResult {
    pub method: String,
    pub tests: Vec<String>,
    pub test_count: usize,
}

/// Look up which tests cover a given method.
pub fn method_coverage(index: &CfxReader, method_name: &str) -> Result<CoverageResult> {
    let tests = index.lookup_method(method_name);
    let test_count = tests.len();
    Ok(CoverageResult {
        method: method_name.to_string(),
        tests,
        test_count,
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

    fn make_test_reader() -> (tempfile::TempDir, CfxReader) {
        let built = codeflux_ingest::builder::build_index(&fixtures_dir()).unwrap();
        let tmp_dir = tempfile::TempDir::new().unwrap();
        let cfx_path = tmp_dir.path().join("test.cfx");
        codeflux_core::index::write_cfx(
            &cfx_path,
            &built.strings,
            &built.inverted,
            &built.forward,
            &built.file_methods,
            &built.commit_sha,
        ).unwrap();
        let reader = CfxReader::open(&cfx_path).unwrap();
        (tmp_dir, reader)
    }

    #[test]
    fn test_method_coverage_found() {
        let (_tmp, reader) = make_test_reader();
        let result = method_coverage(&reader, "User#deactivate!").unwrap();
        assert_eq!(result.method, "User#deactivate!");
        assert!(result.test_count > 0);
        assert!(result.tests.iter().any(|t| t.contains("test_deactivate")));
    }

    #[test]
    fn test_method_coverage_not_found() {
        let (_tmp, reader) = make_test_reader();
        let result = method_coverage(&reader, "NonExistent#method").unwrap();
        assert_eq!(result.test_count, 0);
        assert!(result.tests.is_empty());
    }
}
