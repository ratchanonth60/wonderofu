//! Small git helpers backed by `git` subprocesses.

use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    string::FromUtf8Error,
};

use thiserror::Error;

/// Errors produced by git helper functions.
#[derive(Debug, Error)]
pub enum GitError {
    /// A git subprocess could not be started.
    #[error(transparent)]
    Io(#[from] io::Error),
    /// Git output was not valid UTF-8.
    #[error(transparent)]
    Utf8(#[from] FromUtf8Error),
    /// Git returned a non-success status.
    #[error("git {args:?} failed with status {status:?}: {stderr}")]
    CommandFailed {
        /// Stores the args
        args: Vec<String>,
        /// Stores the status
        status: Option<i32>,
        /// Stores the stderr
        stderr: String,
    },
}

/// Returns whether the provided directory is inside a git worktree.
pub fn is_git_repo(cwd: impl AsRef<Path>) -> bool {
    Command::new("git")
        .arg("rev-parse")
        .arg("--is-inside-work-tree")
        .current_dir(cwd)
        .output()
        .is_ok_and(|output| output.status.success() && trim_ascii(&output.stdout) == b"true")
}

/// Returns the repository root for a working tree.
pub fn get_git_root(cwd: impl AsRef<Path>) -> Result<PathBuf, GitError> {
    run_git(cwd.as_ref(), ["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

/// Returns the current branch name.
pub fn get_current_branch(cwd: impl AsRef<Path>) -> Result<String, GitError> {
    run_git(cwd.as_ref(), ["rev-parse", "--abbrev-ref", "HEAD"])
}

/// Returns the current git diff.
pub fn get_diff(cwd: impl AsRef<Path>, staged: bool) -> Result<String, GitError> {
    let mut args = vec!["diff"];
    if staged {
        args.push("--cached");
    }
    run_git(cwd.as_ref(), args)
}

/// Reads the repository's top-level `.gitignore` file when present.
pub fn read_gitignore(root: impl AsRef<Path>) -> io::Result<Option<String>> {
    let path = root.as_ref().join(".gitignore");
    match fs::read_to_string(path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn run_git<S, I>(cwd: &Path, args: I) -> Result<String, GitError>
where
    S: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
{
    let args = args
        .into_iter()
        .map(|arg| arg.as_ref().to_os_string())
        .collect::<Vec<_>>();
    let output = Command::new("git").args(&args).current_dir(cwd).output()?;

    if !output.status.success() {
        return Err(GitError::CommandFailed {
            args: args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            status: output.status.code(),
            stderr: String::from_utf8(output.stderr)?,
        });
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    &bytes[start..end]
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{get_current_branch, get_diff, get_git_root, is_git_repo, read_gitignore};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new() -> Self {
            let unique = format!(
                "wonder-of-u-core-git-test-{}-{}",
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

    fn run(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?} failed", args);
    }

    fn setup_repo() -> TestDir {
        let dir = TestDir::new();
        run(dir.path(), &["init"]);
        run(dir.path(), &["config", "user.name", "Wonder Test"]);
        run(dir.path(), &["config", "user.email", "wonder@example.com"]);
        fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        fs::write(dir.path().join("file.txt"), "hello\n").unwrap();
        run(dir.path(), &["add", "."]);
        run(dir.path(), &["commit", "-m", "init"]);
        run(dir.path(), &["checkout", "-b", "test-branch"]);
        dir
    }

    #[test]
    fn detects_git_repositories() {
        let repo = setup_repo();

        assert!(is_git_repo(repo.path()));
        assert_eq!(get_git_root(repo.path()).unwrap(), repo.path());
        assert_eq!(get_current_branch(repo.path()).unwrap(), "test-branch");
        assert_eq!(
            read_gitignore(repo.path()).unwrap(),
            Some("target/\n".to_owned())
        );
    }

    #[test]
    fn reads_staged_and_unstaged_diffs() {
        let repo = setup_repo();
        let file = repo.path().join("file.txt");

        fs::write(&file, "hello\nworld\n").unwrap();
        assert!(get_diff(repo.path(), false).unwrap().contains("+world"));

        run(repo.path(), &["add", "file.txt"]);
        let staged_diff = get_diff(repo.path(), true).unwrap();
        assert!(staged_diff.contains("+world"));
    }
}
