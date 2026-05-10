use std::{
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use serde::Deserialize;
use wonder_of_u_core::{Result, WonderError};
use wonder_of_u_tui::message::SearchMatch;

const MAX_MATCHES_PER_FILE: usize = 10;
const MAX_TOTAL_MATCHES: usize = 200;

#[derive(Debug, Deserialize)]
struct RipgrepEvent {
    #[serde(rename = "type")]
    kind: String,
    data: Option<RipgrepMatchData>,
}

#[derive(Debug, Deserialize)]
struct RipgrepMatchData {
    path: RipgrepText,
    line_number: u32,
    lines: RipgrepText,
}

#[derive(Debug, Deserialize)]
struct RipgrepText {
    text: Option<String>,
}

pub(crate) fn search_workspace(cwd: &Path, query: &str) -> Result<Vec<SearchMatch>> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }

    let output = ProcessCommand::new("rg")
        .arg("--json")
        .arg("-m")
        .arg(MAX_MATCHES_PER_FILE.to_string())
        .arg("-e")
        .arg(query)
        .arg(cwd)
        .output()?;

    if !output.status.success() && output.status.code() != Some(1) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(WonderError::internal(format!(
            "ripgrep search failed: {}",
            stderr.trim()
        )));
    }

    let mut matches = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if matches.len() >= MAX_TOTAL_MATCHES {
            break;
        }
        let Ok(event) = serde_json::from_str::<RipgrepEvent>(line) else {
            continue;
        };
        if event.kind != "match" {
            continue;
        }
        let Some(data) = event.data else {
            continue;
        };
        let Some(path) = data.path.text else {
            continue;
        };
        matches.push(SearchMatch {
            file: display_path(cwd, &path),
            line: data.line_number,
            text: data
                .lines
                .text
                .unwrap_or_default()
                .trim_end_matches(['\r', '\n'])
                .to_string(),
        });
    }

    Ok(matches)
}

fn display_path(cwd: &Path, raw_path: &str) -> String {
    let path = PathBuf::from(raw_path);
    let display = if path.is_absolute() {
        path.strip_prefix(cwd).unwrap_or(&path).to_path_buf()
    } else {
        path
    };
    display.to_string_lossy().replace('\\', "/")
}
