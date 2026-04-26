use std::{
    env,
    ffi::OsString,
    fs,
    path::PathBuf,
    sync::{Mutex, MutexGuard, OnceLock},
};

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

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub struct EnvVarGuard {
    _lock: MutexGuard<'static, ()>,
    key: String,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    pub fn set(key: impl Into<String>, value: impl Into<OsString>) -> Self {
        let key = key.into();
        let value = value.into();
        let lock = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("lock env guard");
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
