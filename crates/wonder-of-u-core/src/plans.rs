//! Helpers for locating and updating local plan files.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// Candidate filenames checked when looking up a plan file.
pub const PLAN_FILE_CANDIDATES: [&str; 3] = ["plan.md", "PLAN.md", "claude_plan.md"];

/// Reads the first matching plan file in a directory.
pub fn read_plan_file(cwd: impl AsRef<Path>) -> io::Result<Option<String>> {
    for candidate in PLAN_FILE_CANDIDATES {
        match fs::read_to_string(cwd.as_ref().join(candidate)) {
            Ok(contents) => return Ok(Some(contents)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        }
    }

    Ok(None)
}

/// Writes a plan file, preferring an existing candidate name when present.
pub fn write_plan_file(cwd: impl AsRef<Path>, content: &str) -> io::Result<PathBuf> {
    let cwd = cwd.as_ref();
    let path = PLAN_FILE_CANDIDATES
        .iter()
        .map(|candidate| cwd.join(candidate))
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| cwd.join(PLAN_FILE_CANDIDATES[0]));

    fs::write(&path, content)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{read_plan_file, write_plan_file};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = format!(
                "wonder-of-u-plan-test-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
                    + u128::from(NEXT_ID.fetch_add(1, Ordering::Relaxed))
            );
            let path = std::env::temp_dir().join(unique);
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn reads_existing_plan_candidates_in_priority_order() {
        let dir = TestDir::new();
        fs::write(dir.path().join("PLAN.md"), "capital").unwrap();

        assert_eq!(
            read_plan_file(dir.path()).unwrap(),
            Some("capital".to_owned())
        );
    }

    #[test]
    fn writes_to_existing_plan_file_or_default_name() {
        let dir = TestDir::new();
        fs::write(dir.path().join("claude_plan.md"), "old").unwrap();

        let path = write_plan_file(dir.path(), "new").unwrap();

        assert_eq!(path.file_name().unwrap(), "claude_plan.md");
        assert_eq!(read_plan_file(dir.path()).unwrap(), Some("new".to_owned()));
    }
}
