use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
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

pub fn retrospective_template(run_id: &str) -> String {
    format!(
        "# Retrospective: {run_id}\n\n## What failed?\n\n## What surprised us?\n\n## Was context missing?\n\n## Was the task too broad?\n\n## Did the local model violate scope?\n\n## Did checks catch the issue?\n\n## Should project config change?\n\n## Should prompt, skills, or model routing change?\n\n## Should Prime score this run?\n\n## Should Prime create a comment card for Rebotica?\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "rebotica-runlog-{name}-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("temp dir should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    struct EnvGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &Path) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.previous {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock should not be poisoned")
    }

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

        let scorecard = fs::read_to_string(run.directory.join("scorecard.yml")).unwrap();
        assert!(scorecard.contains("mode: review"));
        assert!(scorecard.contains("model: test-model"));
        assert_eq!(
            fs::read_to_string(temp.path().join(".rebotica/model-scorecards.yml")).unwrap(),
            "models: {}\n"
        );
    }
}
