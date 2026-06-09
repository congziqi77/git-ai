use crate::daemon::DaemonConfig;
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

const RAW_TRACE2_FILE: &str = "trace2-raw.jsonl";
const DEFAULT_RAW_TRACE2_MAX_BYTES: u64 = 10 * 1024 * 1024;

pub(crate) fn raw_trace2_log_path() -> Result<PathBuf, crate::error::GitAiError> {
    Ok(DaemonConfig::from_env_or_default_paths()?
        .internal_dir
        .join("daemon")
        .join(RAW_TRACE2_FILE))
}

pub(crate) fn maybe_append_raw_trace2_payload(payload: &Value) {
    if !raw_trace2_diagnostics_enabled() {
        return;
    }

    let Err(error) = append_raw_trace2_payload(payload) else {
        return;
    };
    tracing::warn!(%error, "raw trace2 diagnostic append failed");
}

pub(crate) fn trace2_summary(payload: &Value) -> Value {
    json!({
        "event": payload.get("event").and_then(Value::as_str),
        "sid": payload.get("sid").and_then(Value::as_str),
        "thread": payload.get("thread").and_then(Value::as_str),
        "time": payload.get("time").and_then(Value::as_str),
        "name": payload.get("name").and_then(Value::as_str),
        "argv": payload.get("argv"),
        "code": payload.get("code"),
        "param": payload.get("param").and_then(Value::as_str),
        "value": payload.get("value").and_then(Value::as_str),
        "repo": payload.get("repo"),
        "worktree": payload.get("worktree"),
    })
}

fn raw_trace2_diagnostics_enabled() -> bool {
    if let Some(enabled) = env_flag("GIT_AI_RAW_TRACE2_LOG") {
        return enabled;
    }
    if let Some(enabled) = env_flag("GIT_AI_TRACE2_DIAGNOSTICS") {
        return enabled;
    }
    true
}

fn env_flag(name: &str) -> Option<bool> {
    std::env::var(name).ok().map(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn append_raw_trace2_payload(payload: &Value) -> Result<(), crate::error::GitAiError> {
    let path = raw_trace2_log_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    rotate_if_needed(&path)?;
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    serde_json::to_writer(&mut file, payload)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn rotate_if_needed(path: &PathBuf) -> Result<(), crate::error::GitAiError> {
    let max_bytes = std::env::var("GIT_AI_TRACE2_DIAGNOSTICS_MAX_BYTES")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_RAW_TRACE2_MAX_BYTES);

    let Ok(metadata) = fs::metadata(path) else {
        return Ok(());
    };
    if metadata.len() < max_bytes {
        return Ok(());
    }

    let rotated = path.with_extension("jsonl.1");
    let _ = fs::remove_file(&rotated);
    fs::rename(path, rotated)?;
    Ok(())
}
