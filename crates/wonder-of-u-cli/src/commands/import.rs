//! `import` command — explicit one-shot TypeScript transcript importer.
//!
//! Converts TS-shaped JSONL files produced by the upstream Claude Code
//! TypeScript client (`~/.claude/projects/*/`) into Rust storage schema.
//! This command never touches the normal session load path.

use std::path::Path;

use wonder_of_u_core::SessionId;
use wonder_of_u_storage::{TranscriptStore, import_ts_file, inspect_ts_file};

use wonder_of_u_core::Result;

/// Runs the `import` command, writing a human-readable report to `writer`.
///
/// * `source`      – Path to the TS JSONL transcript file to import.
/// * `storage_dir` – Destination storage directory (required unless `dry_run`).
/// * `dry_run`     – When `true`, parse and report without writing.
/// * `session_id`  – Optional override; otherwise detected from source or fresh.
pub fn run_import<W: std::io::Write>(
    source: &Path,
    storage_dir: Option<&Path>,
    dry_run: bool,
    session_id: Option<SessionId>,
    writer: &mut W,
) -> Result<()> {
    if dry_run {
        let report = inspect_ts_file(source)?;
        render_report(&report, writer)?;
        return Ok(());
    }

    let storage_dir = storage_dir.ok_or_else(|| {
        wonder_of_u_core::WonderError::validation(
            "import requires --storage-dir (or set WONDER_OF_U_STORAGE_DIR / HOME)",
        )
    })?;

    let store = TranscriptStore::new(storage_dir);
    let report = import_ts_file(source, &store, session_id)?;
    render_report(&report, writer)?;
    Ok(())
}

fn render_report<W: std::io::Write>(
    report: &wonder_of_u_storage::TsImportReport,
    writer: &mut W,
) -> Result<()> {
    let mode = if report.dry_run { "DRY-RUN" } else { "IMPORT" };
    writeln!(writer, "[{mode}] {}", report.source_path.display())?;
    writeln!(writer, "  session : {}", report.session_id)?;
    if report.dry_run {
        writeln!(writer, "  would import : N/A (dry-run)")?;
    } else {
        writeln!(writer, "  imported : {} message(s)", report.imported_count)?;
    }
    writeln!(writer, "  skipped  : {} record(s)", report.skipped.len())?;
    writeln!(
        writer,
        "  paste-ref warnings : {}",
        report.paste_warnings.len()
    )?;

    if !report.skipped.is_empty() {
        writeln!(writer, "\nSkipped records:")?;
        for skip in &report.skipped {
            writeln!(
                writer,
                "  line {:>4}  type={:?}  {}",
                skip.line,
                skip.ts_type,
                skip.reason.description()
            )?;
        }
    }

    if !report.paste_warnings.is_empty() {
        writeln!(
            writer,
            "\nPaste-reference warnings (placeholder text preserved):"
        )?;
        for w in &report.paste_warnings {
            writeln!(writer, "  line {:>4}  {:?}", w.line, w.snippet)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::path::PathBuf;

    use super::*;
    use wonder_of_u_test_support::unique_test_dir;

    fn write_fixture(dir: &Path, lines: &[&str]) -> PathBuf {
        let path = dir.join("fixture.jsonl");
        let mut f = std::fs::File::create(&path).expect("create fixture");
        for line in lines {
            writeln!(f, "{line}").expect("write line");
        }
        path
    }

    const USER_MSG: &str = r#"{"type":"user","message":{"role":"user","content":"Hello CLI"},"uuid":"cccccccc-0000-0000-0000-000000000001","sessionId":"dddddddd-0000-0000-0000-000000000001","timestamp":"2024-07-01T08:00:00Z","version":"2.1.0"}"#;
    const ASST_MSG: &str = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Hi CLI!"}],"role":"assistant"},"uuid":"cccccccc-0000-0000-0000-000000000002","sessionId":"dddddddd-0000-0000-0000-000000000001","timestamp":"2024-07-01T08:00:01Z","version":"2.1.0"}"#;
    const SYS_MSG: &str = r#"{"type":"system","message":{"role":"system","content":"system prompt"},"uuid":"cccccccc-0000-0000-0000-000000000003","sessionId":"dddddddd-0000-0000-0000-000000000001","timestamp":"2024-07-01T08:00:02Z","version":"2.1.0"}"#;

    #[test]
    fn dry_run_writes_nothing_to_storage() {
        let dir = unique_test_dir("import-cmd-dryrun");
        let source = write_fixture(&dir, &[USER_MSG, ASST_MSG]);

        let mut out = Vec::new();
        run_import(&source, Some(&dir), true, None, &mut out).expect("dry run");

        let output = String::from_utf8(out).expect("utf8");
        assert!(output.contains("DRY-RUN"));
        assert!(output.contains("skipped  : 0"));

        // Nothing written to storage (no .jsonl transcript files).
        let sessions_dir = dir.join("sessions");
        let written = std::fs::read_dir(&sessions_dir)
            .map(|rd| rd.count())
            .unwrap_or(0);
        assert_eq!(written, 0, "dry-run must not write any transcript files");
    }

    #[test]
    fn real_import_writes_transcript_and_reports() {
        let dir = unique_test_dir("import-cmd-real");
        let source = write_fixture(&dir, &[USER_MSG, ASST_MSG, SYS_MSG]);

        let mut out = Vec::new();
        run_import(&source, Some(&dir), false, None, &mut out).expect("import");

        let output = String::from_utf8(out).expect("utf8");
        assert!(output.contains("IMPORT"));
        assert!(output.contains("imported : 2 message(s)"));
        assert!(output.contains("skipped  : 1 record(s)"));
        assert!(output.contains("system")); // skip reason includes the type
    }

    #[test]
    fn missing_storage_dir_without_dry_run_returns_error() {
        let dir = unique_test_dir("import-cmd-no-storage");
        let source = write_fixture(&dir, &[USER_MSG]);

        let mut out = Vec::new();
        let result = run_import(&source, None, false, None, &mut out);
        assert!(result.is_err(), "expected error when storage_dir is None");
    }
}
