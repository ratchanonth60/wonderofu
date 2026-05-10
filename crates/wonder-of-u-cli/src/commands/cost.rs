//! Renders persisted session cost summaries for top-level CLI commands.

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use time::OffsetDateTime;
use wonder_of_u_core::{CostState, Result, SessionId, TokenUsage};
use wonder_of_u_storage::{SessionCostLedger, SessionMetadata, TranscriptStore};

const INPUT_USD_PER_MILLION_TOKENS: f64 = 3.0;
const OUTPUT_USD_PER_MILLION_TOKENS: f64 = 15.0;

#[derive(Clone, Debug, PartialEq)]
struct CostRecord {
    session_id: SessionId,
    cwd: Option<PathBuf>,
    updated_at: OffsetDateTime,
    usage: TokenUsage,
    total_cost_usd: f64,
}

impl CostRecord {
    fn from_metadata(metadata: &SessionMetadata) -> Option<Self> {
        (!metadata.costs.is_empty()).then(|| Self {
            session_id: metadata.session_id,
            cwd: Some(metadata.cwd.clone()),
            updated_at: metadata.updated_at,
            usage: metadata.costs.usage,
            total_cost_usd: resolve_cost_usd(&metadata.costs),
        })
    }

    fn from_ledger(ledger: &SessionCostLedger) -> Option<Self> {
        (!ledger.costs.is_empty()).then(|| Self {
            session_id: ledger.session_id,
            cwd: None,
            updated_at: ledger.costs.updated_at,
            usage: ledger.costs.usage,
            total_cost_usd: resolve_cost_usd(&ledger.costs),
        })
    }

    fn apply_ledger(&mut self, ledger: SessionCostLedger) {
        self.usage = ledger.costs.usage;
        self.total_cost_usd = resolve_cost_usd(&ledger.costs);
        if ledger.costs.updated_at > self.updated_at {
            self.updated_at = ledger.costs.updated_at;
        }
    }
}

/// Shows persisted session cost information.
pub(crate) fn show(storage_dir: Option<&Path>, cwd: &Path, all: bool) -> Result<String> {
    let Some(storage_dir) = storage_dir else {
        return Ok(
            "No cost data available. Re-run with --storage-dir to inspect persisted sessions."
                .into(),
        );
    };

    let records = load_cost_records(storage_dir)?;
    if records.is_empty() {
        return Ok("No cost data available.".into());
    }

    Ok(if all {
        render_all_records(&records)
    } else {
        render_current_record(&records, cwd)
    })
}

fn load_cost_records(storage_dir: &Path) -> Result<Vec<CostRecord>> {
    let store = TranscriptStore::new(storage_dir);
    let mut records = store
        .list_metadata()?
        .into_iter()
        .filter_map(|metadata| CostRecord::from_metadata(&metadata))
        .map(|record| (record.session_id, record))
        .collect::<BTreeMap<_, _>>();

    let sessions_dir = store.paths().sessions_dir();
    match fs::read_dir(&sessions_dir) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if !entry.file_type()?.is_file() {
                    continue;
                }
                if entry.path().extension().and_then(OsStr::to_str) != Some("costs") {
                    continue;
                }

                let ledger: SessionCostLedger =
                    serde_json::from_str(&fs::read_to_string(entry.path())?)?;
                let session_id = ledger.session_id;
                if let Some(record) = CostRecord::from_ledger(&ledger) {
                    match records.get_mut(&session_id) {
                        Some(existing) => existing.apply_ledger(ledger),
                        None => {
                            records.insert(record.session_id, record);
                        }
                    }
                }
            }
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let mut records = records.into_values().collect::<Vec<_>>();
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    Ok(records)
}

fn render_current_record(records: &[CostRecord], cwd: &Path) -> String {
    let current = select_current_record(records, cwd).unwrap_or(&records[0]);
    let mut lines = render_record_lines(current);
    lines.push(String::new());
    lines.push(format!(
        "All-time total:   {}",
        format_usd(sum_total_cost(records))
    ));
    lines.join("\n")
}

fn render_all_records(records: &[CostRecord]) -> String {
    let mut lines = Vec::new();
    for (index, record) in records.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        lines.extend(render_record_lines(record));
    }
    lines.push(String::new());
    lines.push(format!(
        "All-time total:   {}",
        format_usd(sum_total_cost(records))
    ));
    lines.join("\n")
}

fn select_current_record<'a>(records: &'a [CostRecord], cwd: &Path) -> Option<&'a CostRecord> {
    let canonical_cwd = cwd.canonicalize().ok();
    records.iter().find(|record| {
        record
            .cwd
            .as_deref()
            .is_some_and(|record_cwd| paths_match(record_cwd, cwd, canonical_cwd.as_deref()))
    })
}

fn paths_match(left: &Path, right: &Path, canonical_right: Option<&Path>) -> bool {
    left == right
        || match (left.canonicalize().ok(), canonical_right) {
            (Some(canonical_left), Some(canonical_right)) => canonical_left == canonical_right,
            _ => false,
        }
}

fn render_record_lines(record: &CostRecord) -> Vec<String> {
    vec![
        format!("Session: {}", record.session_id),
        render_metric_line("Input tokens", &format_tokens(record.usage.input_tokens)),
        render_metric_line("Output tokens", &format_tokens(record.usage.output_tokens)),
        render_metric_line("Cache read", &format_tokens(record.usage.cache_read_tokens)),
        render_metric_line(
            "Cache write",
            &format_tokens(record.usage.cache_creation_tokens),
        ),
        render_metric_line("Total cost", &format_usd(record.total_cost_usd)),
    ]
}

fn render_metric_line(label: &str, value: &str) -> String {
    format!("  {:<15} {}", format!("{label}:"), value)
}

fn resolve_cost_usd(costs: &CostState) -> f64 {
    costs
        .estimated_cost_usd
        .unwrap_or_else(|| estimate_cost_usd(costs.usage))
}

fn estimate_cost_usd(usage: TokenUsage) -> f64 {
    let input_side_tokens =
        usage.input_tokens + usage.cache_creation_tokens + usage.cache_read_tokens;
    (input_side_tokens as f64 / 1_000_000.0) * INPUT_USD_PER_MILLION_TOKENS
        + (usage.output_tokens as f64 / 1_000_000.0) * OUTPUT_USD_PER_MILLION_TOKENS
}

fn sum_total_cost(records: &[CostRecord]) -> f64 {
    records.iter().map(|record| record.total_cost_usd).sum()
}

fn format_tokens(value: u64) -> String {
    let digits = value.to_string();
    if digits.len() <= 3 {
        return digits;
    }

    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    let first_group_len = match digits.len() % 3 {
        0 => 3,
        remainder => remainder,
    };
    formatted.push_str(&digits[..first_group_len]);
    let mut index = first_group_len;
    while index < digits.len() {
        formatted.push(',');
        formatted.push_str(&digits[index..index + 3]);
        index += 3;
    }
    formatted
}

fn format_usd(value: f64) -> String {
    format!("${value:.4}")
}

#[cfg(test)]
mod tests {
    use super::*;

    use time::OffsetDateTime;
    use wonder_of_u_core::AppState;
    use wonder_of_u_storage::{CostStore, SessionCostLedger, SessionMetadata};
    use wonder_of_u_test_support::unique_test_dir;

    #[test]
    fn show_reports_no_cost_data_when_storage_is_empty() {
        let dir = unique_test_dir("cli-cost-empty");

        let rendered =
            show(Some(dir.as_path()), dir.as_path(), false).expect("render cost summary");

        assert_eq!(rendered, "No cost data available.");
    }

    #[test]
    fn render_record_lines_formats_tokens_and_cost() {
        let record = CostRecord {
            session_id: SessionId::new(),
            cwd: None,
            updated_at: OffsetDateTime::now_utc(),
            usage: TokenUsage {
                input_tokens: 12_345,
                output_tokens: 3_456,
                cache_creation_tokens: 567,
                cache_read_tokens: 1_234,
            },
            total_cost_usd: 0.0842,
        };

        let lines = render_record_lines(&record);

        assert_eq!(lines[0], format!("Session: {}", record.session_id));
        assert_eq!(lines[1], "  Input tokens:   12,345");
        assert_eq!(lines[2], "  Output tokens:  3,456");
        assert_eq!(lines[3], "  Cache read:     1,234");
        assert_eq!(lines[4], "  Cache write:    567");
        assert_eq!(lines[5], "  Total cost:     $0.0842");
    }

    #[test]
    fn show_prefers_matching_cwd_and_falls_back_to_estimated_cost() {
        let dir = unique_test_dir("cli-cost-summary");
        let store = TranscriptStore::new(&dir);
        let cost_store = CostStore::new(&dir);

        let mut previous = AppState::new(dir.join("workspace-old"));
        previous.record_cost_usage(
            TokenUsage {
                input_tokens: 1_000,
                output_tokens: 200,
                cache_creation_tokens: 0,
                cache_read_tokens: 0,
            },
            Some(0.0100),
        );
        store
            .write_metadata(&SessionMetadata::from_app_state(&previous))
            .expect("write previous metadata");
        cost_store
            .write_costs(&SessionCostLedger::from_app_state(&previous))
            .expect("write previous costs");

        let mut current = AppState::new(dir.join("workspace-current"));
        current.record_cost_usage(
            TokenUsage {
                input_tokens: 12_345,
                output_tokens: 3_456,
                cache_creation_tokens: 567,
                cache_read_tokens: 1_234,
            },
            None,
        );
        store
            .write_metadata(&SessionMetadata::from_app_state(&current))
            .expect("write current metadata");
        cost_store
            .write_costs(&SessionCostLedger::from_app_state(&current))
            .expect("write current costs");

        let rendered = show(Some(dir.as_path()), current.session.cwd.as_path(), false)
            .expect("render cost summary");

        assert!(rendered.contains(&format!("Session: {}", current.session.id)));
        assert!(rendered.contains("  Input tokens:   12,345"));
        assert!(rendered.contains("  Output tokens:  3,456"));
        assert!(rendered.contains("  Cache read:     1,234"));
        assert!(rendered.contains("  Cache write:    567"));
        assert!(rendered.contains("All-time total:   $0.1043"));
    }
}
