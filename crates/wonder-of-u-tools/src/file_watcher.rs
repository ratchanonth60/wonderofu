//! Filesystem change monitoring for workspace awareness.
//!
//! Provides a lightweight wrapper around `notify` for watching
//! workspace file changes that the TUI or tools may need to react to.

use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// A workspace file change event.
#[derive(Clone, Debug)]
pub struct FileChangeEvent {
    /// The path that changed.
    pub path: PathBuf,
    /// The type of change.
    pub kind: FileChangeKind,
}

/// Kind of filesystem change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileChangeKind {
    /// File or directory was created.
    Created,
    /// File or directory was modified.
    Modified,
    /// File or directory was removed.
    Removed,
    /// Other/unknown change.
    Other,
}

impl From<EventKind> for FileChangeKind {
    fn from(kind: EventKind) -> Self {
        match kind {
            EventKind::Create(_) => Self::Created,
            EventKind::Modify(_) => Self::Modified,
            EventKind::Remove(_) => Self::Removed,
            _ => Self::Other,
        }
    }
}

/// A file system watcher that monitors workspace changes.
pub struct FileWatcher {
    /// Receiver for change events.
    rx: mpsc::Receiver<FileChangeEvent>,
    /// The underlying notify watcher.
    _watcher: RecommendedWatcher,
}

impl FileWatcher {
    /// Start watching a directory for changes.
    pub fn watch(dir: &Path) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();

        let tx_clone = tx.clone();
        let mut watcher = notify::recommended_watcher(
            move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    for path in event.paths {
                        let _ = tx_clone.send(FileChangeEvent {
                            path,
                            kind: event.kind.into(),
                        });
                    }
                }
            },
        )
        .map_err(|e| format!("failed to create watcher: {e}"))?;

        watcher
            .configure(
                Config::default()
                    .with_poll_interval(Duration::from_secs(2)),
            )
            .map_err(|e| format!("failed to configure watcher: {e}"))?;

        watcher
            .watch(dir, RecursiveMode::NonRecursive)
            .map_err(|e| format!("failed to watch dir: {e}"))?;

        Ok(Self {
            rx,
            _watcher: watcher,
        })
    }

    /// Poll for the next change event, non-blocking.
    pub fn try_recv(&self) -> Option<FileChangeEvent> {
        self.rx.try_recv().ok()
    }

    /// Block until a change event arrives.
    pub fn recv(&self) -> Result<FileChangeEvent, mpsc::RecvError> {
        self.rx.recv()
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn detect_file_creation() {
        let dir = unique_test_dir("file-watcher-create");
        let watcher = FileWatcher::watch(&dir).expect("watch");
        let test_file = dir.join("test.txt");

        let mut f = fs::File::create(&test_file).expect("create");
        writeln!(f, "hello").expect("write");
        drop(f);

        std::thread::sleep(Duration::from_millis(250));

        let events: Vec<_> = std::iter::from_fn(|| watcher.try_recv()).collect();
        assert!(!events.is_empty(), "expected at least one event");
    }
}
