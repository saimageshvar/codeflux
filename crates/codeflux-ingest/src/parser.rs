use anyhow::{Context, Result, bail};
use std::path::Path;

/// A single method entry from a .cft file.
#[derive(Debug, Clone, PartialEq)]
pub struct MethodEntry {
    /// e.g., "User#deactivate!"
    pub qualified_name: String,
    /// e.g., "app/models/user.rb"
    pub file_path: String,
    /// e.g., 142
    pub line: u32,
}

/// Parsed contents of a single .cft trace file.
#[derive(Debug, Clone)]
pub struct TraceFile {
    /// e.g., "test/unit/models/user_test.rb::UserTest#test_deactivate"
    pub test_id: String,
    /// Commit SHA at trace time.
    pub commit_sha: String,
    /// Methods invoked by this test.
    pub methods: Vec<MethodEntry>,
}

/// Parse a .cft file. Returns None if the file is empty.
/// Skips malformed M lines with a warning to stderr.
pub fn parse_cft(path: &Path) -> Result<Option<TraceFile>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;

    if content.trim().is_empty() {
        return Ok(None);
    }

    let mut test_id: Option<String> = None;
    let mut commit_sha = String::new();
    let mut methods = Vec::new();

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix("T ") {
            test_id = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("C ") {
            commit_sha = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("M ") {
            match parse_method_line(rest) {
                Some(entry) => methods.push(entry),
                None => {
                    eprintln!(
                        "warning: {}:{}: skipping malformed method line: {}",
                        path.display(),
                        line_num + 1,
                        line
                    );
                }
            }
        }
        // Unknown prefixes are silently ignored for forward compatibility
    }

    let test_id = match test_id {
        Some(id) => id,
        None => bail!("no T (test ID) line in {}", path.display()),
    };

    Ok(Some(TraceFile {
        test_id,
        commit_sha,
        methods,
    }))
}

/// Parse the remainder after "M ": "User#deactivate! app/models/user.rb:142"
fn parse_method_line(rest: &str) -> Option<MethodEntry> {
    // Split on first space: "qualified_name file_path:line"
    let (qualified_name, file_and_line) = rest.split_once(' ')?;
    let (file_path, line_str) = file_and_line.rsplit_once(':')?;
    let line: u32 = line_str.parse().ok()?;

    Some(MethodEntry {
        qualified_name: qualified_name.to_string(),
        file_path: file_path.to_string(),
        line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-fixtures")
            .join(name)
    }

    #[test]
    fn test_parse_simple() {
        let result = parse_cft(&fixture("simple.cft")).unwrap().unwrap();
        assert_eq!(
            result.test_id,
            "test/unit/models/user_test.rb::UserTest#test_deactivate"
        );
        assert_eq!(result.commit_sha, "fdd907a7cd4b");
        assert_eq!(result.methods.len(), 3);
        assert_eq!(result.methods[0].qualified_name, "User#deactivate!");
        assert_eq!(result.methods[0].file_path, "app/models/user.rb");
        assert_eq!(result.methods[0].line, 142);
    }

    #[test]
    fn test_parse_empty() {
        let result = parse_cft(&fixture("empty.cft")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_corrupt_skips_bad_lines() {
        let result = parse_cft(&fixture("corrupt.cft")).unwrap().unwrap();
        assert_eq!(result.test_id, "test/unit/models/user_test.rb::UserTest#test_broken");
        // "GARBAGE LINE" is skipped, "M User#bar app/mod" is skipped (no :line)
        assert_eq!(result.methods.len(), 1);
        assert_eq!(result.methods[0].qualified_name, "User#foo");
    }
}
