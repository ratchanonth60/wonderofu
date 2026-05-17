use std::{
    cell::Cell,
    env,
    ffi::OsString,
    fs,
    path::PathBuf,
    sync::{Mutex, MutexGuard, OnceLock},
};

use uuid::Uuid;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("test support crate lives under workspace crates directory")
        .to_path_buf()
}

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
    let path = workspace_root()
        .join("target")
        .join("test-workspaces")
        .join(format!("{}-{}", safe_prefix, Uuid::new_v4()));
    fs::create_dir_all(&path).expect("create unique test directory");
    path
}

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

// Per-thread reentrance counter so nested `EnvVarGuard::set` calls within the
// same test don't deadlock on the non-reentrant `Mutex`.
thread_local! {
    static ENV_LOCK_DEPTH: Cell<u32> = const { Cell::new(0) };
}

pub struct EnvVarGuard {
    // `Some` only for the outermost guard on this thread; `None` for reentrant
    // inner guards (which rely on the outermost guard still holding the lock).
    _lock: Option<MutexGuard<'static, ()>>,
    key: String,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    pub fn set(key: impl Into<String>, value: impl Into<OsString>) -> Self {
        let key = key.into();
        let value = value.into();

        // Acquire the process-wide env lock only once per thread call stack.
        // Recover from poison so that one panicking test doesn't cascade
        // failures into unrelated tests.
        let depth = ENV_LOCK_DEPTH.get();
        let lock = if depth == 0 {
            Some(
                ENV_LOCK
                    .get_or_init(|| Mutex::new(()))
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            )
        } else {
            None
        };
        ENV_LOCK_DEPTH.set(depth + 1);

        let previous = env::var_os(&key);
        unsafe {
            env::set_var(&key, &value);
        }
        Self {
            _lock: lock,
            key,
            previous,
        }
    }

    /// Removes `key` from the environment for the duration of the guard, then
    /// restores its previous value (if any) on drop.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let _guard = EnvVarGuard::remove("MY_VAR");
    /// assert!(std::env::var("MY_VAR").is_err()); // absent during test
    /// // restored (or still absent if it wasn't set before) after drop
    /// ```
    pub fn remove(key: impl Into<String>) -> Self {
        let key = key.into();

        let depth = ENV_LOCK_DEPTH.get();
        let lock = if depth == 0 {
            Some(
                ENV_LOCK
                    .get_or_init(|| Mutex::new(()))
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            )
        } else {
            None
        };
        ENV_LOCK_DEPTH.set(depth + 1);

        let previous = env::var_os(&key);
        unsafe {
            env::remove_var(&key);
        }
        Self {
            _lock: lock,
            key,
            previous,
        }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => unsafe {
                env::set_var(&self.key, value);
            },
            None => unsafe {
                env::remove_var(&self.key);
            },
        }
        let depth = ENV_LOCK_DEPTH.get();
        ENV_LOCK_DEPTH.set(depth.saturating_sub(1));
    }
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

    #[test]
    fn env_var_guard_restores_previous_value() {
        let key = "WONDER_OF_U_TEST_SUPPORT_ENV";
        unsafe {
            std::env::remove_var(key);
        }
        {
            let _guard = EnvVarGuard::set(key, "hello");
            assert_eq!(std::env::var(key).as_deref(), Ok("hello"));
        }
        assert!(std::env::var_os(key).is_none());
    }
}
