use codeflux_core::filter::is_project_method;
use codeflux_core::index::CfxReader;
use crate::treesitter::RubyMethodMapper;
use anyhow::Result;
use std::collections::HashSet;
use std::path::Path;

pub struct UntestedMethod {
    pub qualified_name: String,
    pub file_path: String,
}

pub struct UntestedResult {
    pub methods: Vec<UntestedMethod>,
    pub total_methods: usize,
    pub untested_count: usize,
}

/// Find methods with zero test coverage.
///
/// When `project_root` is provided, source files matching `path_filter` are
/// parsed with tree-sitter to enumerate *all* defined methods — including those
/// that were never executed by any test and are therefore absent from the index.
///
/// When `project_root` is None, falls back to the index-only scan (only methods
/// that appear in at least one trace are considered).
///
/// `path_filter`: optional prefix filter, e.g., "app/models/".
/// When no filter is given and no project_root, only project-owned source
/// methods are considered (gems, stdlib, and test files are excluded).
pub fn untested_methods(
    index: &CfxReader,
    path_filter: Option<&str>,
    project_root: Option<&Path>,
) -> Result<UntestedResult> {
    if let (Some(filter), Some(root)) = (path_filter, project_root) {
        untested_methods_from_source(index, filter, root)
    } else {
        untested_methods_from_index(index, path_filter)
    }
}

/// Scan source files on disk matching `path_filter` using tree-sitter,
/// then cross-reference all discovered methods against the index.
/// This catches methods that were never traced at all.
fn untested_methods_from_source(
    index: &CfxReader,
    path_filter: &str,
    project_root: &Path,
) -> Result<UntestedResult> {
    let mut methods = Vec::new();
    let mut total_methods = 0usize;
    let mut seen: HashSet<String> = HashSet::new();

    // Walk the project directory for Ruby files matching the filter.
    let search_root = project_root.join(path_filter.trim_end_matches('/'));
    collect_ruby_files_untested(
        index,
        &search_root,
        path_filter,
        project_root,
        &mut methods,
        &mut total_methods,
        &mut seen,
    )?;

    let untested_count = methods.len();
    methods.sort_by(|a, b| a.file_path.cmp(&b.file_path).then(a.qualified_name.cmp(&b.qualified_name)));

    Ok(UntestedResult {
        methods,
        total_methods,
        untested_count,
    })
}

fn collect_ruby_files_untested(
    index: &CfxReader,
    dir: &Path,
    path_filter: &str,
    project_root: &Path,
    methods: &mut Vec<UntestedMethod>,
    total_methods: &mut usize,
    seen: &mut HashSet<String>,
) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    // If the path_filter points directly to a file, handle it
    if dir.is_file() {
        if dir.extension().and_then(|e| e.to_str()) == Some("rb") {
            let rel = dir.strip_prefix(project_root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            process_ruby_file(index, dir, &rel, methods, total_methods, seen)?;
        }
        return Ok(());
    }

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_ruby_files_untested(index, &path, path_filter, project_root, methods, total_methods, seen)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rb") {
            let rel = path.strip_prefix(project_root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            if rel.starts_with(path_filter) {
                process_ruby_file(index, &path, &rel, methods, total_methods, seen)?;
            }
        }
    }
    Ok(())
}

fn process_ruby_file(
    index: &CfxReader,
    abs_path: &Path,
    rel_path: &str,
    methods: &mut Vec<UntestedMethod>,
    total_methods: &mut usize,
    seen: &mut HashSet<String>,
) -> Result<()> {
    let source = match std::fs::read_to_string(abs_path) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };

    let mapper = match RubyMethodMapper::parse(&source) {
        Ok(m) => m,
        Err(_) => return Ok(()),
    };

    for method_range in mapper.all_methods() {
        let key = format!("{}@{}", method_range.qualified_name, rel_path);
        if !seen.insert(key) {
            continue;
        }
        *total_methods += 1;
        let covering_tests = index.lookup_method(&method_range.qualified_name);
        if covering_tests.is_empty() {
            methods.push(UntestedMethod {
                qualified_name: method_range.qualified_name.clone(),
                file_path: rel_path.to_string(),
            });
        }
    }
    Ok(())
}

/// Index-only scan: only considers methods that appeared in at least one trace.
fn untested_methods_from_index(index: &CfxReader, path_filter: Option<&str>) -> Result<UntestedResult> {
    let mut methods = Vec::new();
    let mut total_methods = 0usize;

    for (&file_id, method_ids) in index.file_methods().iter() {
        let file_path = index.strings().resolve(file_id);

        if let Some(filter) = path_filter {
            if !file_path.starts_with(filter) {
                continue;
            }
        }

        for &method_id in method_ids {
            let method_name = index.strings().resolve(method_id.0);

            // When no explicit filter, skip non-project methods
            if path_filter.is_none() && !is_project_method(method_name, file_path) {
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
    fn test_untested_with_filter_index_only() {
        let (_tmp, reader) = make_test_reader();
        // Index-only mode (no project_root): all traced methods have coverage
        let result = untested_methods(&reader, Some("app/"), None).unwrap();
        assert_eq!(result.untested_count, 0);
    }

    #[test]
    fn test_untested_nonexistent_path() {
        let (_tmp, reader) = make_test_reader();
        let result = untested_methods(&reader, Some("nonexistent/"), None).unwrap();
        assert_eq!(result.total_methods, 0);
        assert_eq!(result.untested_count, 0);
    }

    #[test]
    fn test_untested_source_scan_finds_untraced_method() {
        let (_tmp, reader) = make_test_reader();

        // Create a temp project with a Ruby file that has an extra method not in the index
        let project_dir = tempfile::TempDir::new().unwrap();
        let models_dir = project_dir.path().join("app/models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("user.rb"), r#"
class User
  def deactivate!
    self.active = false
  end

  def never_called_method
    # This method was never traced
  end
end
"#).unwrap();

        let result = untested_methods(
            &reader,
            Some("app/models/"),
            Some(project_dir.path()),
        ).unwrap();

        // deactivate! is in the index with coverage; never_called_method is not
        assert!(result.methods.iter().any(|m| m.qualified_name == "User#never_called_method"),
            "expected never_called_method to be reported as untested, got: {:?}",
            result.methods.iter().map(|m| &m.qualified_name).collect::<Vec<_>>());
    }
}
