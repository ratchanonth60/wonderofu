//! Provides the top-level `output-style` CLI command.

use std::path::{Path, PathBuf};

use wonder_of_u_agent::SettingsStore;
use wonder_of_u_core::{Result, WonderError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputStyle {
    Markdown,
    Plain,
    Raw,
}

impl OutputStyle {
    const ALL: [Self; 3] = [Self::Markdown, Self::Plain, Self::Raw];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Plain => "plain",
            Self::Raw => "raw",
        }
    }
}

pub(crate) fn show(storage_dir: Option<&Path>) -> Result<String> {
    Ok(format!(
        "current_output_style={}",
        output_style_name(persisted_output_style(storage_dir)?.as_deref())
    ))
}

#[must_use]
pub(crate) fn list() -> String {
    OutputStyle::ALL
        .iter()
        .map(|style| style.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn set(storage_dir: Option<&Path>, name: &str) -> Result<String> {
    let style = parse_output_style(name)?;
    let store = SettingsStore::new(require_storage_dir(storage_dir)?);
    let mut settings = store.read()?;
    settings.output_style = (style != OutputStyle::Markdown).then(|| style.as_str().to_string());
    store.write(&settings)?;
    Ok(format!(
        "output_style={}\nstatus=output style updated",
        style.as_str()
    ))
}

fn persisted_output_style(storage_dir: Option<&Path>) -> Result<Option<String>> {
    let Some(storage_dir) = storage_dir else {
        return Ok(None);
    };
    Ok(SettingsStore::new(storage_dir).read()?.output_style)
}

fn parse_output_style(value: &str) -> Result<OutputStyle> {
    match value {
        "markdown" => Ok(OutputStyle::Markdown),
        "plain" => Ok(OutputStyle::Plain),
        "raw" => Ok(OutputStyle::Raw),
        other => Err(WonderError::validation(format!(
            "unknown output style: {other}"
        ))),
    }
}

#[must_use]
fn output_style_name(value: Option<&str>) -> &'static str {
    match value {
        Some("plain") => "plain",
        Some("raw") => "raw",
        _ => "markdown",
    }
}

fn require_storage_dir(storage_dir: Option<&Path>) -> Result<PathBuf> {
    storage_dir.map(Path::to_path_buf).ok_or_else(|| {
        WonderError::validation(
            "output-style command requires --storage-dir or HOME/XDG_CONFIG_HOME",
        )
    })
}

#[cfg(test)]
mod tests {
    use wonder_of_u_agent::SettingsStore;
    use wonder_of_u_test_support::unique_test_dir;

    use super::*;

    #[test]
    fn output_style_list_contains_available_styles() {
        let styles = list();

        assert_eq!(styles, "markdown\nplain\nraw");
    }

    #[test]
    fn output_style_set_persists_valid_style() {
        let dir = unique_test_dir("cli-output-style-set");

        let output = set(Some(dir.as_path()), "plain").expect("set output style");

        assert!(output.contains("output_style=plain"));
        let settings = SettingsStore::new(&dir).read().expect("read settings");
        assert_eq!(settings.output_style.as_deref(), Some("plain"));
        assert_eq!(
            show(Some(dir.as_path())).expect("show output style"),
            "current_output_style=plain"
        );
    }

    #[test]
    fn output_style_set_rejects_unknown_style() {
        let dir = unique_test_dir("cli-output-style-invalid");

        let error = set(Some(dir.as_path()), "html").expect_err("unknown output style should fail");

        assert!(error.to_string().contains("unknown output style: html"));
    }
}
