use codeflux_core::index::CfxReader;
use anyhow::Result;

pub struct UntestedMethod {
    pub qualified_name: String,
    pub file_path: String,
}

pub struct UntestedResult {
    pub methods: Vec<UntestedMethod>,
    pub total_methods: usize,
    pub untested_count: usize,
}

/// Known path segments that indicate non-project (gem/stdlib) files.
const EXTERNAL_PATH_MARKERS: &[&str] = &[
    "gems/",
    "/ruby/",
    "/rubygems/",
    "/bundler/",
    "rubygems.rb",
    "<internal:",
];

/// Ruby core/stdlib classes whose methods are traced when called from project
/// code, but which are not project-defined methods.
const STDLIB_CLASS_PREFIXES: &[&str] = &[
    "BasicObject#",
    "BasicObject.",
    "Kernel#",
    "Kernel.",
    "Object#",
    "Object.",
    "Class#",
    "Class.",
    "Module#",
    "Module.",
    "Integer#",
    "Integer.",
    "Float#",
    "Float.",
    "String#",
    "String.",
    "Array#",
    "Array.",
    "Hash#",
    "Hash.",
    "Symbol#",
    "Symbol.",
    "NilClass#",
    "TrueClass#",
    "FalseClass#",
    "Comparable#",
    "Enumerable#",
    "IO#",
    "IO.",
    "File#",
    "File.",
    "Dir#",
    "Dir.",
    "Proc#",
    "Thread#",
    "Thread.",
    "Mutex#",
];

/// Returns true if a file path looks like it belongs to the project
/// (as opposed to gems, stdlib, or absolute system paths).
fn is_project_file(path: &str) -> bool {
    !EXTERNAL_PATH_MARKERS.iter().any(|marker| path.contains(marker))
}

/// Returns true if a method name looks like a Ruby stdlib/core method.
fn is_stdlib_method(name: &str) -> bool {
    STDLIB_CLASS_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// Find methods with zero test coverage.
/// Uses the file_methods map: for each file, check each method.
/// A method is "untested" if it has zero entries in the inverted index.
///
/// `path_filter`: optional prefix filter, e.g., "app/models/".
/// When no filter is given, only project-owned files are considered
/// (gems and stdlib paths are excluded).
pub fn untested_methods(index: &CfxReader, path_filter: Option<&str>) -> Result<UntestedResult> {
    let mut methods = Vec::new();
    let mut total_methods = 0usize;

    // Iterate all files and their methods via file_methods
    for (&file_id, method_ids) in index.file_methods().iter() {
        let file_path = index.strings().resolve(file_id);

        // Apply path filter or default project-file filter
        if let Some(filter) = path_filter {
            if !file_path.starts_with(filter) {
                continue;
            }
        } else {
            // Default: skip non-project files, test files, and tracing infrastructure
            if !is_project_file(file_path) {
                continue;
            }
            if file_path.starts_with("test/") || file_path.contains("/test/") {
                continue;
            }
        }

        for &method_id in method_ids {
            let method_name = index.strings().resolve(method_id.0);

            // Skip Ruby core/stdlib methods that were traced at call sites
            // in project files but aren't project-defined methods.
            if path_filter.is_none() && is_stdlib_method(method_name) {
                continue;
            }

            total_methods += 1;
            let tests = index.inverted().get(method_id);
            if tests.is_empty() {
                methods.push(UntestedMethod {
                    qualified_name: method_name.to_string(),
                    file_path: file_path.to_string(),
                });
            }
        }
    }

    let untested_count = methods.len();
    methods.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.qualified_name.cmp(&b.qualified_name)));

    Ok(UntestedResult {
        methods,
        total_methods,
        untested_count,
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
    fn test_untested_with_filter() {
        let (_tmp, reader) = make_test_reader();
        // All methods in our test fixtures should be tested (they come from trace files)
        let result = untested_methods(&reader, Some("app/")).unwrap();
        // In our fixtures, methods are traced so they should have coverage
        assert_eq!(result.untested_count, 0);
    }

    #[test]
    fn test_untested_nonexistent_path() {
        let (_tmp, reader) = make_test_reader();
        let result = untested_methods(&reader, Some("nonexistent/")).unwrap();
        assert_eq!(result.total_methods, 0);
        assert_eq!(result.untested_count, 0);
    }
}
