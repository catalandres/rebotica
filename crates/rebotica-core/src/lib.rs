use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub mod output;
pub mod run;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProjectConfig {
    #[serde(default)]
    pub project: ProjectInfo,
    #[serde(default)]
    pub commands: Commands,
    #[serde(default)]
    pub forbidden_paths: Vec<String>,
    #[serde(default)]
    pub sensitive_paths: Vec<String>,
    #[serde(default)]
    pub default_limits: Limits,
    #[serde(default)]
    pub providers: Providers,
    #[serde(default)]
    pub models: Models,
    #[serde(default)]
    pub policy: Policy,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProjectInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub r#type: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Commands {
    #[serde(default)]
    pub test: String,
    #[serde(default)]
    pub check: String,
    #[serde(default)]
    pub format_check: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Limits {
    #[serde(default = "default_max_changed_lines")]
    pub max_changed_lines: usize,
    #[serde(default = "default_max_files_changed")]
    pub max_files_changed: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_changed_lines: default_max_changed_lines(),
            max_files_changed: default_max_files_changed(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Providers {
    #[serde(default = "default_provider")]
    pub default: String,
    #[serde(flatten)]
    pub entries: BTreeMap<String, ProviderConfig>,
}

impl Default for Providers {
    fn default() -> Self {
        Self {
            default: default_provider(),
            entries: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProviderConfig {
    #[serde(default = "default_provider_kind")]
    pub kind: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub api_key_env: String,
    #[serde(default)]
    pub api_key_header: String,
    #[serde(default)]
    pub api_key_prefix: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Models {
    #[serde(default)]
    pub default: String,
    #[serde(default)]
    pub review: String,
    #[serde(default)]
    pub explain: String,
    #[serde(default)]
    pub tests: String,
    #[serde(default)]
    pub patch: String,
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Policy {
    #[serde(default)]
    pub allow_dependency_changes: bool,
    #[serde(default)]
    pub allow_generated_files: bool,
    #[serde(default = "default_true")]
    pub patch_requires_review: bool,
    #[serde(default = "default_true")]
    pub patch_requires_worktree: bool,
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub path: Option<PathBuf>,
    pub raw: String,
    pub config: ProjectConfig,
}

impl LoadedConfig {
    pub fn read_from(cwd: &Path) -> Result<Self> {
        let candidates = [cwd.join(".rebotica.yml"), cwd.join(".rebotica/project.yml")];
        for candidate in candidates {
            if candidate.exists() {
                let raw = fs::read_to_string(&candidate)
                    .with_context(|| format!("failed to read {}", candidate.display()))?;
                let config = serde_yaml::from_str(&raw)
                    .with_context(|| format!("failed to parse {}", candidate.display()))?;
                return Ok(Self {
                    path: Some(candidate),
                    raw,
                    config,
                });
            }
        }

        Ok(Self {
            path: None,
            raw: String::new(),
            config: ProjectConfig::default(),
        })
    }

    pub fn raw_or_placeholder(&self) -> &str {
        if self.raw.is_empty() {
            "(none)"
        } else {
            &self.raw
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskEnvelope {
    pub task_id: String,
    pub mode: String,
    pub goal: String,
    pub project_context: String,
    pub allowed_files: Vec<String>,
    pub forbidden_files: Vec<String>,
    pub sensitive_files: Vec<String>,
    pub commands_to_run: Vec<String>,
    pub max_changed_lines: usize,
    pub max_files_changed: usize,
    pub output_format: String,
    pub acceptance_criteria: Vec<String>,
    pub risk_level: String,
}

impl TaskEnvelope {
    pub fn for_config(
        task_id: String,
        mode: impl Into<String>,
        goal: impl Into<String>,
        loaded: &LoadedConfig,
        allowed_files: Vec<String>,
        output_format: impl Into<String>,
        risk_level: impl Into<String>,
    ) -> Self {
        Self {
            task_id,
            mode: mode.into(),
            goal: goal.into(),
            project_context: loaded
                .path
                .as_ref()
                .map(|path| format!("config: {}", path.display()))
                .unwrap_or_else(|| "No project config found.".to_string()),
            allowed_files,
            forbidden_files: loaded.config.forbidden_paths.clone(),
            sensitive_files: loaded.config.sensitive_paths.clone(),
            commands_to_run: Vec::new(),
            max_changed_lines: loaded.config.default_limits.max_changed_lines,
            max_files_changed: loaded.config.default_limits.max_files_changed,
            output_format: output_format.into(),
            acceptance_criteria: vec!["Prime reviews output before acceptance.".to_string()],
            risk_level: risk_level.into(),
        }
    }

    pub fn to_yaml(&self) -> Result<String> {
        serde_yaml::to_string(self).context("failed to serialize task envelope")
    }
}

#[derive(Debug, Clone, Copy)]
pub enum WorkerMode {
    Default,
    Review,
    Explain,
    Tests,
    Patch,
}

impl WorkerMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Review => "review",
            Self::Explain => "explain",
            Self::Tests => "tests",
            Self::Patch => "patch",
        }
    }
}

pub fn model_for_mode(config: &ProjectConfig, mode: WorkerMode) -> Option<String> {
    let selected = match mode {
        WorkerMode::Review if !config.models.review.is_empty() => &config.models.review,
        WorkerMode::Explain if !config.models.explain.is_empty() => &config.models.explain,
        WorkerMode::Tests if !config.models.tests.is_empty() => &config.models.tests,
        WorkerMode::Patch if !config.models.patch.is_empty() => &config.models.patch,
        _ => &config.models.default,
    };

    if selected.is_empty() {
        None
    } else {
        Some(resolve_model_alias(config, selected))
    }
}

pub fn resolve_model_alias(config: &ProjectConfig, selected: &str) -> String {
    config
        .models
        .aliases
        .get(selected)
        .cloned()
        .unwrap_or_else(|| selected.to_string())
}

pub fn parse_allowed_files_from_envelope(text: &str) -> Result<Vec<String>> {
    let value: serde_yaml::Value =
        serde_yaml::from_str(text).context("failed to parse task envelope")?;
    Ok(value
        .get("allowed_files")
        .and_then(|value| value.as_sequence())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default())
}

pub fn parse_forbidden_files_from_envelope(text: &str) -> Result<Vec<String>> {
    let value: serde_yaml::Value =
        serde_yaml::from_str(text).context("failed to parse task envelope")?;
    Ok(value
        .get("forbidden_files")
        .and_then(|value| value.as_sequence())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default())
}

fn default_max_changed_lines() -> usize {
    1000
}

fn default_max_files_changed() -> usize {
    25
}

fn default_provider() -> String {
    "lmstudio".to_string()
}

fn default_provider_kind() -> String {
    "openai-compatible".to_string()
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

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
                "rebotica-core-{name}-{}-{suffix}",
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

    #[test]
    fn read_from_prefers_root_config_over_nested_project_config() {
        let temp = TempDir::new("config-precedence");
        fs::create_dir_all(temp.path().join(".rebotica")).unwrap();
        fs::write(
            temp.path().join(".rebotica.yml"),
            "project:\n  name: root-config\nmodels:\n  default: root-model\n",
        )
        .unwrap();
        fs::write(
            temp.path().join(".rebotica/project.yml"),
            "project:\n  name: nested-config\nmodels:\n  default: nested-model\n",
        )
        .unwrap();

        let loaded = LoadedConfig::read_from(temp.path()).unwrap();

        assert_eq!(loaded.path, Some(temp.path().join(".rebotica.yml")));
        assert_eq!(loaded.config.project.name, "root-config");
        assert_eq!(loaded.config.models.default, "root-model");
    }

    #[test]
    fn read_from_uses_nested_project_config_when_root_config_is_missing() {
        let temp = TempDir::new("nested-config");
        fs::create_dir_all(temp.path().join(".rebotica")).unwrap();
        fs::write(
            temp.path().join(".rebotica/project.yml"),
            "project:\n  name: nested-config\nmodels:\n  default: nested-model\n",
        )
        .unwrap();

        let loaded = LoadedConfig::read_from(temp.path()).unwrap();

        assert_eq!(loaded.path, Some(temp.path().join(".rebotica/project.yml")));
        assert_eq!(loaded.config.project.name, "nested-config");
        assert_eq!(loaded.config.models.default, "nested-model");
    }

    #[test]
    fn read_from_returns_defaults_without_project_config() {
        let temp = TempDir::new("no-config");

        let loaded = LoadedConfig::read_from(temp.path()).unwrap();

        assert_eq!(loaded.path, None);
        assert_eq!(loaded.raw_or_placeholder(), "(none)");
        assert_eq!(loaded.config.providers.default, "lmstudio");
        assert_eq!(loaded.config.default_limits.max_changed_lines, 1000);
        assert_eq!(loaded.config.default_limits.max_files_changed, 25);
    }

    #[test]
    fn model_for_mode_resolves_aliases_and_falls_back_to_default_route() {
        let mut config = ProjectConfig::default();
        config.models.default = "default-model".to_string();
        config.models.review = "review-model".to_string();
        config
            .models
            .aliases
            .insert("default-model".to_string(), "raw-default-model".to_string());
        config
            .models
            .aliases
            .insert("review-model".to_string(), "raw-review-model".to_string());

        assert_eq!(
            model_for_mode(&config, WorkerMode::Review),
            Some("raw-review-model".to_string())
        );
        assert_eq!(
            model_for_mode(&config, WorkerMode::Tests),
            Some("raw-default-model".to_string())
        );
        assert_eq!(resolve_model_alias(&config, "unknown"), "unknown");
    }

    #[test]
    fn model_for_mode_returns_none_when_no_route_is_configured() {
        let config = ProjectConfig::default();

        assert_eq!(model_for_mode(&config, WorkerMode::Default), None);
        assert_eq!(model_for_mode(&config, WorkerMode::Patch), None);
    }

    #[test]
    fn parses_allowed_and_forbidden_files_from_task_envelope() {
        let envelope = r#"
task_id: test
allowed_files:
  - src/lib.rs
  - README.md
  - 42
forbidden_files:
  - .env
  - secrets/
"#;

        assert_eq!(
            parse_allowed_files_from_envelope(envelope).unwrap(),
            vec!["src/lib.rs".to_string(), "README.md".to_string()]
        );
        assert_eq!(
            parse_forbidden_files_from_envelope(envelope).unwrap(),
            vec![".env".to_string(), "secrets/".to_string()]
        );
    }
}
