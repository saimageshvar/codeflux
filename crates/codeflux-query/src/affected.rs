use anyhow::Result;
use std::collections::HashSet;
use std::path::Path;
use git2::{Repository, Diff, DiffDelta, DiffHunk, DiffLine, DiffOptions};

use codeflux_core::index::CfxReader;
use crate::treesitter::RubyMethodMapper;

/// A single affected test result.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AffectedTest {
    pub test_id: String,
}

/// Pre-parsed changed file info (from git diff).
#[derive(Debug, Clone)]
pub struct ChangedFile {
    pub path: String,
    pub changed_lines: Vec<u32>,
}

/// Result of an affected query.
#[derive(Debug)]
pub struct AffectedResult {
    pub tests: Vec<AffectedTest>,
    pub changed_methods: Vec<String>,
    pub fallback_files: Vec<String>,
    pub warnings: Vec<String>,
}

/// Pure function: resolve pre-computed changes against the index.
///
/// For each changed Ruby file:
/// 1. Read the file content from disk
/// 2. Parse with RubyMethodMapper to get method ranges
/// 3. Map changed lines → method names
/// 4. Look up each method in the index
/// 5. If method not found, fall back to file-level lookup
/// 6. Collect and deduplicate test IDs
pub fn resolve_changes(
    changed_files: &[ChangedFile],
    index: &CfxReader,
    project_root: &Path,
) -> AffectedResult {
    let mut tests_set: HashSet<String> = HashSet::new();
    let mut changed_methods: Vec<String> = Vec::new();
    let mut fallback_files: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    for changed_file in changed_files {
        // Only process Ruby files
        if !changed_file.path.ends_with(".rb") {
            continue;
        }

        let full_path = project_root.join(&changed_file.path);
        let source = match std::fs::read_to_string(&full_path) {
            Ok(s) => s,
            Err(e) => {
                warnings.push(format!("could not read {}: {}", changed_file.path, e));
                // Fall back to file-level
                for method_name in index.lookup_file(&changed_file.path) {
                    for test_id in index.lookup_method(&method_name) {
                        tests_set.insert(test_id);
                    }
                }
                fallback_files.push(changed_file.path.clone());
                continue;
            }
        };

        let mapper = match RubyMethodMapper::parse(&source) {
            Ok(m) => m,
            Err(e) => {
                warnings.push(format!("could not parse {}: {}", changed_file.path, e));
                // Fall back to file-level
                for method_name in index.lookup_file(&changed_file.path) {
                    for test_id in index.lookup_method(&method_name) {
                        tests_set.insert(test_id);
                    }
                }
                fallback_files.push(changed_file.path.clone());
                continue;
            }
        };

        let methods = mapper.methods_at_lines(&changed_file.changed_lines);

        if methods.is_empty() && !changed_file.changed_lines.is_empty() {
            // Changed lines don't map to any method — could be class-level changes
            // Fall back to file-level
            for method_name in index.lookup_file(&changed_file.path) {
                for test_id in index.lookup_method(&method_name) {
                    tests_set.insert(test_id);
                }
            }
            fallback_files.push(changed_file.path.clone());
            continue;
        }

        for method_name in &methods {
            let method_tests = index.lookup_method(method_name);
            if method_tests.is_empty() {
                // Method not in index — might be new or renamed
                warnings.push(format!("method {} not found in index", method_name));
            }
            for test_id in method_tests {
                tests_set.insert(test_id);
            }
            changed_methods.push(method_name.clone());
        }
    }

    changed_methods.sort();
    changed_methods.dedup();

    let mut tests: Vec<AffectedTest> = tests_set
        .into_iter()
        .map(|t| AffectedTest { test_id: t })
        .collect();
    tests.sort_by(|a, b| a.test_id.cmp(&b.test_id));

    AffectedResult {
        tests,
        changed_methods,
        fallback_files,
        warnings,
    }
}

/// Extract changed files and line numbers from a git diff.
fn extract_changed_files(diff: &Diff) -> Vec<ChangedFile> {
    use std::cell::RefCell;

    let files: RefCell<Vec<ChangedFile>> = RefCell::new(Vec::new());

    diff.foreach(
        &mut |delta: DiffDelta, _progress: f32| -> bool {
            if let Some(path) = delta.new_file().path().and_then(|p| p.to_str()) {
                files.borrow_mut().push(ChangedFile {
                    path: path.to_string(),
                    changed_lines: Vec::new(),
                });
            }
            true
        },
        None, // binary callback
        Some(&mut |_delta: DiffDelta, _hunk: DiffHunk| -> bool {
            true
        }),
        Some(&mut |_delta: DiffDelta, _hunk: Option<DiffHunk>, line: DiffLine| -> bool {
            if let Some(new_lineno) = line.new_lineno() {
                if let Some(file) = files.borrow_mut().last_mut() {
                    file.changed_lines.push(new_lineno);
                }
            }
            true
        }),
    ).ok();

    files.into_inner()
}

/// Run the affected query against a git repository.
///
/// `project_root`: path to the git repo
/// `index`: the loaded .cfx index
/// `diff_ref`: git ref to diff against (None = uncommitted changes vs HEAD)
pub fn affected_tests(
    project_root: &Path,
    index: &CfxReader,
    diff_ref: Option<&str>,
) -> Result<AffectedResult> {
    let repo = Repository::open(project_root)?;

    let diff = if let Some(ref_name) = diff_ref {
        // Diff between ref and HEAD
        let obj = repo.revparse_single(ref_name)?;
        let old_tree = obj.peel_to_tree()?;
        let head = repo.head()?.peel_to_tree()?;
        repo.diff_tree_to_tree(Some(&old_tree), Some(&head), None)?
    } else {
        // Diff of uncommitted changes (working dir vs HEAD)
        let head = repo.head()?.peel_to_tree()?;
        let mut opts = DiffOptions::new();
        let mut workdir_diff = repo.diff_index_to_workdir(None, Some(&mut opts))?;
        let mut merged = repo.diff_tree_to_index(Some(&head), None, Some(&mut opts))?;
        merged.merge(&mut workdir_diff)?;
        merged
    };

    let changed = extract_changed_files(&diff);
    Ok(resolve_changes(&changed, index, project_root))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
    }

    /// Build a test index and write it to a temp file, then read it back
    fn make_test_reader() -> (TempDir, CfxReader) {
        let built = codeflux_ingest::builder::build_index(&fixtures_dir()).unwrap();
        let tmp_dir = TempDir::new().unwrap();
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
    fn test_resolve_changes_with_method_match() {
        let (_tmp, reader) = make_test_reader();

        // Create a temp project dir with a Ruby file that has a method at line 3
        let project_dir = TempDir::new().unwrap();
        let models_dir = project_dir.path().join("app/models");
        std::fs::create_dir_all(&models_dir).unwrap();
        std::fs::write(models_dir.join("user.rb"), r#"
class User
  def deactivate!
    self.active = false
  end
end
"#).unwrap();

        let changed = vec![ChangedFile {
            path: "app/models/user.rb".to_string(),
            changed_lines: vec![3], // line inside deactivate! method
        }];

        let result = resolve_changes(&changed, &reader, project_dir.path());
        assert!(!result.tests.is_empty());
        assert!(result.changed_methods.contains(&"User#deactivate!".to_string()));
    }

    #[test]
    fn test_resolve_changes_non_ruby_files_ignored() {
        let (_tmp, reader) = make_test_reader();
        let project_dir = TempDir::new().unwrap();

        let changed = vec![ChangedFile {
            path: "README.md".to_string(),
            changed_lines: vec![1, 2, 3],
        }];

        let result = resolve_changes(&changed, &reader, project_dir.path());
        assert!(result.tests.is_empty());
        assert!(result.changed_methods.is_empty());
    }

    #[test]
    fn test_resolve_changes_file_not_found_fallback() {
        let (_tmp, reader) = make_test_reader();
        let project_dir = TempDir::new().unwrap();

        let changed = vec![ChangedFile {
            path: "app/models/user.rb".to_string(),
            changed_lines: vec![142],
        }];

        // File doesn't exist in project_dir — should fall back to file-level
        let result = resolve_changes(&changed, &reader, project_dir.path());
        assert!(!result.fallback_files.is_empty());
    }
}
