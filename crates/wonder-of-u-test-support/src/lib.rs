use std::{fs, path::PathBuf};

use uuid::Uuid;

/// Creates a unique test directory below this repository's `target/` tree.
/// This avoids OS temp directories so tests stay inside the workspace.
pub fn unique_test_dir(prefix: &str) -> PathBuf {
    let safe_prefix = prefix
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let path = std::env::current_dir()
        .expect("current directory")
        .join("target")
        .join("test-workspaces")
        .join(format!("{}-{}", safe_prefix, Uuid::new_v4()));
    fs::create_dir_all(&path).expect("create unique test directory");
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_test_dir_is_under_target() {
        let path = unique_test_dir("support");
        assert!(path.ends_with(path.file_name().expect("leaf")));
        assert!(path.to_string_lossy().contains("target/test-workspaces"));
        assert!(path.exists());
    }
}
