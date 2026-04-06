use anyhow::Result;
use tree_sitter::Node;

pub struct MethodRange {
    /// e.g., "User#deactivate!" or "User.find_by_email"
    pub qualified_name: String,
    pub start_line: u32,
    pub end_line: u32,
}

pub struct RubyMethodMapper {
    methods: Vec<MethodRange>,
}

impl RubyMethodMapper {
    pub fn parse(source: &str) -> Result<Self> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_ruby::LANGUAGE.into())?;

        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("tree-sitter failed to parse source"))?;

        let mut methods = Vec::new();
        let root = tree.root_node();

        walk_node(root, source.as_bytes(), &mut Vec::new(), &mut methods);

        Ok(Self { methods })
    }

    /// Returns method names whose range overlaps any of the given 1-based line numbers.
    pub fn methods_at_lines(&self, lines: &[u32]) -> Vec<String> {
        self.methods
            .iter()
            .filter(|m| lines.iter().any(|&l| l >= m.start_line && l <= m.end_line))
            .map(|m| m.qualified_name.clone())
            .collect()
    }

    /// Returns all parsed method ranges.
    pub fn all_methods(&self) -> &[MethodRange] {
        &self.methods
    }
}

/// Extract the text of a node from the source bytes.
fn node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Find the first named child with the given field name.
fn child_by_field<'a>(node: Node<'a>, field: &str) -> Option<Node<'a>> {
    node.child_by_field_name(field)
}

fn walk_node(node: Node, source: &[u8], scope: &mut Vec<String>, methods: &mut Vec<MethodRange>) {
    match node.kind() {
        "class" | "module" => {
            // Get the scope name from the `name` field (a `constant` or `scope_resolution` node)
            let scope_name = child_by_field(node, "name")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();

            scope.push(scope_name);

            // Walk children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_node(child, source, scope, methods);
            }

            scope.pop();
        }
        "method" => {
            let method_name = child_by_field(node, "name")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();

            let scope_prefix = if scope.is_empty() {
                String::new()
            } else {
                scope.join("::")
            };

            let qualified_name = if scope_prefix.is_empty() {
                method_name
            } else {
                format!("{}#{}", scope_prefix, method_name)
            };

            // tree-sitter lines are 0-based; convert to 1-based
            let start_line = node.start_position().row as u32 + 1;
            let end_line = node.end_position().row as u32 + 1;

            methods.push(MethodRange {
                qualified_name,
                start_line,
                end_line,
            });

            // Walk inside method body for any nested classes/modules (rare but possible)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_node(child, source, scope, methods);
            }
        }
        "singleton_method" => {
            // def self.foo  →  object=self, name=foo
            let method_name = child_by_field(node, "name")
                .map(|n| node_text(n, source).to_string())
                .unwrap_or_default();

            let scope_prefix = if scope.is_empty() {
                String::new()
            } else {
                scope.join("::")
            };

            let qualified_name = if scope_prefix.is_empty() {
                method_name
            } else {
                format!("{}.{}", scope_prefix, method_name)
            };

            let start_line = node.start_position().row as u32 + 1;
            let end_line = node.end_position().row as u32 + 1;

            methods.push(MethodRange {
                qualified_name,
                start_line,
                end_line,
            });

            // Walk inside for nested definitions
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_node(child, source, scope, methods);
            }
        }
        _ => {
            // Recurse into all other nodes
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_node(child, source, scope, methods);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_class() {
        let source = r#"
class User
  def deactivate!
    self.active = false
  end

  def activate!
    self.active = true
  end
end
"#;
        let mapper = RubyMethodMapper::parse(source).unwrap();
        let methods = mapper.methods_at_lines(&[3]);
        assert_eq!(methods, vec!["User#deactivate!"]);

        let methods = mapper.methods_at_lines(&[7]);
        assert_eq!(methods, vec!["User#activate!"]);
    }

    #[test]
    fn test_class_method() {
        let source = r#"
class User
  def self.find_active
    where(active: true)
  end
end
"#;
        let mapper = RubyMethodMapper::parse(source).unwrap();
        let methods = mapper.methods_at_lines(&[3]);
        assert_eq!(methods, vec!["User.find_active"]);
    }

    #[test]
    fn test_nested_module() {
        let source = r#"
module Admin
  class UsersController
    def index
      @users = User.all
    end
  end
end
"#;
        let mapper = RubyMethodMapper::parse(source).unwrap();
        let methods = mapper.methods_at_lines(&[4]);
        assert_eq!(methods, vec!["Admin::UsersController#index"]);
    }

    #[test]
    fn test_no_method_at_line() {
        let source = "class Foo\nend\n";
        let mapper = RubyMethodMapper::parse(source).unwrap();
        let methods = mapper.methods_at_lines(&[1]);
        assert!(methods.is_empty());
    }
}
