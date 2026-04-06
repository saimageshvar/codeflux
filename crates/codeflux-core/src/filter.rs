/// Classifies whether a traced method is a project-defined method
/// vs. Ruby core/stdlib/gem noise that was captured at a call site.

/// Known path segments that indicate non-project (gem/stdlib) files.
const EXTERNAL_PATH_MARKERS: &[&str] = &[
    "gems/",
    "/ruby/",
    "/rubygems/",
    "/bundler/",
    "rubygems.rb",
    "<internal:",
    "config/initializers/",
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
    "Enumerator#",
    "Enumerator.",
    "Regexp#",
    "Regexp.",
    "Pathname#",
    "Pathname.",
];

/// Returns true if a file path looks like it belongs to the project
/// (as opposed to gems, stdlib, or absolute system paths).
pub fn is_project_file(path: &str) -> bool {
    !EXTERNAL_PATH_MARKERS.iter().any(|marker| path.contains(marker))
}

/// Returns true if a method name looks like a Ruby stdlib/core method.
pub fn is_stdlib_method(name: &str) -> bool {
    STDLIB_CLASS_PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

/// Returns true if a file path is a test file.
pub fn is_test_file(path: &str) -> bool {
    path.starts_with("test/") || path.contains("/test/")
}

/// Returns true if this (method_name, file_path) pair represents a
/// project-defined method worth reporting to users.
///
/// Excludes: gems, stdlib paths, Ruby core class methods, test files.
pub fn is_project_method(method_name: &str, file_path: &str) -> bool {
    if !is_project_file(file_path) {
        return false;
    }
    if is_test_file(file_path) {
        return false;
    }
    if is_stdlib_method(method_name) {
        return false;
    }
    true
}

/// Count project methods in an index by iterating the file→method map.
pub fn count_project_methods(
    strings: &crate::intern::StringTable,
    file_methods: &crate::graph::FileMethodMap,
) -> usize {
    let mut count = 0;
    for (&file_id, method_ids) in file_methods.iter() {
        let file_path = strings.resolve(file_id);
        for &method_id in method_ids {
            let method_name = strings.resolve(method_id.0);
            if is_project_method(method_name, file_path) {
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_project_file() {
        assert!(is_project_file("app/models/user.rb"));
        assert!(is_project_file("lib/utils.rb"));
        assert!(!is_project_file("gems/activerecord-7.2/lib/ar.rb"));
        assert!(!is_project_file("/usr/lib/ruby/3.2.0/set.rb"));
    }

    #[test]
    fn test_is_stdlib_method() {
        assert!(is_stdlib_method("Integer#+"));
        assert!(is_stdlib_method("BasicObject#initialize"));
        assert!(is_stdlib_method("Class.new"));
        assert!(!is_stdlib_method("Calculator#add"));
        assert!(!is_stdlib_method("User#deactivate!"));
    }

    #[test]
    fn test_is_project_method() {
        assert!(is_project_method("Calculator#add", "app/models/calculator.rb"));
        assert!(!is_project_method("Integer#+", "app/models/calculator.rb"));
        assert!(!is_project_method("Calculator#add", "gems/foo/lib/calc.rb"));
        assert!(!is_project_method("CalculatorTest#test_add", "test/models/calculator_test.rb"));
    }
}
