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
    /// `"staged"` (index), or `"range:BASE..HEAD"` for an explicit ref range.
    #[serde(default)]
    pub source: Option<String>,
    /// Optional model alias override. Uses the project's configured route
    /// for `review` if omitted.
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FileTargetsRequest {
    /// Repo-relative file paths the apprentice should consider. At least one.
    pub files: Vec<String>,
    /// Optional model alias override.
    #[serde(default)]
    pub model: Option<String>,
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

#[tool_router]
impl ApprenticeServer {
    #[tool(
        description = "Review a git diff for correctness bugs, behavioral regressions, missing tests, and scope violations. Call this BEFORE writing your own review of the user's changes — the local apprentice produces structured findings with file and line citations and a confidence score. Use 'staged' for indexed changes, 'working-tree' for unstaged, or 'range:BASE..HEAD' for an explicit range."
    )]
    async fn review_diff(
        &self,
        Parameters(req): Parameters<ReviewDiffRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut adapter_args = Vec::new();
        match req.source.as_deref() {
            None | Some("working-tree") => {}
            Some("staged") => adapter_args.push("--cached".to_string()),
            Some(other) if other.starts_with("range:") => {
                adapter_args.push(format!("--range={}", &other["range:".len()..]));
            }
            Some(other) => {
                return Err(McpError::invalid_params(
                    format!(
                        "unknown source '{other}'. Use 'working-tree', 'staged', or 'range:BASE..HEAD'."
                    ),
                    None,
                ));
            }
        }
        if let Some(model) = req.model {
            adapter_args.push("--model".to_string());
            adapter_args.push(model);
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
}

#[tool_handler]
impl ServerHandler for ApprenticeServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(SERVER_NAME, SERVER_VERSION))
            .with_protocol_version(ProtocolVersion::V_2024_11_05)
            .with_instructions(
                "Rebotica apprentice tools. Use `review_diff` before reviewing diffs yourself, \
                 `propose_tests` before writing tests, `explain_files` before modifying \
                 unfamiliar code. Use `health_check` to verify the local provider is reachable. \
                 After acting on apprentice output, call `rbtc score RUN_ID --disposition <accept|reject|edit_then_use>` \
                 to record feedback so the apprentice can learn from real use."
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
