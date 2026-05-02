//! Simple file-based locks using exclusive file creation.

use std::{
    fs::{self, File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

/// A lock represented by an exclusively created file.
#[derive(Debug)]
pub struct LockFile {
    path: PathBuf,
    file: Option<File>,
}

impl LockFile {
    /// Acquires a lock by creating the lock file exclusively.
    pub fn acquire(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;

        Ok(Self {
            path,
            file: Some(file),
        })
    }

    /// Releases the lock and removes the lock file.
    pub fn release(&mut self) -> io::Result<()> {
        self.file.take();
        if self.path.exists() {
            fs::remove_file(&self.path)?;
        }
        Ok(())
    }

    /// Returns the path backing this lock.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LockFile {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::ErrorKind,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::LockFile;

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    fn unique_lock_path() -> std::path::PathBuf {
        let unique = format!(
            "wonder-of-u-lock-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                + u128::from(NEXT_ID.fetch_add(1, Ordering::Relaxed))
        );
        std::env::temp_dir().join(unique)
    }

    #[test]
    fn acquires_and_releases_locks() {
        let path = unique_lock_path();
        let mut lock = LockFile::acquire(&path).unwrap();

        let error = LockFile::acquire(&path).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);

        lock.release().unwrap();
        assert!(LockFile::acquire(&path).is_ok());

        let _ = fs::remove_file(path);
    }
}
