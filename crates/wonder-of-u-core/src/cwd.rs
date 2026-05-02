//! Current working directory helpers.

use std::{env, io, path::PathBuf};

/// Returns the current working directory.
pub fn get_cwd() -> io::Result<PathBuf> {
    env::current_dir()
}

#[cfg(test)]
mod tests {
    use super::get_cwd;

    #[test]
    fn returns_process_current_dir() {
        assert_eq!(get_cwd().unwrap(), std::env::current_dir().unwrap());
    }
}
