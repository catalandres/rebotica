use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub mod ledger;
#[cfg(test)]
mod tests_support;

#[derive(Debug, Clone)]
pub struct PersistedRun {
    pub id: String,
    pub directory: PathBuf,
}

/// Prime's per-run disposition vocabulary. Matches the ledger event payload
/// shape that will land with the SQLite ledger (issue #44).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    /// Prime accepted the model's output as-is.
    Accept,
    /// Prime rejected the output.
    Reject,
    /// Prime used the output but edited it first.
    EditThenUse,
    /// Default state: Prime has not yet recorded a disposition.
    #[default]
    Unscored,
}

impl Disposition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Reject => "reject",
            Self::EditThenUse => "edit_then_use",
            Self::Unscored => "unscored",
        }
    }
}

/// Per-run scorecard persisted to `~/.rebotica/runs/<id>/scorecard.yml`.
///
/// Written as a stub by [`create_with_id`] and updated by [`update_scorecard`]
/// when Prime records a disposition via `rbtc score`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scorecard {
    pub run_id: String,
    pub project: String,
    pub model: String,
    pub mode: String,
    #[serde(default)]
    pub disposition: Disposition,
    #[serde(default)]
    pub needed_human_correction: Option<bool>,
    #[serde(default)]
    pub tests_passed: Option<bool>,
    #[serde(default)]
    pub diff_lines: Option<u64>,
    #[serde(default)]
    pub files_changed: Option<u64>,
    #[serde(default)]
    pub problems: Vec<String>,
    #[serde(default)]
    pub improvements: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl Scorecard {
    fn stub(run_id: String, project: String, model: String, mode: String) -> Self {
        Self {
            run_id,
            project,
            model,
            mode,
            disposition: Disposition::Unscored,
            needed_human_correction: None,
            tests_passed: None,
            diff_lines: None,
            files_changed: None,
            problems: Vec::new(),
            improvements: Vec::new(),
            rating: None,
            labels: Vec::new(),
            notes: None,
        }
    }
}

pub fn make_id() -> String {
    let stamp = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let suffix = Uuid::new_v4().to_string()[..4].to_string();
    format!("{stamp}-{suffix}")
}

pub fn root() -> PathBuf {
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
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
    let run = create(mode, model, envelope, prompt)?;
    write_model_response(&run, response)?;
    write_parsed_output(
        &run,
        &json!({
            "mode": mode,
            "model": model,
            "response_unparsed": true
        }),
    )?;
    Ok(run)
}

pub fn create(mode: &str, model: &str, envelope: &str, prompt: &str) -> Result<PersistedRun> {
    create_with_id(make_id(), mode, model, envelope, prompt)
}

pub fn create_with_id(
    id: String,
    mode: &str,
    model: &str,
    envelope: &str,
    prompt: &str,
) -> Result<PersistedRun> {
    let directory = runs_root().join(&id);
    fs::create_dir_all(&directory)
        .with_context(|| format!("failed to create {}", directory.display()))?;

    fs::write(directory.join("task-envelope.yml"), envelope)?;
    fs::write(directory.join("prompt.md"), prompt)?;

    let project = std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let stub = Scorecard::stub(id.clone(), project, model.to_string(), mode.to_string());
    write_scorecard_to(&directory, &stub)?;

    let scorecard = root().join("model-scorecards.yml");
    if !scorecard.exists() {
        fs::create_dir_all(root())?;
        fs::write(scorecard, "models: {}\n")?;
    }

    Ok(PersistedRun { id, directory })
}

pub fn write_model_response(run: &PersistedRun, response: &str) -> Result<()> {
    fs::write(run.directory.join("model-response.md"), response)?;
    Ok(())
}

pub fn write_parsed_output<T: Serialize>(run: &PersistedRun, data: &T) -> Result<()> {
    write_json(run, "parsed-output.json", data)
}

pub fn write_parse_failure<T: Serialize>(run: &PersistedRun, details: &T) -> Result<()> {
    write_json(run, "parse-failure.json", details)
}

pub fn write_provider_failure<T: Serialize>(run: &PersistedRun, details: &T) -> Result<()> {
    write_json(run, "provider-failure.json", details)
}

pub fn write_envelope<T: Serialize>(run: &PersistedRun, envelope: &T) -> Result<()> {
    write_json(run, "envelope.json", envelope)
}

fn write_json<T: Serialize>(run: &PersistedRun, name: &str, value: &T) -> Result<()> {
    fs::write(
        run.directory.join(name),
        serde_json::to_string_pretty(value)?,
    )?;
    Ok(())
}

/// Path to the per-run scorecard for `run_id`.
pub fn scorecard_path(run_id: &str) -> PathBuf {
    runs_root().join(run_id).join("scorecard.yml")
}

/// Read the scorecard for a run.
///
/// Returns an error if the run directory or scorecard file does not exist.
/// Older scorecards predating the `Disposition` field still parse (the
/// `disposition` field defaults to [`Disposition::Unscored`]).
pub fn read_scorecard(run_id: &str) -> Result<Scorecard> {
    let path = scorecard_path(run_id);
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

/// Overwrite the scorecard for a run with the provided value.
pub fn write_scorecard(run_id: &str, scorecard: &Scorecard) -> Result<()> {
    write_scorecard_to(&runs_root().join(run_id), scorecard)
}

fn write_scorecard_to(directory: &Path, scorecard: &Scorecard) -> Result<()> {
    let text = serde_yaml::to_string(scorecard).context("failed to serialize scorecard to YAML")?;
    let path = directory.join("scorecard.yml");
    fs::write(&path, text).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Read a scorecard, hand it to `mutate`, and write the result back.
///
/// The closure may modify any field; the returned [`Scorecard`] is the value
/// that was persisted.
pub fn update_scorecard<F>(run_id: &str, mutate: F) -> Result<Scorecard>
where
    F: FnOnce(&mut Scorecard),
{
    let mut scorecard = read_scorecard(run_id)?;
    mutate(&mut scorecard);
    write_scorecard(run_id, &scorecard)?;
    Ok(scorecard)
}

pub fn retrospective_template(run_id: &str) -> String {
    format!(
        "# Retrospective: {run_id}\n\n## What failed?\n\n## What surprised us?\n\n## Was context missing?\n\n## Was the task too broad?\n\n## Did the local model violate scope?\n\n## Did checks catch the issue?\n\n## Should project config change?\n\n## Should prompt, skills, or model routing change?\n\n## Should Prime score this run?\n\n## Should Prime create a comment card for Rebotica?\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{env_lock, EnvGuard, TempDir};
    use serde_json::Value;
    use std::fs;

    #[test]
    fn root_uses_private_state_directory_under_home() {
        let _lock = env_lock();
        let temp = TempDir::new("home-root");
        let _home = EnvGuard::set("HOME", temp.path());

        assert_eq!(root(), temp.path().join(".rebotica"));
        assert_eq!(runs_root(), temp.path().join(".rebotica/runs"));
    }

    #[test]
    fn persist_writes_expected_run_files_and_global_scorecard_seed() {
        let _lock = env_lock();
        let temp = TempDir::new("persist");
        let _home = EnvGuard::set("HOME", temp.path());

        let run = persist(
            "review",
            "test-model",
            "task_id: test\n",
            "# Prompt\n",
            "{\"findings\":[]}\n",
        )
        .unwrap();

        assert!(run
            .directory
            .starts_with(temp.path().join(".rebotica/runs")));
        assert_eq!(
            fs::read_to_string(run.directory.join("task-envelope.yml")).unwrap(),
            "task_id: test\n"
        );
        assert_eq!(
            fs::read_to_string(run.directory.join("prompt.md")).unwrap(),
            "# Prompt\n"
        );
        assert_eq!(
            fs::read_to_string(run.directory.join("model-response.md")).unwrap(),
            "{\"findings\":[]}\n"
        );

        let parsed: Value = serde_json::from_str(
            &fs::read_to_string(run.directory.join("parsed-output.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(parsed["mode"], "review");
        assert_eq!(parsed["model"], "test-model");
        assert_eq!(parsed["response_unparsed"], true);

        let scorecard = read_scorecard(&run.id).unwrap();
        assert_eq!(scorecard.mode, "review");
        assert_eq!(scorecard.model, "test-model");
        assert_eq!(scorecard.disposition, Disposition::Unscored);
        assert_eq!(
            fs::read_to_string(temp.path().join(".rebotica/model-scorecards.yml")).unwrap(),
            "models: {}\n"
        );
    }

    #[test]
    fn update_scorecard_records_disposition_and_round_trips() {
        let _lock = env_lock();
        let temp = TempDir::new("disposition");
        let _home = EnvGuard::set("HOME", temp.path());

        let run = persist(
            "review",
            "test-model",
            "task_id: test\n",
            "# Prompt\n",
            "{}\n",
        )
        .unwrap();

        let updated = update_scorecard(&run.id, |card| {
            card.disposition = Disposition::EditThenUse;
            card.rating = Some(4);
            card.labels = vec!["useful_finding".to_string()];
            card.notes = Some("Tweaked one wording.".to_string());
        })
        .unwrap();
        assert_eq!(updated.disposition, Disposition::EditThenUse);

        let reread = read_scorecard(&run.id).unwrap();
        assert_eq!(reread.disposition, Disposition::EditThenUse);
        assert_eq!(reread.rating, Some(4));
        assert_eq!(reread.labels, vec!["useful_finding".to_string()]);
        assert_eq!(reread.notes.as_deref(), Some("Tweaked one wording."));
    }

    #[test]
    fn legacy_scorecard_without_disposition_field_defaults_to_unscored() {
        let _lock = env_lock();
        let temp = TempDir::new("legacy");
        let _home = EnvGuard::set("HOME", temp.path());

        let run_id = "legacy-run".to_string();
        let directory = runs_root().join(&run_id);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("scorecard.yml"),
            "run_id: legacy-run\nproject: demo\nmodel: m\nmode: review\naccepted: false\nneeded_human_correction: null\ntests_passed: null\ndiff_lines: null\nfiles_changed: null\nproblems: []\nimprovements: []\n",
        )
        .unwrap();

        let scorecard = read_scorecard(&run_id).unwrap();
        assert_eq!(scorecard.disposition, Disposition::Unscored);
        assert_eq!(scorecard.mode, "review");
    }

    #[test]
    fn disposition_serializes_to_snake_case() {
        assert_eq!(
            serde_yaml::to_string(&Disposition::EditThenUse)
                .unwrap()
                .trim(),
            "edit_then_use"
        );
        let parsed: Disposition = serde_yaml::from_str("accept").unwrap();
        assert_eq!(parsed, Disposition::Accept);
    }
}
