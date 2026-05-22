//! MCP (Model Context Protocol) server for Rebotica's apprentice tools.
//!
//! Exposes four narrow tools to a Prime agent (Claude Code, Codex, etc.):
//! `review_diff`, `propose_tests`, `explain_files`, `health_check`. Each
//! tool calls into [`rebotica_run::dispatch`] for the run.* tools (which
//! handles ledger persistence, schema validation, and per-run artifacts)
//! or hits the configured provider directly for `health_check`.
//!
//! Run via [`serve_stdio`] from the `rebotica-mcp` binary or `rbtc mcp serve`.

use anyhow::{Context, Result};
use chrono::Utc;
use rebotica_core::run::{Registry, RegistryRoots};
use rebotica_run::{
    dispatch, harness_root, provider_settings, ProviderArgs, RunOutcome, RunRequest,
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{
        CallToolResult, Content, Implementation, ProtocolVersion, ServerCapabilities, ServerInfo,
    },
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServerHandler, ServiceExt,
};

const SERVER_NAME: &str = "rebotica-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReviewDiffRequest {
    /// Diff source. One of: `"working-tree"` (unstaged changes, default),
    /// `"staged"` (index), `"base:REF"` (this branch vs merge-base with REF —
    /// use `"base:main"` to review the branch; excludes work REF gained after
    /// the fork), or `"range:BASE..HEAD"` for explicit/advanced ranges.
    #[serde(default)]
    pub source: Option<String>,
    /// Optional model alias override. Uses the project's configured route
    /// for `review` if omitted.
    #[serde(default)]
    pub model: Option<String>,
    /// Maximum number of changed lines the apprentice will accept. Overrides
    /// the project default (built-in default: 1000). Pass when the user has
    /// explicitly asked for a larger review.
    #[serde(default)]
    pub max_lines: Option<usize>,
    /// Maximum number of changed files the apprentice will accept. Overrides
    /// the project default (built-in default: 25).
    #[serde(default)]
    pub max_files: Option<usize>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FileTargetsRequest {
    /// Repo-relative file paths the apprentice should consider. At least one.
    pub files: Vec<String>,
    /// Optional model alias override.
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SubmitFeedbackRequest {
    /// One-line feedback title (required).
    pub title: String,
    /// Detailed body: what happened, what was expected, any workaround.
    #[serde(default)]
    pub body: Option<String>,
    /// Feedback kind: `bug`, `ux`, `docs`, `prompt`, or `roadmap`.
    #[serde(default = "default_feedback_kind")]
    pub kind: String,
    /// Affected product area, e.g. `review`, `tests`, `mcp`, `ledger`, `docs`.
    #[serde(default = "default_feedback_area")]
    pub area: String,
    /// The Rebotica `run_id` this feedback is about, if any.
    #[serde(default)]
    pub run_id: Option<String>,
    /// Extra GitHub labels beyond the auto-applied comment-card set.
    #[serde(default)]
    pub labels: Vec<String>,
    /// Override the configured GitHub repo for this submission.
    #[serde(default)]
    pub repo: Option<String>,
}

fn default_feedback_kind() -> String {
    "ux".to_string()
}

fn default_feedback_area() -> String {
    "general".to_string()
}

#[derive(Clone)]
pub struct ApprenticeServer {
    cwd: PathBuf,
    registry: Arc<Registry>,
    #[allow(dead_code)] // wired through #[tool_handler] trait impl
    tool_router: ToolRouter<Self>,
}

impl ApprenticeServer {
    pub fn new(cwd: PathBuf, registry: Registry) -> Self {
        Self {
            cwd,
            registry: Arc::new(registry),
            tool_router: Self::tool_router(),
        }
    }

    async fn run_mode(
        &self,
        mode: &str,
        adapter_args: Vec<String>,
        command: &str,
    ) -> Result<CallToolResult, McpError> {
        if offline_probe_enabled() {
            return Ok(offline_probe_response(mode, command));
        }
        let request = RunRequest {
            mode: mode.to_string(),
            adapter_args,
            command: command.to_string(),
        };
        let started_at = Utc::now();
        let outcome = dispatch(&self.registry, &self.cwd, request, started_at).await;
        match outcome {
            RunOutcome::Success(success) => {
                let body = serde_json::json!({
                    "run_id": success.run.id,
                    "kind": success.kind,
                    "data": success.data,
                });
                Ok(CallToolResult::success(vec![Content::text(
                    body.to_string(),
                )]))
            }
            RunOutcome::Failure(failure) => {
                let body = serde_json::json!({
                    "run_id": failure.run.as_ref().map(|r| r.id.clone()),
                    "kind": failure.kind,
                    "code": failure.code.as_str(),
                    "message": failure.message,
                    "details": failure.details,
                });
                Err(McpError::internal_error(body.to_string(), None))
            }
        }
    }
}

/// Map a `review_diff` `source` parameter to run-engine adapter args.
///
/// `base:REF` resolves to `--base=REF`, which the engine diffs as
/// `merge-base(REF, HEAD)..HEAD` — the correct "review my branch" semantics
/// that exclude work REF gained after the branch forked (#70). `range:R`
/// passes `R` through literally for callers that want explicit two-dot or
/// other ranges. Returns the error message on an unknown source.
fn review_source_adapter_args(source: Option<&str>) -> Result<Vec<String>, String> {
    match source {
        None | Some("working-tree") => Ok(Vec::new()),
        Some("staged") => Ok(vec!["--cached".to_string()]),
        Some(other) => {
            if let Some(base) = other.strip_prefix("base:") {
                Ok(vec![format!("--base={base}")])
            } else if let Some(range) = other.strip_prefix("range:") {
                Ok(vec![format!("--range={range}")])
            } else {
                Err(format!(
                    "unknown source '{other}'. Use 'working-tree', 'staged', 'base:REF' \
                     (merge-base semantics — e.g. 'base:main' to review this branch), or \
                     'range:BASE..HEAD' for an explicit range."
                ))
            }
        }
    }
}

#[tool_router]
impl ApprenticeServer {
    #[tool(
        description = "Review a git diff for correctness bugs, behavioral regressions, missing tests, and scope violations. Call this BEFORE writing your own review of the user's changes — the local apprentice produces structured findings with file and line citations and a confidence score. Source options: 'working-tree' (unstaged, default), 'staged' (indexed), 'base:REF' (changes on this branch vs the merge-base with REF — use 'base:main' for the common 'review my branch' case; this correctly excludes work that REF gained after the branch forked), or 'range:BASE..HEAD' for explicit/advanced ranges (note: a literal two-dot 'main..HEAD' includes deletions of work main gained after forking — prefer 'base:main' unless you specifically want that). For diffs larger than the built-in defaults (1000 lines / 25 files, overridable via `.rebotica.yml`), pass `max_lines` and/or `max_files` when the user has explicitly asked for a larger review."
    )]
    async fn review_diff(
        &self,
        Parameters(req): Parameters<ReviewDiffRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut adapter_args = review_source_adapter_args(req.source.as_deref())
            .map_err(|message| McpError::invalid_params(message, None))?;
        if let Some(model) = req.model {
            adapter_args.push("--model".to_string());
            adapter_args.push(model);
        }
        if let Some(max_lines) = req.max_lines {
            adapter_args.push(format!("--max-lines={max_lines}"));
        }
        if let Some(max_files) = req.max_files {
            adapter_args.push(format!("--max-files={max_files}"));
        }
        self.run_mode("review", adapter_args, "mcp.review_diff")
            .await
    }

    #[tool(
        description = "Propose focused missing tests for the specified files. Call this BEFORE writing tests yourself — the apprentice surfaces test names, scenarios, and likely-edge-case coverage so you can compose against them rather than duplicating."
    )]
    async fn propose_tests(
        &self,
        Parameters(req): Parameters<FileTargetsRequest>,
    ) -> Result<CallToolResult, McpError> {
        if req.files.is_empty() {
            return Err(McpError::invalid_params(
                "`files` must contain at least one path",
                None,
            ));
        }
        let mut adapter_args = req.files;
        if let Some(model) = req.model {
            adapter_args.push("--model".to_string());
            adapter_args.push(model);
        }
        self.run_mode("tests", adapter_args, "mcp.propose_tests")
            .await
    }

    #[tool(
        description = "Explain the specified files with attention to responsibilities, dependencies, and risks. Call this BEFORE writing your own summary or starting to modify unfamiliar files — the apprentice's analysis frames the code so your follow-up edits are informed."
    )]
    async fn explain_files(
        &self,
        Parameters(req): Parameters<FileTargetsRequest>,
    ) -> Result<CallToolResult, McpError> {
        if req.files.is_empty() {
            return Err(McpError::invalid_params(
                "`files` must contain at least one path",
                None,
            ));
        }
        let mut adapter_args = req.files;
        if let Some(model) = req.model {
            adapter_args.push("--model".to_string());
            adapter_args.push(model);
        }
        self.run_mode("explain", adapter_args, "mcp.explain_files")
            .await
    }

    #[tool(
        description = "Check that the local model provider endpoint is reachable and report which models it currently exposes. Use when delegated calls are failing to determine whether the provider is the cause."
    )]
    async fn health_check(&self) -> Result<CallToolResult, McpError> {
        if offline_probe_enabled() {
            return Ok(offline_probe_response("health_check", "mcp.health_check"));
        }
        let loaded = rebotica_core::LoadedConfig::read_from(&self.cwd)
            .map_err(|e| McpError::internal_error(format!("failed to read config: {e:#}"), None))?;
        let settings = provider_settings(&loaded, ProviderArgs::default()).map_err(|e| {
            McpError::internal_error(format!("failed to resolve provider: {e:#}"), None)
        })?;
        let provider = rebotica_provider::OpenAICompatibleProvider::new(&settings)
            .map_err(|e| McpError::internal_error(format!("provider init: {e:#}"), None))?;
        let body = match provider.models().await {
            Ok(models) => serde_json::json!({
                "provider": settings.name,
                "base_url": settings.base_url,
                "ok": true,
                "model_count": models.len(),
                "models": models,
            }),
            Err(error) => serde_json::json!({
                "provider": settings.name,
                "base_url": settings.base_url,
                "ok": false,
                "error": error.to_string(),
            }),
        };
        Ok(CallToolResult::success(vec![Content::text(
            body.to_string(),
        )]))
    }

    #[tool(
        description = "File product feedback about Rebotica itself — an apprentice that produced wrong/low-value output, a confusing tool result, a missing capability, a docs gap. Writes a structured comment card locally; if the maintainer has enabled GitHub submission (rbtc comment-card consent --allow-github), it also files a labeled GitHub issue. Pass `run_id` when the feedback is about a specific delegated run. Use this instead of silently working around a Rebotica shortcoming, so the harness can improve."
    )]
    async fn submit_feedback(
        &self,
        Parameters(req): Parameters<SubmitFeedbackRequest>,
    ) -> Result<CallToolResult, McpError> {
        use rebotica_runlog::feedback;
        if req.title.trim().is_empty() {
            return Err(McpError::invalid_params(
                "`title` must not be empty or whitespace-only",
                None,
            ));
        }
        // Feedback arriving over MCP is filed by the Prime agent.
        let card = feedback::create_card(
            &req.kind,
            &req.area,
            "prime",
            &req.title,
            req.body.as_deref(),
            req.run_id.as_deref(),
            &req.labels,
        )
        .map_err(|e| McpError::internal_error(format!("failed to write comment card: {e:#}"), None))?;

        let consent = feedback::submission_consent().map_err(|e| {
            McpError::internal_error(format!("failed to read consent: {e:#}"), None)
        })?;

        let body = if consent.allowed {
            match feedback::submit_card(&card.card_id, req.repo) {
                Ok(submitted) => serde_json::json!({
                    "card_id": submitted.card_id,
                    "status": "submitted",
                    "repo": submitted.repo,
                    "issue_output": submitted.issue_output.trim(),
                    "labels": card.labels,
                }),
                // The card is already saved as pending; surface the failure
                // with the id so it isn't lost and can be retried manually.
                Err(error) => {
                    return Err(McpError::internal_error(
                        format!(
                            "comment card {} saved as pending, but GitHub submission failed: {error:#}. \
                             Retry with: rbtc comment-card submit {}",
                            card.card_id, card.card_id
                        ),
                        None,
                    ));
                }
            }
        } else {
            serde_json::json!({
                "card_id": card.card_id,
                "status": "pending",
                "submitted": false,
                "labels": card.labels,
                "note": "GitHub submission consent is off; the card is saved locally. \
                         A maintainer can file it with `rbtc comment-card submit <card_id>` \
                         after `rbtc comment-card consent --allow-github`.",
            })
        };
        Ok(CallToolResult::success(vec![Content::text(
            body.to_string(),
        )]))
    }
}

#[tool_handler]
impl ServerHandler for ApprenticeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, SERVER_VERSION))
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "Rebotica apprentice tools. Call `review_diff` before reviewing any diff yourself \
                 — including code Prime just wrote and feels confident about. The apprentice \
                 is an independent reader and routinely catches gaps the author missed; the cost \
                 is one MCP call against a local model. Call `propose_tests` before writing \
                 tests and `explain_files` before modifying unfamiliar code, for the same \
                 reason. If `review_diff` returns `over_limit`, the diff exceeded the project \
                 default; pass `max_lines` (and/or `max_files`) explicitly when you genuinely \
                 want the larger review. Use `health_check` to verify the local provider is \
                 reachable. After acting on apprentice output (or deciding not to), call \
                 `rbtc score RUN_ID --disposition <accept|reject|edit_then_use>` to record \
                 feedback so the apprentice can learn from real use. When Rebotica itself falls \
                 short — a bad apprentice result, a confusing tool response, a missing capability \
                 — call `submit_feedback` (with the `run_id` when relevant) rather than silently \
                 working around it, so the harness can improve."
                    .to_string(),
            )
    }
}

/// Environment variable that short-circuits tool handlers to return a
/// canned stub instead of calling the provider. Used by
/// `scripts/mcp-eval.sh` to measure tool-invocation rates without
/// burning real provider tokens.
const OFFLINE_PROBE_ENV: &str = "REBOTICA_MCP_OFFLINE_PROBE";

fn offline_probe_enabled() -> bool {
    std::env::var_os(OFFLINE_PROBE_ENV).is_some_and(|value| {
        let value = value.to_string_lossy();
        let trimmed = value.trim().to_ascii_lowercase();
        !matches!(trimmed.as_str(), "" | "0" | "false" | "off" | "no")
    })
}

fn offline_probe_response(mode: &str, command: &str) -> CallToolResult {
    let run_id = rebotica_runlog::make_id();
    let kind = if mode == "health_check" {
        "health_check".to_string()
    } else {
        format!("run.{mode}")
    };
    let body = serde_json::json!({
        "run_id": run_id,
        "kind": kind,
        "command": command,
        "data": {
            "offline_probe": true,
            "note": "REBOTICA_MCP_OFFLINE_PROBE is set; no provider call was made. \
                     This response exists only to measure tool-invocation rates."
        }
    });
    CallToolResult::success(vec![Content::text(body.to_string())])
}

/// Build the registry from harness paths + cwd. Mirrors the CLI's
/// `load_run_registry`.
pub fn load_registry(cwd: &Path) -> Result<Registry> {
    let harness = harness_root().context("failed to resolve harness root")?;
    let builtin = harness.join("prompts/runs.d");
    Registry::load(RegistryRoots {
        project: cwd.join(".rebotica/runs.d"),
        user: rebotica_runlog::root().join("runs.d"),
        common_schema: builtin.join("_common/runs-common.schema.json"),
        builtin,
    })
    .context("failed to load run registry")
}

/// Run the apprentice server over stdio (the transport Claude Code and
/// other MCP clients expect by default). Blocks until the client closes
/// the connection.
pub async fn serve_stdio() -> Result<()> {
    if tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .is_err()
    {
        // A tracing subscriber was already initialized (e.g. by the host
        // process). That's fine — keep going.
    }

    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    let registry = load_registry(&cwd)?;
    let server = ApprenticeServer::new(cwd, registry);
    let service = server
        .serve(stdio())
        .await
        .context("failed to start MCP server")?;
    service
        .waiting()
        .await
        .context("MCP server exited with error")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Foundational tests for the apprentice server. Proposed by run
    //! 20260518-190249-b5d4 via `mcp__rebotica__propose_tests` —
    //! the apprentice's first dispositioned-as-`accept` corpus entry.
    //!
    //! Heavier proposals from that run (provider-failure mocking,
    //! full RunOutcome dispatch, registry-load fault injection) are
    //! intentionally deferred; they need mock infrastructure that
    //! doesn't yet exist in this crate.

    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn unset(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
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

    #[test]
    fn offline_probe_enabled_respects_truthy_values() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _g = EnvGuard::unset(OFFLINE_PROBE_ENV);
        assert!(!offline_probe_enabled(), "unset → false");

        for truthy in ["1", "true", "yes", "on", "TRUE", "Yes"] {
            let _g = EnvGuard::set(OFFLINE_PROBE_ENV, truthy);
            assert!(offline_probe_enabled(), "value {truthy:?} should be truthy");
        }

        for falsy in ["0", "false", "no", "off", ""] {
            let _g = EnvGuard::set(OFFLINE_PROBE_ENV, falsy);
            assert!(!offline_probe_enabled(), "value {falsy:?} should be falsy");
        }
    }

    #[test]
    fn offline_probe_response_has_run_id_and_offline_marker() {
        let result = offline_probe_response("review", "mcp.review_diff");
        // CallToolResult wraps content; extract the text payload.
        let text = match &result.content[0].raw {
            rmcp::model::RawContent::Text(t) => t.text.clone(),
            other => panic!("expected text content, got {other:?}"),
        };
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["kind"], "run.review");
        assert_eq!(body["command"], "mcp.review_diff");
        assert_eq!(body["data"]["offline_probe"], true);
        assert!(
            body["run_id"].as_str().is_some_and(|s| !s.is_empty()),
            "run_id should be present and non-empty"
        );
    }

    #[test]
    fn offline_probe_response_uses_health_check_kind_directly() {
        let result = offline_probe_response("health_check", "mcp.health_check");
        let text = match &result.content[0].raw {
            rmcp::model::RawContent::Text(t) => t.text.clone(),
            other => panic!("expected text content, got {other:?}"),
        };
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            body["kind"], "health_check",
            "health_check should not get the run.* prefix"
        );
    }

    fn build_test_server() -> ApprenticeServer {
        let cwd = std::env::current_dir().unwrap();
        let registry = load_registry(&cwd).expect("workspace registry must load");
        ApprenticeServer::new(cwd, registry)
    }

    #[tokio::test]
    async fn propose_tests_rejects_empty_files() {
        let server = build_test_server();
        let err = server
            .propose_tests(Parameters(FileTargetsRequest {
                files: vec![],
                model: None,
            }))
            .await
            .expect_err("empty files should be rejected");
        let message = format!("{err:?}");
        assert!(
            message.contains("files"),
            "error message should mention `files`: {message}"
        );
    }

    #[tokio::test]
    async fn explain_files_rejects_empty_files() {
        let server = build_test_server();
        let err = server
            .explain_files(Parameters(FileTargetsRequest {
                files: vec![],
                model: None,
            }))
            .await
            .expect_err("empty files should be rejected");
        let message = format!("{err:?}");
        assert!(
            message.contains("files"),
            "error message should mention `files`: {message}"
        );
    }

    #[tokio::test]
    async fn review_diff_rejects_unknown_source() {
        let _lock = ENV_LOCK.lock().unwrap();
        // Force offline probe so we don't accidentally hit a provider if
        // the source check is reordered.
        let _g = EnvGuard::set(OFFLINE_PROBE_ENV, "1");
        let server = build_test_server();
        let err = server
            .review_diff(Parameters(ReviewDiffRequest {
                source: Some("not-a-real-source".to_string()),
                model: None,
                max_lines: None,
                max_files: None,
            }))
            .await
            .expect_err("unknown source should be rejected");
        let message = format!("{err:?}");
        assert!(
            message.contains("not-a-real-source") || message.contains("unknown source"),
            "error message should identify the bad source: {message}"
        );
    }

    #[test]
    fn review_source_maps_base_to_merge_base_semantics() {
        // #70: `base:REF` must resolve to `--base=REF`, which the engine
        // diffs as merge-base(REF, HEAD)..HEAD — not a literal two-dot range.
        assert_eq!(
            review_source_adapter_args(Some("base:main")).unwrap(),
            vec!["--base=main".to_string()]
        );
    }

    #[test]
    fn review_source_maps_known_sources() {
        assert!(review_source_adapter_args(None).unwrap().is_empty());
        assert!(review_source_adapter_args(Some("working-tree"))
            .unwrap()
            .is_empty());
        assert_eq!(
            review_source_adapter_args(Some("staged")).unwrap(),
            vec!["--cached".to_string()]
        );
        assert_eq!(
            review_source_adapter_args(Some("range:abc..HEAD")).unwrap(),
            vec!["--range=abc..HEAD".to_string()]
        );
    }

    #[test]
    fn review_source_rejects_unknown_and_names_base_option() {
        let err = review_source_adapter_args(Some("bogus")).unwrap_err();
        assert!(err.contains("bogus"), "should name the bad source: {err}");
        assert!(
            err.contains("base:"),
            "error should steer callers to base:REF: {err}"
        );
    }

    #[tokio::test]
    async fn submit_feedback_rejects_empty_title() {
        let server = build_test_server();
        let err = server
            .submit_feedback(Parameters(SubmitFeedbackRequest {
                title: "   ".to_string(),
                body: None,
                kind: default_feedback_kind(),
                area: default_feedback_area(),
                run_id: None,
                labels: vec![],
                repo: None,
            }))
            .await
            .expect_err("empty title should be rejected");
        assert!(
            format!("{err:?}").contains("title"),
            "error should mention title"
        );
    }

    #[tokio::test]
    async fn submit_feedback_stages_card_when_consent_off() {
        let _lock = ENV_LOCK.lock().unwrap();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home = std::env::temp_dir().join(format!("rebotica-mcp-fb-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let _g = EnvGuard::set("HOME", home.to_str().unwrap());

        let server = build_test_server();
        let result = server
            .submit_feedback(Parameters(SubmitFeedbackRequest {
                title: "review dropped files".to_string(),
                body: Some("only one file surfaced".to_string()),
                kind: "bug".to_string(),
                area: "review".to_string(),
                run_id: Some("run-9".to_string()),
                labels: vec![],
                repo: None,
            }))
            .await
            .expect("staging should succeed with consent off");

        let text = match &result.content[0].raw {
            rmcp::model::RawContent::Text(t) => t.text.clone(),
            other => panic!("expected text content, got {other:?}"),
        };
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["status"], "pending");
        assert_eq!(body["submitted"], false);
        let card_id = body["card_id"].as_str().expect("card_id present");
        assert!(!card_id.is_empty());
        // The auto-applied comment-card labels are echoed back in the response.
        let labels = body["labels"].as_array().expect("labels present");
        assert!(labels.iter().any(|l| l == "kind:bug"));
        assert!(labels.iter().any(|l| l == "area:review"));
        assert!(labels.iter().any(|l| l == "source:prime"));
        assert!(
            rebotica_runlog::feedback::pending_exists(card_id),
            "card should be written under the temp HOME"
        );

        let _ = std::fs::remove_dir_all(&home);
    }
}
