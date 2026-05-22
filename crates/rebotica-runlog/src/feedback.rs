//! Product-feedback "comment cards" — the shared core behind both the
//! `rbtc comment-card` CLI verbs and the MCP `submit_feedback` tool.
//!
//! A card is a small markdown file with a YAML front-matter header, written
//! under `~/.rebotica/comment-cards/<status>/<id>.md` (status is `pending`,
//! `submitted`, or `dismissed`). Submission creates a GitHub issue via the
//! `gh` CLI, gated on a one-time consent recorded in `~/.rebotica/settings.yml`.
//!
//! This module returns plain `anyhow` errors and module-local data structs;
//! the CLI and MCP surfaces map them to their own error codes and envelopes.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::{make_id, root};

const CARD_STATUSES: [&str; 3] = ["pending", "submitted", "dismissed"];

// ─── Settings / consent ────────────────────────────────────────────────────

/// On-disk settings model (`~/.rebotica/settings.yml`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ReboticaSettings {
    #[serde(default)]
    pub comment_cards: CommentCardSettings,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommentCardSettings {
    #[serde(default)]
    pub github_submit_consent: bool,
    #[serde(default = "default_comment_card_repo")]
    pub default_repo: String,
}

impl Default for CommentCardSettings {
    fn default() -> Self {
        Self {
            github_submit_consent: false,
            default_repo: default_comment_card_repo(),
        }
    }
}

fn default_comment_card_repo() -> String {
    "catalandres/rebotica".to_string()
}

pub fn settings_path() -> PathBuf {
    root().join("settings.yml")
}

pub fn read_settings() -> Result<ReboticaSettings> {
    let path = settings_path();
    if !path.exists() {
        return Ok(ReboticaSettings::default());
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

pub fn write_settings(settings: &ReboticaSettings) -> Result<()> {
    std::fs::create_dir_all(root())?;
    std::fs::write(settings_path(), serde_yaml::to_string(settings)?)?;
    Ok(())
}

/// Current submission consent and the repo a submit would target.
#[derive(Debug, Clone)]
pub struct SubmissionConsent {
    pub allowed: bool,
    pub default_repo: String,
}

pub fn submission_consent() -> Result<SubmissionConsent> {
    let settings = read_settings()?;
    Ok(SubmissionConsent {
        allowed: settings.comment_cards.github_submit_consent,
        default_repo: settings.comment_cards.default_repo,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct ConsentState {
    pub github_submit_consent: bool,
    pub default_repo: String,
    pub settings_path: String,
}

/// Apply consent changes (allow/revoke/default-repo) and persist them.
pub fn configure_consent(
    allow_github: bool,
    revoke_github: bool,
    repo: Option<String>,
) -> Result<ConsentState> {
    let mut settings = read_settings()?;
    if allow_github {
        settings.comment_cards.github_submit_consent = true;
    }
    if revoke_github {
        settings.comment_cards.github_submit_consent = false;
    }
    if let Some(repo) = repo {
        settings.comment_cards.default_repo = repo;
    }
    write_settings(&settings)?;
    Ok(ConsentState {
        github_submit_consent: settings.comment_cards.github_submit_consent,
        default_repo: settings.comment_cards.default_repo,
        settings_path: settings_path().display().to_string(),
    })
}

// ─── Storage ────────────────────────────────────────────────────────────────

pub fn comment_cards_root() -> PathBuf {
    root().join("comment-cards")
}

pub fn status_dir(status: &str) -> PathBuf {
    comment_cards_root().join(status)
}

/// Locate a card by id across all statuses. `Ok(None)` if it doesn't exist.
pub fn find_card(card_id: &str) -> Option<PathBuf> {
    CARD_STATUSES
        .iter()
        .map(|status| status_dir(status).join(format!("{card_id}.md")))
        .find(|path| path.exists())
}

/// Whether a card with `card_id` is currently in the `pending` status.
pub fn pending_exists(card_id: &str) -> bool {
    status_dir("pending").join(format!("{card_id}.md")).exists()
}

pub fn pending_count() -> Result<usize> {
    let dir = status_dir("pending");
    if !dir.exists() {
        return Ok(0);
    }
    Ok(std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("md"))
        .count())
}

#[derive(Debug, Clone, Serialize)]
pub struct CardMove {
    pub card_id: String,
    pub from: String,
    pub to: String,
    pub source_path: String,
    pub target_path: String,
}

fn move_card(card_id: &str, from: &str, to: &str) -> Result<CardMove> {
    let source = status_dir(from).join(format!("{card_id}.md"));
    if !source.exists() {
        return Err(anyhow!("comment card not found in {from}: {card_id}"));
    }
    let target_dir = status_dir(to);
    std::fs::create_dir_all(&target_dir)?;
    let target = target_dir.join(format!("{card_id}.md"));
    std::fs::rename(&source, &target)?;
    Ok(CardMove {
        card_id: card_id.to_string(),
        from: from.to_string(),
        to: to.to_string(),
        source_path: source.display().to_string(),
        target_path: target.display().to_string(),
    })
}

// ─── Card model / rendering ──────────────────────────────────────────────────

pub fn labels(kind: &str, area: &str, source: &str, extra: &[String]) -> Vec<String> {
    let mut labels = vec![
        "comment-card".to_string(),
        format!("kind:{kind}"),
        format!("area:{area}"),
        format!("source:{source}"),
    ];
    labels.extend(extra.iter().cloned());
    labels
}

fn yaml_quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

/// A card's full content, ready to render to its on-disk markdown form.
struct CardDoc<'a> {
    id: &'a str,
    status: &'a str,
    title: &'a str,
    kind: &'a str,
    area: &'a str,
    source: &'a str,
    run_id: Option<&'a str>,
    labels: &'a [String],
    body: &'a str,
}

impl CardDoc<'_> {
    fn to_markdown(&self) -> String {
        let labels_yaml = self
            .labels
            .iter()
            .map(|label| format!("  - {}", yaml_quote(label)))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "---\nid: {}\nstatus: {}\ntitle: {}\nkind: {}\narea: {}\nsource: {}\nrun_id: {}\nlabels:\n{}\n---\n\n# {}\n\n{}\n",
            yaml_quote(self.id),
            yaml_quote(self.status),
            yaml_quote(self.title),
            yaml_quote(self.kind),
            yaml_quote(self.area),
            yaml_quote(self.source),
            self.run_id.map(yaml_quote).unwrap_or_else(|| "null".to_string()),
            labels_yaml,
            self.title,
            self.body
        )
    }
}

/// A single field value read out of a card file's front matter.
pub fn card_field(path: &Path, field: &str) -> Result<Option<String>> {
    let text = std::fs::read_to_string(path)?;
    let prefix = format!("{field}:");
    Ok(text
        .lines()
        .find_map(|line| line.strip_prefix(&prefix).map(str::trim))
        .map(|value| value.trim_matches('"').to_string()))
}

pub fn labels_from_file(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path)?;
    let mut labels = Vec::new();
    let mut in_labels = false;
    for line in text.lines() {
        if line.trim() == "labels:" {
            in_labels = true;
            continue;
        }
        if in_labels {
            if let Some(label) = line.trim().strip_prefix("- ") {
                labels.push(label.trim_matches('"').to_string());
                continue;
            }
            if !line.starts_with(' ') {
                break;
            }
        }
    }
    Ok(labels)
}

// ─── Create ──────────────────────────────────────────────────────────────────

/// A newly written pending card.
#[derive(Debug, Clone, Serialize)]
pub struct CreatedCard {
    pub card_id: String,
    pub status: String,
    pub path: String,
    pub title: String,
    pub kind: String,
    pub area: String,
    pub source: String,
    pub run_id: Option<String>,
    pub labels: Vec<String>,
}

const DEFAULT_BODY: &str = "Describe what happened, what you expected, and any workaround.";

/// Write a new `pending` comment card and return its descriptor.
pub fn create_card(
    kind: &str,
    area: &str,
    source: &str,
    title: &str,
    body: Option<&str>,
    run_id: Option<&str>,
    extra_labels: &[String],
) -> Result<CreatedCard> {
    let id = make_id();
    let card_labels = labels(kind, area, source, extra_labels);
    let body = body.unwrap_or(DEFAULT_BODY);
    let text = CardDoc {
        id: &id,
        status: "pending",
        title,
        kind,
        area,
        source,
        run_id,
        labels: &card_labels,
        body,
    }
    .to_markdown();
    let dir = status_dir("pending");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{id}.md"));
    std::fs::write(&path, text)?;
    Ok(CreatedCard {
        card_id: id,
        status: "pending".to_string(),
        path: path.display().to_string(),
        title: title.to_string(),
        kind: kind.to_string(),
        area: area.to_string(),
        source: source.to_string(),
        run_id: run_id.map(str::to_string),
        labels: card_labels,
    })
}

// ─── List / show ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CardListItem {
    pub status: String,
    pub card_id: String,
    pub title: String,
    pub path: String,
}

pub fn list_cards(status: &str) -> Result<Vec<CardListItem>> {
    let statuses: Vec<&str> = if status == "all" {
        CARD_STATUSES.to_vec()
    } else {
        vec![status]
    };
    let mut cards = Vec::new();
    for status in statuses {
        let dir = status_dir(status);
        if !dir.exists() {
            continue;
        }
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }
            let id = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_default();
            let title = card_field(&path, "title")?.unwrap_or_default();
            cards.push(CardListItem {
                status: status.to_string(),
                card_id: id,
                title,
                path: path.display().to_string(),
            });
        }
    }
    Ok(cards)
}

#[derive(Debug, Clone, Serialize)]
pub struct CardContent {
    pub card_id: String,
    pub status: String,
    pub text: String,
    pub path: String,
}

pub fn read_card(card_id: &str) -> Result<Option<CardContent>> {
    let Some(path) = find_card(card_id) else {
        return Ok(None);
    };
    let status = path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(Some(CardContent {
        card_id: card_id.to_string(),
        status,
        text: std::fs::read_to_string(&path)?,
        path: path.display().to_string(),
    }))
}

/// Dismiss a pending card (move it to the `dismissed` status).
pub fn dismiss_card(card_id: &str) -> Result<CardMove> {
    move_card(card_id, "pending", "dismissed")
}

// ─── Submit (GitHub via gh) ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct SubmittedCard {
    pub card_id: String,
    pub repo: String,
    pub issue_output: String,
    pub r#move: CardMove,
}

/// Submit a pending card to GitHub as an issue via the `gh` CLI, then move it
/// to `submitted`. Callers must check [`submission_consent`] and
/// [`pending_exists`] first to surface consent/not-found errors with their
/// own error codes; this returns an `anyhow` error only for `gh`/IO failures.
pub fn submit_card(card_id: &str, repo_override: Option<String>) -> Result<SubmittedCard> {
    let repo = match repo_override {
        Some(repo) => repo,
        None => submission_consent()?.default_repo,
    };
    let path = status_dir("pending").join(format!("{card_id}.md"));
    if !path.exists() {
        return Err(anyhow!("pending comment card not found: {card_id}"));
    }
    let title = card_field(&path, "title")?
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| format!("Comment card {card_id}"));
    let labels = labels_from_file(&path)?;
    ensure_github_labels(&repo, &labels);

    let mut command = Command::new("gh");
    command
        .args([
            "issue",
            "create",
            "--repo",
            &repo,
            "--title",
            &title,
            "--body-file",
        ])
        .arg(&path);
    for label in &labels {
        command.args(["--label", label]);
    }
    let output = command.output().context("failed to run gh issue create")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow!(if stderr.is_empty() {
            "gh issue create failed".to_string()
        } else {
            stderr
        }));
    }

    let moved = move_card(card_id, "pending", "submitted")?;
    Ok(SubmittedCard {
        card_id: card_id.to_string(),
        repo,
        issue_output: String::from_utf8_lossy(&output.stdout).to_string(),
        r#move: moved,
    })
}

fn ensure_github_labels(repo: &str, labels: &[String]) {
    for label in labels {
        let _ = Command::new("gh")
            .args([
                "label",
                "create",
                label,
                "--repo",
                repo,
                "--color",
                label_color(label),
                "--description",
                "Rebotica comment card label",
                "--force",
            ])
            .output();
    }
}

fn label_color(label: &str) -> &'static str {
    if label == "comment-card" {
        "5319e7"
    } else if label.starts_with("kind:") {
        "e99695"
    } else if label.starts_with("area:") {
        "c2e0c6"
    } else if label.starts_with("source:") {
        "d4c5f9"
    } else {
        "cfd3d7"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{env_lock, EnvGuard, TempDir};

    fn fresh_home(name: &str) -> (TempDir, EnvGuard) {
        let temp = TempDir::new(name);
        let guard = EnvGuard::set("HOME", temp.path());
        (temp, guard)
    }

    #[test]
    fn create_card_writes_pending_with_labels_and_runid() {
        let _lock = env_lock();
        let (_temp, _home) = fresh_home("feedback-create");

        let card = create_card(
            "bug",
            "review",
            "model",
            "diff dropped files",
            Some("only one file surfaced"),
            Some("run-123"),
            &["needs-triage".to_string()],
        )
        .unwrap();

        assert_eq!(card.status, "pending");
        assert_eq!(
            card.labels,
            vec![
                "comment-card",
                "kind:bug",
                "area:review",
                "source:model",
                "needs-triage"
            ]
        );
        let text = std::fs::read_to_string(&card.path).unwrap();
        assert!(text.contains("title: \"diff dropped files\""));
        assert!(text.contains("run_id: \"run-123\""));
        assert!(text.contains("only one file surfaced"));
        assert_eq!(pending_count().unwrap(), 1);
        assert!(pending_exists(&card.card_id));
    }

    #[test]
    fn create_card_defaults_body_and_null_runid() {
        let _lock = env_lock();
        let (_temp, _home) = fresh_home("feedback-defaults");

        let card = create_card("ux", "general", "prime", "t", None, None, &[]).unwrap();
        let text = std::fs::read_to_string(&card.path).unwrap();
        assert!(text.contains("run_id: null"));
        assert!(text.contains(DEFAULT_BODY));
    }

    #[test]
    fn labels_from_file_round_trips_created_card() {
        let _lock = env_lock();
        let (_temp, _home) = fresh_home("feedback-labels");

        let card = create_card("docs", "init", "human", "t", None, None, &[]).unwrap();
        let parsed = labels_from_file(Path::new(&card.path)).unwrap();
        assert_eq!(parsed, card.labels);
        assert_eq!(
            card_field(Path::new(&card.path), "kind")
                .unwrap()
                .as_deref(),
            Some("docs")
        );
    }

    #[test]
    fn consent_defaults_off_and_round_trips() {
        let _lock = env_lock();
        let (_temp, _home) = fresh_home("feedback-consent");

        assert!(!submission_consent().unwrap().allowed);

        let state = configure_consent(true, false, Some("owner/repo".to_string())).unwrap();
        assert!(state.github_submit_consent);
        assert_eq!(state.default_repo, "owner/repo");

        let consent = submission_consent().unwrap();
        assert!(consent.allowed);
        assert_eq!(consent.default_repo, "owner/repo");

        let revoked = configure_consent(false, true, None).unwrap();
        assert!(!revoked.github_submit_consent);
        assert_eq!(
            revoked.default_repo, "owner/repo",
            "repo persists across revoke"
        );
    }

    #[test]
    fn dismiss_moves_card_out_of_pending() {
        let _lock = env_lock();
        let (_temp, _home) = fresh_home("feedback-dismiss");

        let card = create_card("ux", "general", "prime", "t", None, None, &[]).unwrap();
        assert!(pending_exists(&card.card_id));
        let moved = dismiss_card(&card.card_id).unwrap();
        assert_eq!(moved.to, "dismissed");
        assert!(!pending_exists(&card.card_id));
        assert!(find_card(&card.card_id).is_some());
    }
}
