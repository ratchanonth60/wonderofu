//! Top-level `wonder-of-u vim` helpers.

use std::path::Path;

use wonder_of_u_agent::SettingsStore;
use wonder_of_u_core::Result;

/// Returns the current persisted vim-mode status line.
pub fn show(storage_dir: &Path) -> Result<String> {
    Ok(render_vim_mode(load_vim_mode(storage_dir)?))
}

/// Persists the vim-mode setting and returns the updated status line.
pub fn set(storage_dir: &Path, enabled: bool) -> Result<String> {
    let store = SettingsStore::new(storage_dir);
    let mut settings = store.read()?;
    settings.vim_mode = Some(enabled);
    store.write(&settings)?;
    Ok(render_vim_mode(enabled))
}

/// Toggles the persisted vim-mode setting and returns the updated status line.
pub fn toggle(storage_dir: &Path) -> Result<String> {
    let enabled = !load_vim_mode(storage_dir)?;
    set(storage_dir, enabled)
}

fn load_vim_mode(storage_dir: &Path) -> Result<bool> {
    Ok(SettingsStore::new(storage_dir)
        .read()?
        .vim_mode
        .unwrap_or(true))
}

fn render_vim_mode(enabled: bool) -> String {
    format!("vim mode: {}", if enabled { "on" } else { "off" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wonder_of_u_agent::SettingsStore;
    use wonder_of_u_test_support::unique_test_dir;

    #[test]
    fn vim_show_reports_default_on_state() {
        let dir = unique_test_dir("cli-vim-show-default");

        let rendered = show(&dir).expect("show vim mode");

        assert_eq!(rendered, "vim mode: on");
    }

    #[test]
    fn vim_set_on_and_off_persists() {
        let dir = unique_test_dir("cli-vim-set");

        let rendered = set(&dir, false).expect("disable vim mode");
        assert_eq!(rendered, "vim mode: off");
        assert_eq!(
            SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .vim_mode,
            Some(false)
        );

        let rendered = set(&dir, true).expect("enable vim mode");
        assert_eq!(rendered, "vim mode: on");
        assert_eq!(
            SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .vim_mode,
            Some(true)
        );
    }

    #[test]
    fn vim_toggle_flips_persisted_value() {
        let dir = unique_test_dir("cli-vim-toggle");

        let rendered = toggle(&dir).expect("toggle vim mode off");
        assert_eq!(rendered, "vim mode: off");
        assert_eq!(
            SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .vim_mode,
            Some(false)
        );

        let rendered = toggle(&dir).expect("toggle vim mode on");
        assert_eq!(rendered, "vim mode: on");
        assert_eq!(
            SettingsStore::new(&dir)
                .read()
                .expect("read settings")
                .vim_mode,
            Some(true)
        );
    }
}
