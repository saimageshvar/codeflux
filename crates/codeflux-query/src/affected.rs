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

        // Skip test files — changes to tests don't tell us anything about
        // which other tests need to run; the user re-runs changed test files directly.
        if changed_file.path.starts_with("test/") || changed_file.path.contains("/test/") {
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
            // Only collect actually changed lines, not context lines.
            // origin() returns '+' for additions, '-' for deletions, ' ' for context.
            let origin = line.origin();
            if origin == '+' || origin == '-' {
                // For additions, use new_lineno; for deletions, use old_lineno
                let lineno = if origin == '+' {
                    line.new_lineno()
                } else {
                    line.old_lineno()
                };
                if let Some(ln) = lineno {
                    if let Some(file) = files.borrow_mut().last_mut() {
                        file.changed_lines.push(ln);
                    }
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
/// `diff_ref`: git ref to diff against (None = working directory vs HEAD)
/// `include_uncommitted`: when true and `diff_ref` is Some, diff the ref against
///   the working directory (including staged changes) rather than against HEAD.
///   When `diff_ref` is None this flag has no effect — working-tree-vs-HEAD is
///   already the default.
pub fn affected_tests(
    project_root: &Path,
    index: &CfxReader,
    diff_ref: Option<&str>,
    include_uncommitted: bool,
) -> Result<AffectedResult> {
    let repo = Repository::open(project_root)?;

    let diff = match (diff_ref, include_uncommitted) {
        (Some(ref_name), true) => {
            // Ref vs working directory (with index) — covers everything on the
            // branch that isn't in `ref_name` yet, whether committed or not.
            let obj = repo.revparse_single(ref_name)?;
            let old_tree = obj.peel_to_tree()?;
            let mut opts = DiffOptions::new();
            repo.diff_tree_to_workdir_with_index(Some(&old_tree), Some(&mut opts))?
        }
        (Some(ref_name), false) => {
            // Ref vs HEAD (committed changes only).
            let obj = repo.revparse_single(ref_name)?;
            let old_tree = obj.peel_to_tree()?;
            let head = repo.head()?.peel_to_tree()?;
            repo.diff_tree_to_tree(Some(&old_tree), Some(&head), None)?
        }
        (None, _) => {
            // Uncommitted changes (working dir vs HEAD). `include_uncommitted`
            // is a no-op here.
            let head = repo.head()?.peel_to_tree()?;
            let mut opts = DiffOptions::new();
            let mut workdir_diff = repo.diff_index_to_workdir(None, Some(&mut opts))?;
            let mut merged = repo.diff_tree_to_index(Some(&head), None, Some(&mut opts))?;
            merged.merge(&mut workdir_diff)?;
            merged
        }
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

    /// End-to-end test for the `--include-uncommitted` mode: initialize a
    /// real git repo, land a committed change to `deactivate!`, then stack
    /// an uncommitted change on top. Verify all three ref+flag combinations
    /// return the expected slice of the diff.
    #[test]
    fn test_affected_tests_ref_vs_worktree_includes_uncommitted() {
        use git2::{Repository, Signature};
        use std::fs;

        let (_idx_tmp, reader) = make_test_reader();

        let project_dir = TempDir::new().unwrap();
        let root = project_dir.path();
        let repo = Repository::init(root).unwrap();
        let sig = Signature::now("Test", "test@example.com").unwrap();

        let models_dir = root.join("app/models");
        fs::create_dir_all(&models_dir).unwrap();
        let user_rb = models_dir.join("user.rb");

        // Base commit — User#deactivate! with an original body. The `--diff`
        // ref will point here.
        fs::write(&user_rb,
            "class User\n  def deactivate!\n    :original\n  end\nend\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("app/models/user.rb")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let base_oid = repo.commit(Some("HEAD"), &sig, &sig, "base", &tree, &[]).unwrap();
        let base_commit = repo.find_commit(base_oid).unwrap();

        // HEAD commit — modifies the body of deactivate!.
        fs::write(&user_rb,
            "class User\n  def deactivate!\n    :committed_change\n  end\nend\n").unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("app/models/user.rb")).unwrap();
        idx.write().unwrap();
        let tree_id = idx.write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "head", &tree, &[&base_commit]).unwrap();

        // Uncommitted change on top — modifies deactivate! again.
        fs::write(&user_rb,
            "class User\n  def deactivate!\n    :uncommitted_change\n  end\nend\n").unwrap();

        let base_sha = base_oid.to_string();

        // (1) include_uncommitted=true against base — combines committed +
        // uncommitted; should see the method change.
        let r = affected_tests(root, &reader, Some(&base_sha), true).unwrap();
        assert!(
            r.changed_methods.contains(&"User#deactivate!".to_string()),
            "ref+worktree: expected User#deactivate!, got {:?}", r.changed_methods
        );
        assert!(!r.tests.is_empty(), "ref+worktree: expected non-empty tests");

        // (2) include_uncommitted=false against HEAD — HEAD vs HEAD, empty.
        let r = affected_tests(root, &reader, Some("HEAD"), false).unwrap();
        assert!(r.tests.is_empty(), "HEAD vs HEAD: expected empty");
        assert!(r.changed_methods.is_empty());

        // (3) include_uncommitted=true against HEAD — only the working-tree
        // slice should show up.
        let r = affected_tests(root, &reader, Some("HEAD"), true).unwrap();
        assert!(
            r.changed_methods.contains(&"User#deactivate!".to_string()),
            "HEAD+worktree: expected User#deactivate! from uncommitted, got {:?}",
            r.changed_methods
        );
    }
}
