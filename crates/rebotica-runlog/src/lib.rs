use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PersistedRun {
    pub id: String,
    pub directory: PathBuf,
}

pub fn make_id() -> String {
    let stamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let suffix = Uuid::new_v4().to_string()[..4].to_string();
    format!("{stamp}-{suffix}")
}

pub fn root() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".rebotica")
}

pub fn runs_root() -> PathBuf {
    root().join("runs")
}

pub fn persist(
    mode: &str,
    model: &str,
    envelope: &str,
    prompt: &str,
    response: &str,
) -> Result<PersistedRun> {
    let id = make_id();
    let directory = runs_root().join(&id);
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;

    fs::write(directory.join("task-envelope.yml"), envelope)?;
    fs::write(directory.join("prompt.md"), prompt)?;
    fs::write(directory.join("model-response.md"), response)?;
    fs::write(
        directory.join("parsed-output.json"),
        serde_json::to_string_pretty(&json!({
            "mode": mode,
            "model": model,
            "response_unparsed": true
        }))?,
    )?;

    let project = std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    fs::write(
        directory.join("scorecard.yml"),
        format!(
            "run_id: {id}\nproject: {project}\nmodel: {model}\nmode: {mode}\naccepted: false\nneeded_human_correction: null\ntests_passed: null\ndiff_lines: null\nfiles_changed: null\nproblems: []\nimprovements: []\n"
        ),
    )?;

    let scorecard = root().join("model-scorecards.yml");
    if !scorecard.exists() {
        fs::create_dir_all(root())?;
        fs::write(scorecard, "models: {}\n")?;
    }

    Ok(PersistedRun { id, directory })
}

pub fn retrospective_template(run_id: &str) -> String {
    format!(
        "# Retrospective: {run_id}\n\n## What failed?\n\n## What surprised us?\n\n## Was context missing?\n\n## Was the task too broad?\n\n## Did the local model violate scope?\n\n## Did checks catch the issue?\n\n## Should project config change?\n\n## Should prompt or model routing change?\n"
    )
}
