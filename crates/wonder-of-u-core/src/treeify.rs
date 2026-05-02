//! ASCII tree rendering helpers.

use std::collections::BTreeMap;

#[derive(Debug, Default)]
struct Node {
    children: BTreeMap<String, Node>,
}

/// Renders file paths as an ASCII directory tree.
pub fn render_path_tree<I, S>(paths: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut root = Node::default();

    for path in paths {
        let parts = path
            .as_ref()
            .split('/')
            .filter(|part| !part.is_empty() && *part != ".");
        let mut current = &mut root;
        for part in parts {
            current = current.children.entry(part.to_owned()).or_default();
        }
    }

    let mut lines = Vec::new();
    render_children(&root, "", &mut lines);
    lines.join("\n")
}

fn render_children(node: &Node, prefix: &str, lines: &mut Vec<String>) {
    let len = node.children.len();
    for (index, (name, child)) in node.children.iter().enumerate() {
        let is_last = index + 1 == len;
        let connector = if is_last { "└── " } else { "├── " };
        lines.push(format!("{prefix}{connector}{name}"));

        let next_prefix = format!("{prefix}{}", if is_last { "    " } else { "│   " });
        render_children(child, &next_prefix, lines);
    }
}

#[cfg(test)]
mod tests {
    use super::render_path_tree;

    #[test]
    fn renders_a_directory_tree() {
        let rendered = render_path_tree(["src/lib.rs", "src/xml.rs", "tests/core.rs"]);

        assert_eq!(
            rendered,
            "├── src\n│   ├── lib.rs\n│   └── xml.rs\n└── tests\n    └── core.rs"
        );
    }
}
