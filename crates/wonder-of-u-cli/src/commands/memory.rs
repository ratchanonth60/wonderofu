//! Memory file management (`wonder-of-u memory`).

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

use wonder_of_u_core::WonderError;

/// Returns the path to the global memory file.
#[must_use]
pub fn global_memory_path(storage_dir: &Path) -> PathBuf {
    storage_dir.join("CLAUDE.md")
}

/// Returns paths to project-local memory files from the current directory upward.
///
/// The walk stops once a Git root is reached, or at the filesystem root when the
/// current directory is not inside a repository.
#[must_use]
pub fn project_memory_paths(cwd: &Path) -> Vec<PathBuf> {
    let git_root = git_root_from(cwd);
    let mut current = Some(cwd);
    let mut paths = Vec::new();

    while let Some(dir) = current {
        let memory_path = dir.join("CLAUDE.md");
        if memory_path.is_file() {
            paths.push(memory_path);
        }

        if git_root.as_deref().is_some_and(|root| root == dir) {
            break;
        }

        current = dir.parent().filter(|parent| *parent != dir);
    }

    paths
}

/// Prints the global and project memory file contents.
#[allow(dead_code)]
pub fn show(storage_dir: &Path) -> Result<(), WonderError> {
    let cwd = std::env::current_dir()?;
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    show_with_writer(storage_dir, &cwd, &mut writer)
}

/// Opens the global memory file in the configured editor.
pub fn edit(storage_dir: &Path) -> Result<(), WonderError> {
    let path = global_memory_path(storage_dir);
    touch_memory_file(&path)?;
    open_editor(&path)
}

/// Prints the path to the global memory file.
#[allow(dead_code)]
pub fn path_cmd(storage_dir: &Path) -> Result<(), WonderError> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    path_cmd_with_writer(storage_dir, &mut writer)
}

pub(crate) fn show_with_writer<W: Write>(
    storage_dir: &Path,
    cwd: &Path,
    writer: &mut W,
) -> Result<(), WonderError> {
    let global_path = global_memory_path(storage_dir);
    writeln!(writer, "== Global memory: {} ==", global_path.display())?;
    write_memory_contents(writer, &global_path)?;

    let project_paths = project_memory_paths(cwd);
    if project_paths.is_empty() {
        writeln!(writer)?;
        writeln!(writer, "== Project memory ==")?;
        writeln!(writer, "(no memory)")?;
        return Ok(());
    }

    for path in project_paths {
        writeln!(writer)?;
        writeln!(writer, "== Project memory: {} ==", path.display())?;
        write_memory_contents(writer, &path)?;
    }

    Ok(())
}

pub(crate) fn path_cmd_with_writer<W: Write>(
    storage_dir: &Path,
    writer: &mut W,
) -> Result<(), WonderError> {
    writeln!(writer, "{}", global_memory_path(storage_dir).display())?;
    Ok(())
}

fn write_memory_contents<W: Write>(writer: &mut W, path: &Path) -> Result<(), WonderError> {
    match fs::read_to_string(path) {
        Ok(contents) if contents.trim().is_empty() => writeln!(writer, "(empty)")?,
        Ok(contents) => {
            write!(writer, "{contents}")?;
            if !contents.ends_with('\n') {
                writeln!(writer)?;
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => writeln!(writer, "(no memory)")?,
        Err(error) => return Err(error.into()),
    }

    Ok(())
}

fn git_root_from(cwd: &Path) -> Option<PathBuf> {
    let mut current = Some(cwd);
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent().filter(|parent| *parent != dir);
    }
    None
}

fn touch_memory_file(path: &Path) -> Result<(), WonderError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new().create(true).append(true).open(path)?;
    Ok(())
}

fn open_editor(path: &Path) -> Result<(), WonderError> {
    if let Some((program, args)) = editor_command() {
        run_editor(&program, &args, path)?;
        return Ok(());
    }

    let mut last_not_found = None;
    for program in ["nano", "vi"] {
        match run_editor(program, &[], path) {
            Ok(()) => return Ok(()),
            Err(error)
                if matches!(
                    &error,
                    WonderError::Io(io_error) if io_error.kind() == io::ErrorKind::NotFound
                ) =>
            {
                last_not_found = Some(error);
            }
            Err(error) => return Err(error),
        }
    }

    Err(last_not_found
        .unwrap_or_else(|| WonderError::validation("no editor configured; set VISUAL or EDITOR")))
}

fn run_editor(program: &str, args: &[String], path: &Path) -> Result<(), WonderError> {
    let status = Command::new(program).args(args).arg(path).status()?;
    if status.success() {
        return Ok(());
    }

    Err(WonderError::validation(format!(
        "editor `{program}` exited with status {status}"
    )))
}

fn editor_command() -> Option<(String, Vec<String>)> {
    let raw = std::env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            std::env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })?;
    let mut tokens = shell_words::split(&raw).ok()?;
    let command = tokens.first()?.clone();
    Some((command, tokens.drain(1..).collect()))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use wonder_of_u_test_support::unique_test_dir;

    use super::{global_memory_path, project_memory_paths, show_with_writer};

    #[test]
    fn global_memory_path_joins_claude_md() {
        let storage_dir = Path::new("/tmp/wonder");

        assert_eq!(
            global_memory_path(storage_dir),
            storage_dir.join("CLAUDE.md")
        );
    }

    #[test]
    fn show_reports_no_memory_when_global_file_is_missing() {
        let storage_dir = unique_test_dir("memory-show-missing");
        let cwd = storage_dir.join("workspace");
        fs::create_dir_all(storage_dir.join(".git")).expect("create git dir");
        fs::create_dir_all(&cwd).expect("create workspace");

        let mut output = Vec::new();
        show_with_writer(&storage_dir, &cwd, &mut output).expect("show memory");

        let text = String::from_utf8(output).expect("utf8");
        assert!(text.contains("(no memory)"));
        assert!(text.contains("== Global memory:"));
        assert!(text.contains("== Project memory =="));
    }

    #[test]
    fn project_memory_paths_finds_claude_md_in_cwd() {
        let repo = unique_test_dir("memory-project-paths");
        let cwd = repo.join("workspace");
        fs::create_dir_all(repo.join(".git")).expect("create git dir");
        fs::create_dir_all(&cwd).expect("create cwd");
        fs::write(cwd.join("CLAUDE.md"), "# project memory\n").expect("write memory");

        let paths = project_memory_paths(&cwd);

        assert_eq!(paths, vec![cwd.join("CLAUDE.md")]);
    }
}
