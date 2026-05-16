use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use rebotica_core::output::{
    CodedCommandError, EmptyData, Envelope, EnvelopeError, ErrorCode, Reporter, ReporterMode,
};
use rebotica_core::{
    model_for_mode, parse_allowed_files_from_envelope, parse_forbidden_files_from_envelope,
    resolve_model_alias, LoadedConfig, ProjectConfig, TaskEnvelope, WorkerMode,
};
use rebotica_provider::{
    ChatMessage, OpenAICompatibleProvider, ProviderOverrides, ProviderSettings,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

#[derive(Debug, Parser)]
#[command(name = "rbtc", version)]
#[command(
    about = "A governed local-model delegation harness for collaborative software craftsmanship."
)]
#[command(after_help = "Command groups:
  Setup and status: init, doctor, providers, models, health, smoke, install
  Delegated work: run review, run explain, run tests, run patch
  Policy and safety: guard-diff
  Skills and prompts: skills
  Feedback and learning: score, scorecards, comment-card, retro

Common workflows:
  rbtc init
  rbtc doctor
  rbtc skills list
  rbtc models --configured-only
  rbtc models configure --detect
  rbtc run review --base main
  rbtc run patch .rebotica/tasks/task.yml --dry-run

Provider setup:
  export REBOTICA_BASE_URL=http://127.0.0.1:1234/v1
  export REBOTICA_MODEL=MODEL_ID")]
struct Cli {
    #[arg(long, global = true, help = "Emit machine-readable JSON envelope.")]
    json: bool,
    #[arg(
        long,
        global = true,
        help = "Suppress stderr; emit only the JSON envelope on stdout. Implies --json."
    )]
    quiet: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Check config, provider routing, git state, and installed adapters.")]
    Doctor(DoctorArgs),
    #[command(about = "Show configured model routes, configure routes, and provider models.")]
    Models(ModelsArgs),
    #[command(about = "Show configured providers, endpoints, and auth environment state.")]
    Providers(ProvidersArgs),
    #[command(about = "Check the selected provider's /models endpoint.")]
    Health(ProviderArgs),
    #[command(about = "Send a tiny chat request to verify a selected model can respond.")]
    Smoke(SmokeArgs),
    #[command(about = "Create .rebotica.yml and local .rebotica/ project state.")]
    Init(InitArgs),
    #[command(about = "Install Claude, Codex, or GitHub adapter assets into this repo.")]
    Install(InstallArgs),
    #[command(about = "Inspect canonical and project-local skills.")]
    Skills(SkillsArgs),
    #[command(about = "Run delegated local-model work modes.")]
    Run(RunArgs),
    #[command(about = "Check a selected git diff against forbidden paths and size limits.")]
    GuardDiff(GuardDiffArgs),
    #[command(about = "Record Prime feedback about a model run.")]
    Score(ScoreArgs),
    #[command(about = "Show accumulated model scorecard summaries.")]
    Scorecards,
    #[command(about = "Create and manage product feedback comment cards.")]
    CommentCard(CommentCardArgs),
    #[command(about = "Create a retrospective template for a saved run.")]
    Retro(RetroArgs),
}

#[derive(Debug, Parser, Clone, Default)]
struct ProviderArgs {
    #[arg(
        long,
        help = "Provider name from config, or an OpenAI-compatible base URL."
    )]
    provider: Option<String>,
    #[arg(
        long,
        help = "Override provider base URL, for example http://127.0.0.1:1234/v1."
    )]
    base_url: Option<String>,
}

#[derive(Debug, Parser)]
struct SmokeArgs {
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(long, help = "Model alias or raw provider model id to smoke test.")]
    model: Option<String>,
    #[arg(
        long,
        default_value_t = 0.0,
        help = "Sampling temperature for the chat request."
    )]
    temperature: f64,
}

#[derive(Debug, Parser)]
struct InitArgs {
    #[arg(
        long,
        help = "Overwrite an existing .rebotica.yml and state .gitignore."
    )]
    force: bool,
}

#[derive(Debug, Parser)]
struct DoctorArgs {
    #[command(flatten)]
    provider: ProviderArgs,
}

#[derive(Debug, Parser)]
struct ModelsArgs {
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(
        long,
        help = "Skip the provider /models request and show configured routes only."
    )]
    configured_only: bool,
    #[command(subcommand)]
    command: Option<ModelsCommand>,
}

#[derive(Debug, Subcommand)]
enum ModelsCommand {
    #[command(about = "Populate model aliases and empty model routes explicitly.")]
    Configure(ModelConfigureArgs),
}

#[derive(Debug, Parser)]
struct ModelConfigureArgs {
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(
        long,
        value_name = "MODEL_ID",
        conflicts_with = "detect",
        help = "Raw provider model id to route through an alias."
    )]
    model: Option<String>,
    #[arg(
        long,
        conflicts_with = "model",
        help = "Inspect the provider /models endpoint and configure only when exactly one model is available."
    )]
    detect: bool,
    #[arg(
        long,
        default_value = "local-model",
        help = "Alias to write under models.aliases and use for empty routes."
    )]
    alias: String,
    #[arg(
        long,
        help = "Replace existing route values and an existing alias target."
    )]
    force: bool,
}

#[derive(Debug, Parser)]
struct ProvidersArgs {}

#[derive(Debug, Parser)]
struct InstallArgs {
    #[arg(
        value_name = "TARGET",
        help = "Adapter target to install: claude, codex, github, or all."
    )]
    target: InstallTarget,
    #[arg(long, help = "Copy assets instead of symlinking them.")]
    copy: bool,
    #[arg(long, help = "Replace existing target files during installation.")]
    force: bool,
    #[arg(
        long,
        value_name = "DIR",
        help = "Install Codex skills into a custom directory."
    )]
    target_dir: Option<String>,
}

#[derive(Debug, Parser)]
struct SkillsArgs {
    #[command(subcommand)]
    command: SkillsCommand,
}

#[derive(Debug, Subcommand)]
enum SkillsCommand {
    #[command(about = "List available canonical and project-local skills.")]
    List(SkillsListArgs),
    #[command(about = "Print a skill exactly as it would be attached to a delegated run.")]
    Show(SkillsShowArgs),
}

#[derive(Debug, Parser)]
struct SkillsListArgs {}

#[derive(Debug, Parser)]
struct SkillsShowArgs {
    #[arg(
        value_name = "SKILL",
        help = "Skill id, or canonical:<id> / project:<id> when disambiguating."
    )]
    skill: String,
}

#[derive(Debug, Parser)]
struct RunArgs {
    #[command(subcommand)]
    mode: RunMode,
}

#[derive(Debug, Subcommand)]
enum RunMode {
    #[command(about = "Ask a local model to review a selected git diff.")]
    Review(ReviewArgs),
    #[command(about = "Ask a local model to explain selected files.")]
    Explain(FileWorkerArgs),
    #[command(about = "Ask a local model to propose focused tests for selected files.")]
    Tests(FileWorkerArgs),
    #[command(about = "Ask a local model for a dry-run unified diff from a task envelope.")]
    Patch(PatchArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum InstallTarget {
    Claude,
    Codex,
    Github,
    All,
}

#[derive(Debug, Parser)]
struct ReviewArgs {
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(
        long = "model",
        value_name = "MODEL",
        help = "Model alias or raw provider model id for review. Repeat to run multiple models side by side."
    )]
    models: Vec<String>,
    #[arg(
        long,
        value_name = "REF",
        conflicts_with_all = ["range", "cached"],
        help = "Review changes from merge-base(REF, HEAD) to HEAD."
    )]
    base: Option<String>,
    #[arg(
        long,
        value_name = "REV_RANGE",
        conflicts_with_all = ["base", "cached"],
        help = "Review an explicit git diff range, for example main..HEAD or main...HEAD."
    )]
    range: Option<String>,
    #[arg(
        long,
        conflicts_with_all = ["base", "range"],
        help = "Review staged changes instead of unstaged working tree changes."
    )]
    cached: bool,
    #[arg(
        long,
        value_name = "COUNT",
        help = "Override max_files_changed in the review task envelope."
    )]
    max_files: Option<usize>,
    #[arg(
        long,
        value_name = "COUNT",
        help = "Override max_changed_lines in the review task envelope."
    )]
    max_lines: Option<usize>,
    #[arg(
        long = "skill",
        value_name = "SKILL",
        help = "Attach a canonical or project-local skill as delegated run context."
    )]
    skills: Vec<String>,
    #[arg(long, help = "Optional review goal to put in the task envelope.")]
    goal: Option<String>,
    #[arg(
        long,
        default_value = "medium",
        help = "Risk level to record in the task envelope."
    )]
    risk: String,
    #[arg(
        long,
        default_value_t = 0.0,
        help = "Sampling temperature for the chat request."
    )]
    temperature: f64,
}

#[derive(Debug, Parser)]
struct FileWorkerArgs {
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(long, help = "Model alias or raw provider model id for this run.")]
    model: Option<String>,
    #[arg(long, help = "Optional goal to put in the task envelope.")]
    goal: Option<String>,
    #[arg(
        long = "skill",
        value_name = "SKILL",
        help = "Attach a canonical or project-local skill as delegated run context."
    )]
    skills: Vec<String>,
    #[arg(
        long,
        default_value_t = 0.0,
        help = "Sampling temperature for the chat request."
    )]
    temperature: f64,
    #[arg(
        value_name = "FILE",
        help = "Project file to include in the model context."
    )]
    files: Vec<String>,
}

#[derive(Debug, Parser)]
struct PatchArgs {
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(
        long,
        help = "Model alias or raw provider model id for patch drafting."
    )]
    model: Option<String>,
    #[arg(
        long,
        default_value_t = 0.0,
        help = "Sampling temperature for the chat request."
    )]
    temperature: f64,
    #[arg(
        long = "skill",
        value_name = "SKILL",
        help = "Attach a canonical or project-local skill as delegated run context."
    )]
    skills: Vec<String>,
    #[arg(
        long,
        help = "Print the proposed diff and run metadata without applying anything."
    )]
    dry_run: bool,
    #[arg(long, help = "Request direct application; currently rejected in v0.1.")]
    apply: bool,
    #[arg(
        value_name = "TASK_ENVELOPE",
        help = "Path to a task-envelope YAML file."
    )]
    envelope: String,
}

#[derive(Debug, Parser)]
struct GuardDiffArgs {
    #[arg(
        long,
        value_name = "REF",
        conflicts_with_all = ["range", "cached"],
        help = "Check changes from merge-base(REF, HEAD) to HEAD."
    )]
    base: Option<String>,
    #[arg(
        long,
        value_name = "REV_RANGE",
        conflicts_with_all = ["base", "cached"],
        help = "Check an explicit git diff range, for example main..HEAD or main...HEAD."
    )]
    range: Option<String>,
    #[arg(
        long,
        conflicts_with_all = ["base", "range"],
        help = "Check staged changes instead of unstaged working tree changes."
    )]
    cached: bool,
    #[arg(long, help = "Override the configured maximum changed file count.")]
    max_files: Option<usize>,
    #[arg(long, help = "Override the configured maximum changed line count.")]
    max_lines: Option<usize>,
}

#[derive(Debug, Parser)]
struct ScoreArgs {
    #[arg(value_name = "RUN_ID", help = "Run id under ~/.rebotica/runs.")]
    run_id: String,
    #[arg(long, value_name = "1-5", help = "Prime rating for the model output.")]
    rating: Option<u8>,
    #[arg(
        long,
        conflicts_with = "rejected",
        help = "Mark the run as accepted/useful."
    )]
    accepted: bool,
    #[arg(
        long,
        conflicts_with = "accepted",
        help = "Mark the run as rejected/not useful."
    )]
    rejected: bool,
    #[arg(
        long = "label",
        value_name = "LABEL",
        help = "Feedback label to attach."
    )]
    labels: Vec<String>,
    #[arg(long, help = "Short Prime feedback notes.")]
    notes: Option<String>,
}

#[derive(Debug, Parser)]
struct CommentCardArgs {
    #[command(subcommand)]
    command: CommentCardCommand,
}

#[derive(Debug, Subcommand)]
enum CommentCardCommand {
    #[command(about = "Create a local product feedback comment card.")]
    New(CommentCardNewArgs),
    #[command(about = "List local comment cards by status.")]
    List(CommentCardListArgs),
    #[command(about = "Print a local comment card.")]
    Show(CommentCardShowArgs),
    #[command(about = "Dismiss a pending comment card.")]
    Dismiss(CommentCardShowArgs),
    #[command(about = "Configure consent for GitHub comment-card submission.")]
    Consent(CommentCardConsentArgs),
    #[command(about = "Submit a pending comment card to GitHub when consent is enabled.")]
    Submit(CommentCardSubmitArgs),
}

#[derive(Debug, Parser)]
struct CommentCardNewArgs {
    #[arg(long, help = "Link this card to a Rebotica run id.")]
    from_run: Option<String>,
    #[arg(
        long,
        default_value = "ux",
        help = "Feedback kind, for example ux, bug, docs, prompt, or roadmap."
    )]
    kind: String,
    #[arg(
        long,
        default_value = "general",
        help = "Affected product area, for example review, init, skills, or docs."
    )]
    area: String,
    #[arg(
        long,
        default_value = "prime",
        help = "Feedback source, for example prime, human, or model."
    )]
    source: String,
    #[arg(long, help = "Comment card title.")]
    title: String,
    #[arg(long, help = "Comment card body text.")]
    body: Option<String>,
    #[arg(long = "label", value_name = "LABEL", help = "Extra label to attach.")]
    labels: Vec<String>,
}

#[derive(Debug, Parser)]
struct CommentCardListArgs {
    #[arg(
        long,
        default_value = "pending",
        help = "Card status to list: pending, submitted, dismissed, or all."
    )]
    status: String,
}

#[derive(Debug, Parser)]
struct CommentCardShowArgs {
    #[arg(value_name = "CARD_ID", help = "Comment card id.")]
    card_id: String,
}

#[derive(Debug, Parser)]
struct CommentCardConsentArgs {
    #[arg(
        long,
        conflicts_with = "revoke_github",
        help = "Allow GitHub submission of comment cards."
    )]
    allow_github: bool,
    #[arg(
        long,
        conflicts_with = "allow_github",
        help = "Revoke GitHub submission consent."
    )]
    revoke_github: bool,
    #[arg(
        long,
        value_name = "OWNER/REPO",
        help = "Default GitHub repo for comment cards."
    )]
    repo: Option<String>,
}

#[derive(Debug, Parser)]
struct CommentCardSubmitArgs {
    #[arg(value_name = "CARD_ID", help = "Comment card id.")]
    card_id: String,
    #[arg(
        long,
        value_name = "OWNER/REPO",
        help = "Override the configured GitHub repo."
    )]
    repo: Option<String>,
}

#[derive(Debug, Parser)]
struct RetroArgs {
    #[arg(long, help = "Overwrite an existing retrospective file.")]
    force: bool,
    #[arg(value_name = "RUN_ID", help = "Run id under ~/.rebotica/runs.")]
    run_id: String,
}

#[tokio::main]
async fn main() {
    let started_at = Utc::now();
    let args: Vec<OsString> = std::env::args_os().collect();

    match Cli::try_parse_from(args.clone()) {
        Ok(cli) => {
            let reporter_mode = reporter_mode_from_cli_and_env(&cli);
            let command_path = command_path(&cli);
            match run_until_done_or_cancelled(cli, reporter_mode, started_at, &command_path).await {
                Ok(code) => std::process::exit(code),
                Err(error) => {
                    let code = error_code_for(&error);
                    if reporter_mode == ReporterMode::Human {
                        eprintln!("rbtc: {error:#}");
                    } else {
                        emit_top_level_error(
                            reporter_mode,
                            &command_path,
                            started_at,
                            code,
                            format!("{error:#}"),
                        );
                    }
                    std::process::exit(code.exit_code());
                }
            }
        }
        Err(error) => {
            let reporter_mode = reporter_mode_from_args_and_env_for_parse_error(&args);
            use clap::error::ErrorKind;
            // Help and version are not errors. clap prints them via error.exit()
            // and returns exit code 0. Bypass the JSON-envelope path so we don't
            // emit a self-contradicting `ok: false` envelope paired with exit 0.
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp
                    | ErrorKind::DisplayVersion
                    | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ) {
                error.exit();
            }
            if reporter_mode == ReporterMode::Human {
                error.exit();
            }
            emit_top_level_error(
                reporter_mode,
                "rbtc",
                started_at,
                ErrorCode::Usage,
                error.to_string(),
            );
            std::process::exit(ErrorCode::Usage.exit_code());
        }
    }
}

async fn run_until_done_or_cancelled(
    cli: Cli,
    reporter_mode: ReporterMode,
    started_at: DateTime<Utc>,
    command_path: &str,
) -> Result<i32> {
    tokio::select! {
        result = run(cli, reporter_mode, started_at) => result,
        signal = tokio::signal::ctrl_c() => {
            signal.context("failed to listen for cancellation signal")?;
            let message = "operation cancelled";
            if reporter_mode == ReporterMode::Human {
                eprintln!("rbtc: {message}");
            } else {
                emit_top_level_error(
                    reporter_mode,
                    command_path,
                    started_at,
                    ErrorCode::Cancelled,
                    message.to_string(),
                );
            }
            Ok(ErrorCode::Cancelled.exit_code())
        }
    }
}

async fn run(cli: Cli, reporter_mode: ReporterMode, started_at: DateTime<Utc>) -> Result<i32> {
    let Some(command) = cli.command else {
        if reporter_mode.is_json() {
            emit_top_level_error(
                reporter_mode,
                "rbtc",
                started_at,
                ErrorCode::Usage,
                "missing subcommand".to_string(),
            );
            return Ok(ErrorCode::Usage.exit_code());
        }
        Cli::command().print_help()?;
        println!();
        return Ok(0);
    };
    match command {
        Command::Doctor(args) => doctor(args, reporter_mode, started_at).await,
        Command::Models(args) => {
            let (kind, command) = if matches!(&args.command, Some(ModelsCommand::Configure(_))) {
                ("models.configure", "models configure")
            } else {
                ("models", "models")
            };
            handle_migrated_result(
                models(args, reporter_mode, started_at).await,
                reporter_mode,
                started_at,
                kind,
                command,
            )
        }
        Command::Providers(args) => handle_migrated_result(
            providers(args, reporter_mode, started_at),
            reporter_mode,
            started_at,
            "providers",
            "providers",
        ),
        Command::Health(args) => {
            health(args).await?;
            Ok(0)
        }
        Command::Smoke(args) => {
            smoke(args).await?;
            Ok(0)
        }
        Command::Init(args) => handle_migrated_result(
            init_project(args, reporter_mode, started_at),
            reporter_mode,
            started_at,
            "init",
            "init",
        ),
        Command::Install(args) => handle_migrated_result(
            install(args, reporter_mode, started_at),
            reporter_mode,
            started_at,
            "install",
            "install",
        ),
        Command::Skills(args) => {
            let (kind, command) = match &args.command {
                SkillsCommand::List(_) => ("skills.list", "skills list"),
                SkillsCommand::Show(_) => ("skills.show", "skills show"),
            };
            handle_migrated_result(
                skills(args, reporter_mode, started_at),
                reporter_mode,
                started_at,
                kind,
                command,
            )
        }
        Command::Run(args) => {
            run_mode(args).await?;
            Ok(0)
        }
        Command::GuardDiff(args) => {
            guard_diff(args)?;
            Ok(0)
        }
        Command::Score(args) => handle_migrated_result(
            score(args, reporter_mode, started_at),
            reporter_mode,
            started_at,
            "score",
            "score",
        ),
        Command::Scorecards => handle_migrated_result(
            scorecards(reporter_mode, started_at),
            reporter_mode,
            started_at,
            "scorecards",
            "scorecards",
        ),
        Command::CommentCard(args) => {
            let (kind, command) = match &args.command {
                CommentCardCommand::New(_) => ("comment-card.new", "comment-card new"),
                CommentCardCommand::List(_) => ("comment-card.list", "comment-card list"),
                CommentCardCommand::Show(_) => ("comment-card.show", "comment-card show"),
                CommentCardCommand::Dismiss(_) => ("comment-card.dismiss", "comment-card dismiss"),
                CommentCardCommand::Consent(_) => ("comment-card.consent", "comment-card consent"),
                CommentCardCommand::Submit(_) => ("comment-card.submit", "comment-card submit"),
            };
            handle_migrated_result(
                comment_card(args, reporter_mode, started_at),
                reporter_mode,
                started_at,
                kind,
                command,
            )
        }
        Command::Retro(args) => handle_migrated_result(
            retrospective(args, reporter_mode, started_at),
            reporter_mode,
            started_at,
            "retro",
            "retro",
        ),
    }
}

fn reporter_mode_from_cli_and_env(cli: &Cli) -> ReporterMode {
    ReporterMode::from_flags(
        cli.json || env_truthy("REBOTICA_JSON"),
        cli.quiet || env_truthy("REBOTICA_QUIET"),
    )
}

/// Detects output mode after clap parsing fails so parse/setup errors can still emit envelopes.
///
/// This intentionally uses a simple token scan; an argument value literally equal to `--json`
/// or `--quiet` can be mistaken for a global output flag before clap knows the command shape.
/// Successful parses use clap's resolved global flags instead, so normal argument values are not
/// misclassified.
fn reporter_mode_from_args_and_env_for_parse_error(args: &[OsString]) -> ReporterMode {
    let mut json = env_truthy("REBOTICA_JSON");
    let mut quiet = env_truthy("REBOTICA_QUIET");
    for arg in args.iter().skip(1).filter_map(|arg| arg.to_str()) {
        match arg {
            "--json" => json = true,
            "--quiet" => quiet = true,
            _ => {}
        }
    }
    ReporterMode::from_flags(json, quiet)
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

fn command_path(cli: &Cli) -> String {
    match &cli.command {
        Some(Command::Doctor(_)) => "doctor",
        Some(Command::Models(args)) => match &args.command {
            Some(ModelsCommand::Configure(_)) => "models configure",
            None => "models",
        },
        Some(Command::Providers(_)) => "providers",
        Some(Command::Health(_)) => "health",
        Some(Command::Smoke(_)) => "smoke",
        Some(Command::Init(_)) => "init",
        Some(Command::Install(_)) => "install",
        Some(Command::Skills(args)) => match &args.command {
            SkillsCommand::List(_) => "skills list",
            SkillsCommand::Show(_) => "skills show",
        },
        Some(Command::Run(args)) => match &args.mode {
            RunMode::Review(_) => "run review",
            RunMode::Explain(_) => "run explain",
            RunMode::Tests(_) => "run tests",
            RunMode::Patch(_) => "run patch",
        },
        Some(Command::GuardDiff(_)) => "guard-diff",
        Some(Command::Score(_)) => "score",
        Some(Command::Scorecards) => "scorecards",
        Some(Command::CommentCard(args)) => match &args.command {
            CommentCardCommand::New(_) => "comment-card new",
            CommentCardCommand::List(_) => "comment-card list",
            CommentCardCommand::Show(_) => "comment-card show",
            CommentCardCommand::Dismiss(_) => "comment-card dismiss",
            CommentCardCommand::Consent(_) => "comment-card consent",
            CommentCardCommand::Submit(_) => "comment-card submit",
        },
        Some(Command::Retro(_)) => "retro",
        None => "rbtc",
    }
    .to_string()
}

fn emit_top_level_error(
    reporter_mode: ReporterMode,
    command: &str,
    started_at: DateTime<Utc>,
    code: ErrorCode,
    message: String,
) {
    let mut reporter = Reporter::from_mode(reporter_mode);
    let envelope = Envelope::builder("error")
        .command(command)
        .started_at(started_at)
        .error(EnvelopeError {
            code,
            message,
            details: None,
        })
        .build();
    let _ = reporter.emit(&envelope);
}

fn emit_success<T: Serialize>(
    reporter: &mut Reporter,
    kind: &'static str,
    command: &'static str,
    started_at: DateTime<Utc>,
    data: &T,
) -> Result<()> {
    if reporter.is_json() {
        let envelope = Envelope::builder(kind)
            .command(command)
            .started_at(started_at)
            .data(data)
            .build();
        reporter.emit(&envelope)?;
    }
    Ok(())
}

fn emit_failure<T: Serialize>(
    reporter: &mut Reporter,
    kind: &'static str,
    command: &'static str,
    started_at: DateTime<Utc>,
    data: &T,
    code: ErrorCode,
    message: impl Into<String>,
) -> Result<()> {
    if reporter.is_json() {
        let envelope = Envelope::builder(kind)
            .command(command)
            .started_at(started_at)
            .data(data)
            .error(EnvelopeError {
                code,
                message: message.into(),
                details: None,
            })
            .build();
        reporter.emit(&envelope)?;
    }
    Ok(())
}

fn coded_error(code: ErrorCode, message: impl Into<String>) -> anyhow::Error {
    CodedCommandError::new(code, message).into()
}

fn with_error_code<T>(result: Result<T>, code: ErrorCode) -> Result<T> {
    result.map_err(|error| {
        // Preserve typed inner failures; outer context should not collapse a
        // more specific producer code into a generic wrapper code.
        if error.downcast_ref::<CodedCommandError>().is_some() {
            error
        } else {
            coded_error(code, format!("{error:#}"))
        }
    })
}

fn error_code_for(error: &anyhow::Error) -> ErrorCode {
    error
        .downcast_ref::<CodedCommandError>()
        .map(CodedCommandError::code)
        .unwrap_or(ErrorCode::Internal)
}

fn handle_migrated_result(
    result: Result<i32>,
    reporter_mode: ReporterMode,
    started_at: DateTime<Utc>,
    kind: &'static str,
    command: &'static str,
) -> Result<i32> {
    match result {
        Ok(code) => Ok(code),
        Err(error) if reporter_mode.is_json() => {
            let message = format!("{error:#}");
            let code = error_code_for(&error);
            let mut reporter = Reporter::from_mode(reporter_mode);
            let envelope = Envelope::builder(kind)
                .command(command)
                .started_at(started_at)
                .error(EnvelopeError {
                    code,
                    message,
                    details: None,
                })
                .data(EmptyData)
                .build();
            reporter.emit(&envelope)?;
            Ok(code.exit_code())
        }
        Err(error) => Err(error),
    }
}

async fn run_mode(args: RunArgs) -> Result<()> {
    match args.mode {
        RunMode::Review(args) => review(args).await,
        RunMode::Explain(args) => explain(args).await,
        RunMode::Tests(args) => propose_tests(args).await,
        RunMode::Patch(args) => propose_patch(args).await,
    }
}

async fn doctor(
    args: DoctorArgs,
    reporter_mode: ReporterMode,
    started_at: DateTime<Utc>,
) -> Result<i32> {
    let mut reporter = Reporter::from_mode(reporter_mode);
    let cwd = std::env::current_dir()?;
    let mut checks = Vec::new();

    let harness = harness_root();
    checks.push(Check::from_result(
        "harness.root",
        "Rebotica harness root is discoverable",
        harness.as_ref().ok().map(|path| path.display().to_string()),
        &harness,
    ));

    let loaded = match LoadedConfig::read_from(&cwd) {
        Ok(loaded) => {
            checks.push(Check::ok(
                "config.parse",
                "Project config parses",
                loaded
                    .path
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "no project config found; using defaults".to_string()),
            ));
            loaded
        }
        Err(error) => {
            checks.push(Check::fail(
                "config.parse",
                "Project config parses",
                error.to_string(),
            ));
            LoadedConfig {
                path: None,
                raw: String::new(),
                config: ProjectConfig::default(),
            }
        }
    };

    checks.extend(validate_config(&loaded));

    let provider = provider_settings(&loaded, args.provider.clone());
    checks.push(Check::from_result(
        "provider.resolve",
        "Provider selection resolves",
        provider
            .as_ref()
            .ok()
            .map(|settings| format!("{} -> {}", settings.name, settings.base_url)),
        &provider,
    ));

    for mode in [
        WorkerMode::Default,
        WorkerMode::Review,
        WorkerMode::Explain,
        WorkerMode::Tests,
        WorkerMode::Patch,
    ] {
        let model = model_for_mode(&loaded.config, mode);
        let id = format!("model.{}", mode.as_str());
        if let Some(model) = model {
            checks.push(Check::ok(&id, "Model route resolves", model));
        } else {
            checks.push(Check::warn(
                &id,
                "Model route resolves",
                "missing; run rbtc models configure --detect, configure models.default, or pass --model",
            ));
        }
    }

    checks.push(match rebotica_git::assert_repository() {
        Ok(()) => Check::ok(
            "git.repository",
            "Current directory is a git repository",
            "yes",
        ),
        Err(error) => Check::warn(
            "git.repository",
            "Current directory is a git repository",
            error.to_string(),
        ),
    });

    checks.push(installed_check(
        "install.claude",
        ".claude/commands",
        "Claude commands installed",
    ));
    checks.push(installed_any_check(
        "install.codex",
        &[".agents/skills", ".rebotica/adapters/codex/skills"],
        "Codex/agent skills installed",
    ));
    checks.push(installed_check(
        "install.github",
        ".github",
        "GitHub assets installed",
    ));
    let settings = read_settings().unwrap_or_default();
    if settings.comment_cards.github_submit_consent {
        checks.push(Check::ok(
            "comment_cards.consent",
            "Comment-card GitHub submission consent",
            settings.comment_cards.default_repo,
        ));
    } else {
        checks.push(Check::warn(
            "comment_cards.consent",
            "Comment-card GitHub submission consent",
            "not enabled; run rbtc comment-card consent --allow-github when ready",
        ));
    }
    let pending_cards = pending_comment_card_count().unwrap_or(0);
    if pending_cards == 0 {
        checks.push(Check::ok(
            "comment_cards.pending",
            "Pending comment cards",
            "0",
        ));
    } else {
        checks.push(Check::warn(
            "comment_cards.pending",
            "Pending comment cards",
            format!("{pending_cards}; run rbtc comment-card list"),
        ));
    }

    let failed = checks.iter().any(|check| check.status == "fail");
    if reporter.is_json() {
        let mut builder = Envelope::builder("doctor")
            .command("doctor")
            .started_at(started_at)
            .data(&checks);
        if failed {
            builder = builder.error(EnvelopeError {
                code: ErrorCode::Config,
                message: "doctor found failing checks".to_string(),
                details: None,
            });
        }
        let envelope = builder.build();
        reporter.emit(&envelope)?;
    } else {
        for check in &checks {
            reporter.human(&format!(
                "{:<5} {:<24} {}{}",
                check.status,
                check.id,
                check.message,
                check
                    .detail
                    .as_ref()
                    .map(|detail| format!(" ({detail})"))
                    .unwrap_or_default()
            ))?;
        }
    }

    if failed {
        if reporter.is_json() {
            Ok(ErrorCode::Config.exit_code())
        } else {
            Err(coded_error(
                ErrorCode::Config,
                "doctor found failing checks",
            ))
        }
    } else {
        Ok(0)
    }
}

#[derive(Debug, Clone, Serialize)]
struct ModelsData {
    configured: ModelRoutesData,
    provider: Option<ProviderModelsData>,
}

#[derive(Debug, Clone, Serialize)]
struct ModelRoutesData {
    default: String,
    review: String,
    explain: String,
    tests: String,
    patch: String,
    aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderModelsData {
    provider: String,
    base_url: String,
    models: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ProvidersData {
    default: String,
    providers: Vec<ProviderItemData>,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderItemData {
    name: String,
    kind: String,
    base_url: String,
    api_key_env: String,
    api_key_present: bool,
    headers_count: usize,
    implicit: bool,
}

async fn models(
    args: ModelsArgs,
    reporter_mode: ReporterMode,
    started_at: DateTime<Utc>,
) -> Result<i32> {
    if let Some(command) = args.command {
        return match command {
            ModelsCommand::Configure(configure_args) => {
                configure_models(configure_args, reporter_mode, started_at).await
            }
        };
    }

    let mut reporter = Reporter::from_mode(reporter_mode);
    let loaded = with_error_code(
        LoadedConfig::read_from(&std::env::current_dir()?),
        ErrorCode::Config,
    )?;
    let routes = model_routes_data(&loaded.config);

    let provider_models = if args.configured_only {
        None
    } else {
        let settings =
            with_error_code(provider_settings(&loaded, args.provider), ErrorCode::Config)?;
        let provider = OpenAICompatibleProvider::new(&settings)?;
        Some(ProviderModelsData {
            provider: settings.name,
            base_url: settings.base_url,
            models: provider
                .models()
                .await
                .map_err(|error| coded_error(ErrorCode::ProviderUnavailable, error.to_string()))?,
        })
    };

    let data = ModelsData {
        configured: routes,
        provider: provider_models,
    };

    if reporter.is_json() {
        emit_success(&mut reporter, "models", "models", started_at, &data)?;
    } else {
        reporter.human("Configured routes:")?;
        print_model_route(
            &mut reporter,
            "default",
            &loaded.config.models.default,
            &loaded.config,
        )?;
        print_model_route(
            &mut reporter,
            "review",
            &loaded.config.models.review,
            &loaded.config,
        )?;
        print_model_route(
            &mut reporter,
            "explain",
            &loaded.config.models.explain,
            &loaded.config,
        )?;
        print_model_route(
            &mut reporter,
            "tests",
            &loaded.config.models.tests,
            &loaded.config,
        )?;
        print_model_route(
            &mut reporter,
            "patch",
            &loaded.config.models.patch,
            &loaded.config,
        )?;
        if !loaded.config.models.aliases.is_empty() {
            reporter.human("\nAliases:")?;
            for (alias, target) in &loaded.config.models.aliases {
                reporter.human(&format!("  {alias} -> {target}"))?;
            }
        }
        if let Some(provider_models) = &data.provider {
            reporter.human(&format!(
                "\nProvider models ({}):",
                provider_models.provider
            ))?;
            for model in &provider_models.models {
                reporter.human(&format!("  {model}"))?;
            }
        }
    }
    Ok(0)
}

async fn configure_models(
    args: ModelConfigureArgs,
    reporter_mode: ReporterMode,
    started_at: DateTime<Utc>,
) -> Result<i32> {
    let mut reporter = Reporter::from_mode(reporter_mode);
    let loaded = with_error_code(
        LoadedConfig::read_from(&std::env::current_dir()?),
        ErrorCode::Config,
    )?;
    let Some(config_path) = loaded.path.clone() else {
        return Err(coded_error(
            ErrorCode::Config,
            "no project config found. Run rbtc init before configuring model routes.",
        ));
    };

    if args.model.is_none() && !args.detect {
        return Err(coded_error(
            ErrorCode::Usage,
            "pass --model MODEL_ID to configure manually, or --detect to inspect the provider.",
        ));
    }

    let alias = normalize_model_alias(&args.alias)
        .map_err(|error| coded_error(ErrorCode::Usage, error.to_string()))?;
    let report = if let Some(model) = args.model {
        let model = normalize_model_id(&model)
            .map_err(|error| coded_error(ErrorCode::Usage, error.to_string()))?;
        let update = with_error_code(
            write_model_routes(&config_path, &alias, &model, args.force),
            ErrorCode::Config,
        )?;
        ModelConfigureReport::Configured {
            source: "manual".to_string(),
            provider: None,
            base_url: None,
            update,
        }
    } else {
        let settings = match provider_settings(&loaded, args.provider) {
            Ok(settings) => settings,
            Err(error) => {
                let report = ModelConfigureReport::ProviderUnavailable {
                    provider: None,
                    base_url: None,
                    error: error.to_string(),
                    next_step: model_configure_next_step(),
                };
                return finish_model_configure_report(
                    &mut reporter,
                    started_at,
                    &report,
                    Some(ErrorCode::Config),
                );
            }
        };
        let provider = OpenAICompatibleProvider::new(&settings)?;
        match choose_model_from_detection(provider.models().await.map_err(|error| error.to_string()))
        {
            DetectedModelChoice::One(model) => {
                let update = with_error_code(
                    write_model_routes(&config_path, &alias, &model, args.force),
                    ErrorCode::Config,
                )?;
                ModelConfigureReport::Configured {
                    source: "detected".to_string(),
                    provider: Some(settings.name),
                    base_url: Some(settings.base_url),
                    update,
                }
            }
            DetectedModelChoice::ProviderUnavailable(error) => {
                ModelConfigureReport::ProviderUnavailable {
                    provider: Some(settings.name),
                    base_url: Some(settings.base_url),
                    error,
                    next_step: model_configure_next_step(),
                }
            }
            DetectedModelChoice::NoModels => ModelConfigureReport::NoModels {
                provider: settings.name,
                base_url: settings.base_url,
                next_step: model_configure_next_step(),
            },
            DetectedModelChoice::Multiple(models) => ModelConfigureReport::MultipleModels {
                provider: settings.name,
                base_url: settings.base_url,
                models,
                next_step: format!(
                    "Choose one model and run rbtc models configure --model MODEL_ID --alias {alias}"
                ),
            },
        }
    };

    let error_code = match &report {
        ModelConfigureReport::Configured { .. } => None,
        ModelConfigureReport::ProviderUnavailable { .. } => Some(ErrorCode::ProviderUnavailable),
        ModelConfigureReport::NoModels { .. } | ModelConfigureReport::MultipleModels { .. } => {
            Some(ErrorCode::ProviderUnavailable)
        }
    };
    finish_model_configure_report(&mut reporter, started_at, &report, error_code)
}

fn finish_model_configure_report(
    reporter: &mut Reporter,
    started_at: DateTime<Utc>,
    report: &ModelConfigureReport,
    error_code: Option<ErrorCode>,
) -> Result<i32> {
    print_model_configure_report(reporter, &report)?;
    let data = ModelConfigureData::from(report);
    if let Some(code) = error_code {
        emit_failure(
            reporter,
            "models.configure",
            "models configure",
            started_at,
            &data,
            code,
            data.error_message(),
        )?;
        Ok(if reporter.is_json() {
            code.exit_code()
        } else {
            0
        })
    } else {
        emit_success(
            reporter,
            "models.configure",
            "models configure",
            started_at,
            &data,
        )?;
        Ok(0)
    }
}

fn providers(
    _args: ProvidersArgs,
    reporter_mode: ReporterMode,
    started_at: DateTime<Utc>,
) -> Result<i32> {
    let mut reporter = Reporter::from_mode(reporter_mode);
    let loaded = with_error_code(
        LoadedConfig::read_from(&std::env::current_dir()?),
        ErrorCode::Config,
    )?;
    let summary = provider_summary(&loaded.config);
    if reporter.is_json() {
        emit_success(
            &mut reporter,
            "providers",
            "providers",
            started_at,
            &summary,
        )?;
    } else {
        reporter.human(&format!(
            "Default provider: {}",
            loaded.config.providers.default
        ))?;
        for item in &summary.providers {
            let auth = if item.api_key_env.is_empty() {
                "no api key env".to_string()
            } else if item.api_key_present {
                format!("{} present", item.api_key_env)
            } else {
                format!("{} missing", item.api_key_env)
            };
            reporter.human(&format!(
                "  {}: {} {} ({auth})",
                item.name, item.kind, item.base_url
            ))?;
        }
    }
    Ok(0)
}

async fn health(args: ProviderArgs) -> Result<()> {
    let loaded = LoadedConfig::read_from(&std::env::current_dir()?)?;
    let settings = provider_settings(&loaded, args)?;
    let provider = OpenAICompatibleProvider::new(&settings)?;
    let models = provider.models().await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "provider": settings.name,
            "base_url": settings.base_url,
            "model_count": models.len(),
            "models": models
        }))?
    );
    Ok(())
}

async fn smoke(args: SmokeArgs) -> Result<()> {
    let loaded = LoadedConfig::read_from(&std::env::current_dir()?)?;
    let model = resolve_model(&loaded, WorkerMode::Default, args.model)?;
    let settings = provider_settings(&loaded, args.provider)?;
    let provider = OpenAICompatibleProvider::new(&settings)?;
    let text = provider
        .chat(
            &model,
            vec![
                ChatMessage::new("system", "Reply exactly with LOCAL_OK and no other text."),
                ChatMessage::new("user", "Reply with LOCAL_OK only."),
            ],
            args.temperature,
        )
        .await?;
    println!("{}", text.trim());
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct InitData {
    written: Vec<String>,
    skipped: Vec<String>,
    model_routes_empty: bool,
    next_steps: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct InstallData {
    target: String,
    actions: Vec<InstallActionData>,
}

#[derive(Debug, Clone, Serialize)]
struct InstallActionData {
    action: String,
    subject: String,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
struct SkillsListData {
    skills: Vec<SkillInfo>,
}

#[derive(Debug, Clone, Serialize)]
struct SkillsShowData {
    skill: SkillInfo,
    rendered: String,
}

fn init_project(
    args: InitArgs,
    reporter_mode: ReporterMode,
    started_at: DateTime<Utc>,
) -> Result<i32> {
    let mut reporter = Reporter::from_mode(reporter_mode);
    let cwd = std::env::current_dir()?;
    let data = init_project_at(&cwd, args.force, None)?;
    if reporter.is_json() {
        emit_success(&mut reporter, "init", "init", started_at, &data)?;
    } else {
        print_init_report(&mut reporter, &data)?;
    }
    Ok(0)
}

fn init_project_at(cwd: &Path, force: bool, template_override: Option<&str>) -> Result<InitData> {
    let config_path = cwd.join(".rebotica.yml");
    let state_dir = cwd.join(".rebotica");
    if config_path.exists() && !force {
        return Err(coded_error(
            ErrorCode::Config,
            ".rebotica.yml already exists. Use --force to overwrite.",
        ));
    }

    let task_dir = state_dir.join("tasks");
    let runs_dir = state_dir.join("runs");
    let state_ignore = state_dir.join(".gitignore");
    let paths = [
        config_path.clone(),
        task_dir.clone(),
        runs_dir.clone(),
        state_ignore.clone(),
    ];
    let existed = paths
        .iter()
        .map(|path| (path.display().to_string(), path.exists()))
        .collect::<BTreeMap<_, _>>();

    ensure_dir(&task_dir)?;
    ensure_dir(&runs_dir)?;

    let project_name = cwd
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "project".to_string());
    let raw_template = match template_override {
        Some(template) => template.to_string(),
        None => read_harness_file("templates/project.rebotica.yml")?,
    };
    let template = raw_template.replace("name: example-project", &format!("name: {project_name}"));
    fs::write(&config_path, template)?;

    if !state_ignore.exists() || force {
        fs::write(&state_ignore, "runs/\n")?;
    }

    let loaded = LoadedConfig::read_from(cwd)?;
    let model_routes_empty = model_routes_empty(&loaded.config);
    let mut written = Vec::new();
    let mut skipped = Vec::new();
    for path in paths {
        let display = path.display().to_string();
        if force || !existed.get(&display).copied().unwrap_or(false) {
            written.push(display);
        } else {
            skipped.push(display);
        }
    }
    Ok(InitData {
        written,
        skipped,
        model_routes_empty,
        next_steps: if model_routes_empty {
            vec![
                "rbtc models configure --detect".to_string(),
                "rbtc models configure --model MODEL_ID".to_string(),
            ]
        } else {
            Vec::new()
        },
    })
}

fn print_init_report(reporter: &mut Reporter, data: &InitData) -> Result<()> {
    for path in &data.written {
        reporter.human(&format!("created {path}"))?;
    }
    for path in &data.skipped {
        reporter.human(&format!("created {path}"))?;
    }
    if data.model_routes_empty {
        reporter.human("")?;
        reporter.human("model routes are empty.")?;
        reporter.human("next: rbtc models configure --detect")?;
        reporter.human("or:   rbtc models configure --model MODEL_ID")?;
    }
    Ok(())
}

fn install(
    args: InstallArgs,
    reporter_mode: ReporterMode,
    started_at: DateTime<Utc>,
) -> Result<i32> {
    let mut reporter = Reporter::from_mode(reporter_mode);
    let target = args.target;
    let data = match target {
        InstallTarget::Claude => InstallData {
            target: "claude".to_string(),
            actions: install_claude(args.copy, args.force)?,
        },
        InstallTarget::Codex => InstallData {
            target: "codex".to_string(),
            actions: install_codex(args.copy, args.force, args.target_dir)?,
        },
        InstallTarget::Github => InstallData {
            target: "github".to_string(),
            actions: install_github(args.force)?,
        },
        InstallTarget::All => {
            let mut actions = install_claude(args.copy, args.force)?;
            actions.extend(install_codex(args.copy, args.force, args.target_dir)?);
            actions.extend(install_github(args.force)?);
            InstallData {
                target: "all".to_string(),
                actions,
            }
        }
    };
    if reporter.is_json() {
        emit_success(&mut reporter, "install", "install", started_at, &data)?;
    } else {
        reporter.human(&format!("Installed {} adapter assets:", data.target))?;
        for action in &data.actions {
            reporter.human(&format!(
                "  {} {} into {}",
                action.action, action.subject, action.path
            ))?;
        }
    }
    Ok(0)
}

fn skills(args: SkillsArgs, reporter_mode: ReporterMode, started_at: DateTime<Utc>) -> Result<i32> {
    let mut reporter = Reporter::from_mode(reporter_mode);
    let cwd = std::env::current_dir()?;
    match args.command {
        SkillsCommand::List(_args) => {
            let skills = discover_skills(&cwd)?;
            let data = SkillsListData { skills };
            if reporter.is_json() {
                emit_success(
                    &mut reporter,
                    "skills.list",
                    "skills list",
                    started_at,
                    &data,
                )?;
                return Ok(0);
            }
            if data.skills.is_empty() {
                reporter.human("No skills found.")?;
                return Ok(0);
            }
            for skill in data.skills {
                reporter.human(&format!(
                    "{}\t{}\t{}\t{}",
                    skill.source, skill.id, skill.content_hash, skill.path
                ))?;
            }
            Ok(0)
        }
        SkillsCommand::Show(args) => {
            let skill = resolve_skill(&cwd, &args.skill)?;
            let rendered = render_selected_skills(std::slice::from_ref(&skill));
            let data = SkillsShowData {
                skill: skill.info,
                rendered,
            };
            if reporter.is_json() {
                emit_success(
                    &mut reporter,
                    "skills.show",
                    "skills show",
                    started_at,
                    &data,
                )?;
            } else {
                reporter.human(&data.rendered)?;
            }
            Ok(0)
        }
    }
}

async fn review(args: ReviewArgs) -> Result<()> {
    rebotica_git::assert_repository()?;
    let cwd = std::env::current_dir()?;
    let loaded = LoadedConfig::read_from(&cwd)?;
    let diff_source = review_diff_source(&args)?;
    let diff_source_description = diff_source.description();
    let selected_skills = resolve_skills(&cwd, &args.skills)?;
    let changed_files = rebotica_git::changed_files_for(&diff_source)?;
    let mut envelope = TaskEnvelope::for_config(
        rebotica_runlog::make_id(),
        "review",
        args.goal.unwrap_or_else(|| {
            format!("Review the selected git diff ({diff_source_description}) for correctness, risk, and missing tests.")
        }),
        &loaded,
        changed_files,
        "json",
        args.risk,
    );
    if let Some(max_files) = args.max_files {
        envelope.max_files_changed = max_files;
    }
    if let Some(max_lines) = args.max_lines {
        envelope.max_changed_lines = max_lines;
    }
    let envelope_yaml = envelope.to_yaml()?;
    let mut prompt_parts = vec![
        read_harness_file("prompts/system/local-reviewer.md")?,
        read_harness_file("prompts/contracts/review-only.md")?,
        format!("## Task Envelope\n{envelope_yaml}"),
        format!("## Project Config\n{}", loaded.raw_or_placeholder()),
    ];
    if !selected_skills.is_empty() {
        prompt_parts.push(render_selected_skills(&selected_skills));
    }
    prompt_parts.extend([
        format!(
            "## Repository Instructions\n{}",
            collect_instruction_files(&cwd)?
        ),
        format!(
            "## Git Status\n{}",
            fenced(&rebotica_git::status_short()?, "text")
        ),
        format!(
            "## Git Diff Source\n{}",
            fenced(&diff_source_description, "text")
        ),
        format!(
            "## Git Diff Stat\n{}",
            fenced(&rebotica_git::diff_stat_for(&diff_source)?, "text")
        ),
        format!(
            "## Git Diff\n{}",
            fenced(
                &truncate(&rebotica_git::diff_for(&diff_source)?, 120_000),
                "diff"
            )
        ),
    ]);
    let prompt = prompt_parts.join("\n\n");
    let model_requests = model_requests(args.models);
    let multi_model = model_requests.len() > 1;
    for model_override in model_requests {
        let (model, text) = run_worker(
            &loaded,
            WorkerMode::Review,
            model_override,
            args.provider.clone(),
            args.temperature,
            prompt.clone(),
        )
        .await?;
        let run = rebotica_runlog::persist("review", &model, &envelope_yaml, &prompt, &text)?;
        persist_selected_skills(&run.directory, &selected_skills)?;
        if multi_model {
            println!("===== model: {model} run: {} =====", run.id);
        }
        println!("{text}");
        print_post_run_footer(&run.id, "review");
    }
    Ok(())
}

fn review_diff_source(args: &ReviewArgs) -> Result<rebotica_git::DiffSource> {
    selected_diff_source(&args.base, &args.range, args.cached)
}

fn model_requests(models: Vec<String>) -> Vec<Option<String>> {
    if models.is_empty() {
        vec![None]
    } else {
        models.into_iter().map(Some).collect()
    }
}

async fn explain(args: FileWorkerArgs) -> Result<()> {
    file_worker(args, WorkerMode::Explain, "explain", "analysis").await
}

async fn propose_tests(args: FileWorkerArgs) -> Result<()> {
    file_worker(args, WorkerMode::Tests, "propose_tests", "json").await
}

async fn file_worker(
    args: FileWorkerArgs,
    mode: WorkerMode,
    envelope_mode: &str,
    output_format: &str,
) -> Result<()> {
    rebotica_git::assert_repository()?;
    if args.files.is_empty() {
        return Err(anyhow!("{envelope_mode} requires at least one file."));
    }
    let cwd = std::env::current_dir()?;
    let loaded = LoadedConfig::read_from(&cwd)?;
    let selected_skills = resolve_skills(&cwd, &args.skills)?;
    rebotica_guard::ensure_allowed(&args.files, &loaded.config.forbidden_paths)?;
    let file_blocks = args
        .files
        .iter()
        .map(|file| {
            let text = read_project_file(&cwd, file)?;
            Ok(format!(
                "## File: {file}\n{}",
                fenced(&truncate(&text, 80_000), &language_for(file))
            ))
        })
        .collect::<Result<Vec<_>>>()?
        .join("\n\n");
    let default_goal = match mode {
        WorkerMode::Explain => {
            "Explain the selected files with attention to responsibilities, dependencies, and risks."
        }
        WorkerMode::Tests => "Propose focused missing tests for the selected files. Do not edit files.",
        _ => "Handle the selected files within the task envelope.",
    };
    let envelope = TaskEnvelope::for_config(
        rebotica_runlog::make_id(),
        envelope_mode,
        args.goal.unwrap_or_else(|| default_goal.to_string()),
        &loaded,
        args.files,
        output_format,
        "low",
    );
    let envelope_yaml = envelope.to_yaml()?;
    let system_prompt = match mode {
        WorkerMode::Tests => "prompts/system/local-test-writer.md",
        _ => "prompts/system/delegated-run.md",
    };
    let mut prompt_parts = vec![
        read_harness_file(system_prompt)?,
        format!("## Task Envelope\n{envelope_yaml}"),
    ];
    if !selected_skills.is_empty() {
        prompt_parts.push(render_selected_skills(&selected_skills));
    }
    prompt_parts.push(file_blocks);
    let prompt = prompt_parts.join("\n\n");
    let (model, text) = run_worker(
        &loaded,
        mode,
        args.model,
        args.provider,
        args.temperature,
        prompt.clone(),
    )
    .await?;
    let run = rebotica_runlog::persist(envelope_mode, &model, &envelope_yaml, &prompt, &text)?;
    persist_selected_skills(&run.directory, &selected_skills)?;
    println!("{text}");
    print_post_run_footer(&run.id, envelope_mode);
    Ok(())
}

async fn propose_patch(args: PatchArgs) -> Result<()> {
    rebotica_git::assert_repository()?;
    let cwd = std::env::current_dir()?;
    let loaded = LoadedConfig::read_from(&cwd)?;
    let selected_skills = resolve_skills(&cwd, &args.skills)?;
    let envelope_path = cwd.join(&args.envelope);
    let envelope_text = fs::read_to_string(&envelope_path)
        .with_context(|| format!("failed to read {}", envelope_path.display()))?;
    let allowed_files = parse_allowed_files_from_envelope(&envelope_text)?;
    let mut forbidden = loaded.config.forbidden_paths.clone();
    forbidden.extend(parse_forbidden_files_from_envelope(&envelope_text)?);
    rebotica_guard::ensure_allowed(&allowed_files, &forbidden)?;
    let mut prompt_parts = vec![
        read_harness_file("prompts/system/delegated-run.md")?,
        read_harness_file("prompts/contracts/patch-only.md")?,
        format!("## Task Envelope\n{envelope_text}"),
        format!("## Project Config\n{}", loaded.raw_or_placeholder()),
    ];
    if !selected_skills.is_empty() {
        prompt_parts.push(render_selected_skills(&selected_skills));
    }
    prompt_parts.push(format!(
        "## Current Context\n{}",
        collect_files_for_envelope(&cwd, &allowed_files)?
    ));
    let prompt = prompt_parts.join("\n\n");
    let (model, text) = run_worker(
        &loaded,
        WorkerMode::Patch,
        args.model,
        args.provider,
        args.temperature,
        prompt.clone(),
    )
    .await?;
    let run = rebotica_runlog::persist("propose_patch", &model, &envelope_text, &prompt, &text)?;
    persist_selected_skills(&run.directory, &selected_skills)?;
    if args.dry_run || !args.apply {
        println!("{text}");
        println!(
            "\ndry_run: true\nrun_id: {}\nnext_step: review the unified diff before applying it",
            run.id
        );
        print_post_run_footer(&run.id, "patch");
        return Ok(());
    }
    Err(anyhow!(
        "direct patch application is intentionally disabled in v0.1. Review the run output and apply manually."
    ))
}

fn guard_diff(args: GuardDiffArgs) -> Result<()> {
    rebotica_git::assert_repository()?;
    let loaded = LoadedConfig::read_from(&std::env::current_dir()?)?;
    let diff_source = guard_diff_source(&args)?;
    let diff_source_description = diff_source.description();
    let changed = rebotica_git::changed_files_for(&diff_source)?;
    let changed_lines = rebotica_git::changed_line_count_for(&diff_source)?;
    let max_files = args
        .max_files
        .unwrap_or(loaded.config.default_limits.max_files_changed);
    let max_lines = args
        .max_lines
        .unwrap_or(loaded.config.default_limits.max_changed_lines);
    rebotica_guard::ensure_allowed(&changed, &loaded.config.forbidden_paths)?;
    if changed.len() > max_files {
        return Err(anyhow!(
            "changed file count {} exceeds limit {}",
            changed.len(),
            max_files
        ));
    }
    if changed_lines > max_lines {
        return Err(anyhow!(
            "changed line count {} exceeds limit {}",
            changed_lines,
            max_lines
        ));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "ok": true,
            "diff_source": diff_source_description,
            "changed_files": changed.len(),
            "changed_lines": changed_lines
        }))?
    );
    Ok(())
}

fn guard_diff_source(args: &GuardDiffArgs) -> Result<rebotica_git::DiffSource> {
    selected_diff_source(&args.base, &args.range, args.cached)
}

fn selected_diff_source(
    base: &Option<String>,
    range: &Option<String>,
    cached: bool,
) -> Result<rebotica_git::DiffSource> {
    let source = if let Some(base) = base {
        rebotica_git::DiffSource::Base(base.clone())
    } else if let Some(range) = range {
        rebotica_git::DiffSource::Range(range.clone())
    } else if cached {
        rebotica_git::DiffSource::Cached
    } else {
        rebotica_git::DiffSource::WorkingTree
    };
    source.validate()?;
    Ok(source)
}

#[derive(Debug, Clone, Serialize)]
struct SkillInfo {
    id: String,
    source: String,
    path: String,
    title: String,
    content_hash: String,
}

#[derive(Debug, Clone)]
struct ResolvedSkill {
    info: SkillInfo,
    text: String,
}

fn resolve_skills(cwd: &Path, references: &[String]) -> Result<Vec<ResolvedSkill>> {
    references
        .iter()
        .map(|reference| resolve_skill(cwd, reference))
        .collect()
}

fn resolve_skill(cwd: &Path, reference: &str) -> Result<ResolvedSkill> {
    let (source_filter, id) = parse_skill_reference(reference)?;
    let mut matches = discover_skills_with_text(cwd)?
        .into_iter()
        .filter(|skill| skill.info.id == id)
        .filter(|skill| {
            source_filter
                .as_ref()
                .map(|source| skill.info.source == *source)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(anyhow!("skill not found: {reference}")),
        1 => Ok(matches.remove(0)),
        _ => Err(anyhow!(
            "ambiguous skill '{reference}'. Use canonical:{id} or project:{id}."
        )),
    }
}

fn parse_skill_reference(reference: &str) -> Result<(Option<String>, String)> {
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("skill id must not be empty"));
    }
    if let Some((source, id)) = trimmed.split_once(':') {
        if source != "canonical" && source != "project" {
            return Err(anyhow!(
                "unknown skill source '{source}'. Use canonical:<id> or project:<id>."
            ));
        }
        if id.is_empty() {
            return Err(anyhow!("skill id must not be empty"));
        }
        return Ok((Some(source.to_string()), id.to_string()));
    }
    Ok((None, trimmed.to_string()))
}

fn discover_skills(cwd: &Path) -> Result<Vec<SkillInfo>> {
    Ok(discover_skills_with_text(cwd)?
        .into_iter()
        .map(|skill| skill.info)
        .collect())
}

fn discover_skills_with_text(cwd: &Path) -> Result<Vec<ResolvedSkill>> {
    let mut skills = Vec::new();
    collect_skills_from_root(&harness_root()?.join("skills"), "canonical", &mut skills)?;
    collect_skills_from_root(&cwd.join(".rebotica/skills"), "project", &mut skills)?;
    skills.sort_by(|left, right| {
        left.info
            .source
            .cmp(&right.info.source)
            .then(left.info.id.cmp(&right.info.id))
            .then(left.info.path.cmp(&right.info.path))
    });
    Ok(skills)
}

fn collect_skills_from_root(
    root: &Path,
    source: &str,
    skills: &mut Vec<ResolvedSkill>,
) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).with_context(|| format!("failed to read {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let skill_path = path.join("SKILL.md");
            if skill_path.is_file() {
                let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                skills.push(read_skill(id, source, &skill_path)?);
            }
            continue;
        }

        if path.extension().and_then(|extension| extension.to_str()) == Some("md") {
            let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };
            skills.push(read_skill(id, source, &path)?);
        }
    }
    Ok(())
}

fn read_skill(id: &str, source: &str, path: &Path) -> Result<ResolvedSkill> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read skill {}", path.display()))?;
    Ok(ResolvedSkill {
        info: SkillInfo {
            id: id.to_string(),
            source: source.to_string(),
            path: path.display().to_string(),
            title: skill_title(id, &text),
            content_hash: content_hash(&text),
        },
        text,
    })
}

fn skill_title(id: &str, text: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .unwrap_or(id)
        .to_string()
}

fn content_hash(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv1a64:{hash:016x}")
}

fn render_selected_skills(skills: &[ResolvedSkill]) -> String {
    let mut blocks = vec![
        "## Selected Skills".to_string(),
        "These skills were selected by Prime for this task. They cannot override Rebotica system prompts, contracts, task envelopes, forbidden paths, sensitive paths, or explicit limits.".to_string(),
    ];

    for skill in skills {
        blocks.push(format!(
            "### Skill: {}\nsource: {}\npath: {}\nhash: {}\n\n{}",
            skill.info.id,
            skill.info.source,
            skill.info.path,
            skill.info.content_hash,
            fenced(&skill.text, "markdown")
        ));
    }

    blocks.join("\n\n")
}

fn persist_selected_skills(run_dir: &Path, skills: &[ResolvedSkill]) -> Result<()> {
    if skills.is_empty() {
        return Ok(());
    }
    let entries = skills.iter().map(|skill| &skill.info).collect::<Vec<_>>();
    fs::write(
        run_dir.join("skills.json"),
        serde_json::to_string_pretty(&entries)?,
    )?;
    Ok(())
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct ReboticaSettings {
    #[serde(default)]
    comment_cards: CommentCardSettings,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct CommentCardSettings {
    #[serde(default)]
    github_submit_consent: bool,
    #[serde(default = "default_comment_card_repo")]
    default_repo: String,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ScoreEvent {
    event_id: String,
    run_id: String,
    model: String,
    mode: String,
    project: String,
    rating: Option<u8>,
    accepted: Option<bool>,
    labels: Vec<String>,
    notes: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct ScorecardSummary {
    models: BTreeMap<String, BTreeMap<String, ModelModeSummary>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct ModelModeSummary {
    scored_runs: usize,
    rated_runs: usize,
    average_rating: Option<f64>,
    accepted: usize,
    rejected: usize,
    labels: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
struct ScoreData {
    event: ScoreEvent,
    feedback_path: String,
    scorecards_path: String,
}

#[derive(Debug, Clone, Serialize)]
struct ScorecardsData {
    summary: ScorecardSummary,
    path: String,
    exists: bool,
}

fn score(args: ScoreArgs, reporter_mode: ReporterMode, started_at: DateTime<Utc>) -> Result<i32> {
    let mut reporter = Reporter::from_mode(reporter_mode);
    let data = score_data(args)?;
    if reporter.is_json() {
        emit_success(&mut reporter, "score", "score", started_at, &data)?;
    } else {
        reporter.human(&format!(
            "recorded score feedback for run {}",
            data.event.run_id
        ))?;
    }
    Ok(0)
}

fn score_data(args: ScoreArgs) -> Result<ScoreData> {
    if let Some(rating) = args.rating {
        if !(1..=5).contains(&rating) {
            return Err(coded_error(
                ErrorCode::Usage,
                "--rating must be between 1 and 5",
            ));
        }
    }

    let run_dir = rebotica_runlog::runs_root().join(&args.run_id);
    if !run_dir.exists() {
        return Err(coded_error(
            ErrorCode::Config,
            format!("run not found: {}", args.run_id),
        ));
    }

    let scorecard = parse_scorecard_seed(&run_dir.join("scorecard.yml"))?;
    let accepted = if args.accepted {
        Some(true)
    } else if args.rejected {
        Some(false)
    } else {
        None
    };
    let event = ScoreEvent {
        event_id: rebotica_runlog::make_id(),
        run_id: args.run_id.clone(),
        model: scorecard
            .get("model")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        mode: scorecard
            .get("mode")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        project: scorecard
            .get("project")
            .cloned()
            .unwrap_or_else(|| "unknown".to_string()),
        rating: args.rating,
        accepted,
        labels: args.labels,
        notes: args.notes.unwrap_or_default(),
    };

    fs::write(
        run_dir.join("feedback.yml"),
        serde_yaml::to_string(&event).context("failed to serialize feedback")?,
    )?;
    append_model_event(&event)?;
    rebuild_model_scorecards()?;
    Ok(ScoreData {
        event,
        feedback_path: run_dir.join("feedback.yml").display().to_string(),
        scorecards_path: rebotica_runlog::root()
            .join("model-scorecards.yml")
            .display()
            .to_string(),
    })
}

fn scorecards(reporter_mode: ReporterMode, started_at: DateTime<Utc>) -> Result<i32> {
    let mut reporter = Reporter::from_mode(reporter_mode);
    let path = rebotica_runlog::root().join("model-scorecards.yml");
    if path.exists() {
        let text = fs::read_to_string(&path)?;
        if reporter.is_json() {
            let summary = serde_yaml::from_str(&text)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            let data = ScorecardsData {
                summary,
                path: path.display().to_string(),
                exists: true,
            };
            emit_success(&mut reporter, "scorecards", "scorecards", started_at, &data)?;
        } else {
            reporter.human(text.trim_end())?;
        }
    } else {
        let data = ScorecardsData {
            summary: ScorecardSummary::default(),
            path: path.display().to_string(),
            exists: false,
        };
        if reporter.is_json() {
            emit_success(&mut reporter, "scorecards", "scorecards", started_at, &data)?;
        } else {
            reporter.human("models: {}")?;
        }
    }
    Ok(0)
}

fn parse_scorecard_seed(path: &Path) -> Result<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    if !path.exists() {
        return Ok(values);
    }
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if value.starts_with('[') || value == "null" {
            continue;
        }
        values.insert(key.trim().to_string(), value.trim_matches('"').to_string());
    }
    Ok(values)
}

fn append_model_event(event: &ScoreEvent) -> Result<()> {
    fs::create_dir_all(rebotica_runlog::root())?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(rebotica_runlog::root().join("model-events.jsonl"))?;
    writeln!(file, "{}", serde_json::to_string(event)?)?;
    Ok(())
}

fn rebuild_model_scorecards() -> Result<()> {
    let events_path = rebotica_runlog::root().join("model-events.jsonl");
    let mut summary = ScorecardSummary::default();
    if events_path.exists() {
        let text = fs::read_to_string(&events_path)
            .with_context(|| format!("failed to read {}", events_path.display()))?;
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let event: ScoreEvent = serde_json::from_str(line)
                .with_context(|| format!("failed to parse model event: {line}"))?;
            let mode_summary = summary
                .models
                .entry(event.model.clone())
                .or_default()
                .entry(event.mode.clone())
                .or_default();
            mode_summary.scored_runs += 1;
            if let Some(true) = event.accepted {
                mode_summary.accepted += 1;
            } else if let Some(false) = event.accepted {
                mode_summary.rejected += 1;
            }
            for label in event.labels {
                *mode_summary.labels.entry(label).or_insert(0) += 1;
            }
            if let Some(rating) = event.rating {
                let existing_total =
                    mode_summary.average_rating.unwrap_or(0.0) * mode_summary.rated_runs as f64;
                mode_summary.rated_runs += 1;
                mode_summary.average_rating =
                    Some((existing_total + f64::from(rating)) / mode_summary.rated_runs as f64);
            }
        }
    }
    fs::create_dir_all(rebotica_runlog::root())?;
    fs::write(
        rebotica_runlog::root().join("model-scorecards.yml"),
        serde_yaml::to_string(&summary)?,
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct CommentCardNewData {
    card_id: String,
    status: String,
    path: String,
    title: String,
    kind: String,
    area: String,
    source: String,
    run_id: Option<String>,
    labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct CommentCardListData {
    status_filter: String,
    cards: Vec<CommentCardListItemData>,
}

#[derive(Debug, Clone, Serialize)]
struct CommentCardListItemData {
    status: String,
    card_id: String,
    title: String,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
struct CommentCardShowData {
    card_id: String,
    status: String,
    path: String,
    text: String,
}

#[derive(Debug, Clone, Serialize)]
struct CommentCardMoveData {
    card_id: String,
    from: String,
    to: String,
    source_path: String,
    target_path: String,
}

#[derive(Debug, Clone, Serialize)]
struct CommentCardConsentData {
    github_submit_consent: bool,
    default_repo: String,
    settings_path: String,
}

#[derive(Debug, Clone, Serialize)]
struct CommentCardSubmitData {
    card_id: String,
    repo: String,
    issue_output: String,
    move_result: CommentCardMoveData,
}

fn comment_card(
    args: CommentCardArgs,
    reporter_mode: ReporterMode,
    started_at: DateTime<Utc>,
) -> Result<i32> {
    let mut reporter = Reporter::from_mode(reporter_mode);
    match args.command {
        CommentCardCommand::New(args) => {
            let data = create_comment_card(args)?;
            if reporter.is_json() {
                emit_success(
                    &mut reporter,
                    "comment-card.new",
                    "comment-card new",
                    started_at,
                    &data,
                )?;
            } else {
                reporter.human(&format!("created comment card: {}", data.path))?;
            }
            Ok(0)
        }
        CommentCardCommand::List(args) => {
            let data = list_comment_cards(&args.status)?;
            if reporter.is_json() {
                emit_success(
                    &mut reporter,
                    "comment-card.list",
                    "comment-card list",
                    started_at,
                    &data,
                )?;
            } else {
                for card in &data.cards {
                    reporter.human(&format!(
                        "{}\t{}\t{}",
                        card.status, card.card_id, card.title
                    ))?;
                }
            }
            Ok(0)
        }
        CommentCardCommand::Show(args) => {
            let data = show_comment_card(&args.card_id)?;
            if reporter.is_json() {
                emit_success(
                    &mut reporter,
                    "comment-card.show",
                    "comment-card show",
                    started_at,
                    &data,
                )?;
            } else {
                reporter.human(&data.text)?;
            }
            Ok(0)
        }
        CommentCardCommand::Dismiss(args) => {
            let data = move_comment_card(&args.card_id, "pending", "dismissed")?;
            if reporter.is_json() {
                emit_success(
                    &mut reporter,
                    "comment-card.dismiss",
                    "comment-card dismiss",
                    started_at,
                    &data,
                )?;
            } else {
                reporter.human(&format!(
                    "moved comment card {} to {}",
                    data.card_id, data.to
                ))?;
            }
            Ok(0)
        }
        CommentCardCommand::Consent(args) => {
            let data = configure_comment_card_consent(args)?;
            if reporter.is_json() {
                emit_success(
                    &mut reporter,
                    "comment-card.consent",
                    "comment-card consent",
                    started_at,
                    &data,
                )?;
            } else {
                reporter.human(&format!(
                    "comment-card github_submit_consent: {}",
                    data.github_submit_consent
                ))?;
                reporter.human(&format!("comment-card default_repo: {}", data.default_repo))?;
            }
            Ok(0)
        }
        CommentCardCommand::Submit(args) => {
            let data = submit_comment_card(args)?;
            if reporter.is_json() {
                emit_success(
                    &mut reporter,
                    "comment-card.submit",
                    "comment-card submit",
                    started_at,
                    &data,
                )?;
            } else {
                reporter.human(&format!(
                    "moved comment card {} to {}",
                    data.move_result.card_id, data.move_result.to
                ))?;
                let issue_output = data.issue_output.trim_end();
                if !issue_output.is_empty() {
                    reporter.human(issue_output)?;
                }
            }
            Ok(0)
        }
    }
}

fn create_comment_card(args: CommentCardNewArgs) -> Result<CommentCardNewData> {
    let id = rebotica_runlog::make_id();
    let labels = comment_card_labels(&args.kind, &args.area, &args.source, &args.labels);
    let body = args.body.unwrap_or_else(|| {
        "Describe what happened, what you expected, and any workaround.".to_string()
    });
    let text = render_comment_card(
        &id,
        "pending",
        &args.title,
        &args.kind,
        &args.area,
        &args.source,
        args.from_run.as_deref(),
        &labels,
        &body,
    );
    let dir = comment_card_status_dir("pending");
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{id}.md"));
    fs::write(&path, text)?;
    Ok(CommentCardNewData {
        card_id: id,
        status: "pending".to_string(),
        path: path.display().to_string(),
        title: args.title,
        kind: args.kind,
        area: args.area,
        source: args.source,
        run_id: args.from_run,
        labels,
    })
}

fn comment_card_labels(kind: &str, area: &str, source: &str, extra: &[String]) -> Vec<String> {
    let mut labels = vec![
        "comment-card".to_string(),
        format!("kind:{kind}"),
        format!("area:{area}"),
        format!("source:{source}"),
    ];
    labels.extend(extra.iter().cloned());
    labels
}

fn render_comment_card(
    id: &str,
    status: &str,
    title: &str,
    kind: &str,
    area: &str,
    source: &str,
    run_id: Option<&str>,
    labels: &[String],
    body: &str,
) -> String {
    let labels_yaml = labels
        .iter()
        .map(|label| format!("  - {}", yaml_quote(label)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "---\nid: {}\nstatus: {}\ntitle: {}\nkind: {}\narea: {}\nsource: {}\nrun_id: {}\nlabels:\n{}\n---\n\n# {}\n\n{}\n",
        yaml_quote(id),
        yaml_quote(status),
        yaml_quote(title),
        yaml_quote(kind),
        yaml_quote(area),
        yaml_quote(source),
        run_id
            .map(yaml_quote)
            .unwrap_or_else(|| "null".to_string()),
        labels_yaml,
        title,
        body
    )
}

fn yaml_quote(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn list_comment_cards(status: &str) -> Result<CommentCardListData> {
    let statuses = if status == "all" {
        vec!["pending", "submitted", "dismissed"]
    } else {
        vec![status]
    };
    let mut cards = Vec::new();
    for status in statuses {
        let dir = comment_card_status_dir(status);
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
                continue;
            }
            let id = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_default();
            let title = comment_card_field(&path, "title")?.unwrap_or_default();
            cards.push(CommentCardListItemData {
                status: status.to_string(),
                card_id: id,
                title,
                path: path.display().to_string(),
            });
        }
    }
    Ok(CommentCardListData {
        status_filter: status.to_string(),
        cards,
    })
}

fn show_comment_card(card_id: &str) -> Result<CommentCardShowData> {
    let path = find_comment_card(card_id)?;
    let status = path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(CommentCardShowData {
        card_id: card_id.to_string(),
        status,
        text: fs::read_to_string(&path)?,
        path: path.display().to_string(),
    })
}

fn move_comment_card(card_id: &str, from: &str, to: &str) -> Result<CommentCardMoveData> {
    let source = comment_card_status_dir(from).join(format!("{card_id}.md"));
    if !source.exists() {
        return Err(anyhow!("comment card not found in {from}: {card_id}"));
    }
    let target_dir = comment_card_status_dir(to);
    fs::create_dir_all(&target_dir)?;
    let target = target_dir.join(format!("{card_id}.md"));
    fs::rename(&source, &target)?;
    Ok(CommentCardMoveData {
        card_id: card_id.to_string(),
        from: from.to_string(),
        to: to.to_string(),
        source_path: source.display().to_string(),
        target_path: target.display().to_string(),
    })
}

fn configure_comment_card_consent(args: CommentCardConsentArgs) -> Result<CommentCardConsentData> {
    let mut settings = read_settings()?;
    if args.allow_github {
        settings.comment_cards.github_submit_consent = true;
    }
    if args.revoke_github {
        settings.comment_cards.github_submit_consent = false;
    }
    if let Some(repo) = args.repo {
        settings.comment_cards.default_repo = repo;
    }
    write_settings(&settings)?;
    Ok(CommentCardConsentData {
        github_submit_consent: settings.comment_cards.github_submit_consent,
        default_repo: settings.comment_cards.default_repo,
        settings_path: settings_path().display().to_string(),
    })
}

fn submit_comment_card(args: CommentCardSubmitArgs) -> Result<CommentCardSubmitData> {
    let settings = read_settings()?;
    if !settings.comment_cards.github_submit_consent {
        return Err(anyhow!(
            "GitHub comment-card submission needs consent. Run: rbtc comment-card consent --allow-github"
        ));
    }
    let repo = args
        .repo
        .unwrap_or_else(|| settings.comment_cards.default_repo.clone());
    let path = comment_card_status_dir("pending").join(format!("{}.md", args.card_id));
    if !path.exists() {
        return Err(anyhow!("pending comment card not found: {}", args.card_id));
    }
    let title = comment_card_field(&path, "title")?
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| format!("Comment card {}", args.card_id));
    let labels = comment_card_labels_from_file(&path)?;
    ensure_github_labels(&repo, &labels);
    let mut command = ProcessCommand::new("gh");
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
    let output = command
        .output()
        .with_context(|| "failed to run gh issue create")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow!(
            "{}",
            if stderr.is_empty() {
                "gh issue create failed".to_string()
            } else {
                stderr
            }
        ));
    }
    let move_result = move_comment_card(&args.card_id, "pending", "submitted")?;
    Ok(CommentCardSubmitData {
        card_id: args.card_id,
        repo,
        issue_output: String::from_utf8_lossy(&output.stdout).to_string(),
        move_result,
    })
}

fn ensure_github_labels(repo: &str, labels: &[String]) {
    for label in labels {
        let _ = ProcessCommand::new("gh")
            .args([
                "label",
                "create",
                label,
                "--repo",
                repo,
                "--color",
                comment_card_label_color(label),
                "--description",
                "Rebotica comment card label",
                "--force",
            ])
            .output();
    }
}

fn comment_card_label_color(label: &str) -> &'static str {
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

fn read_settings() -> Result<ReboticaSettings> {
    let path = settings_path();
    if !path.exists() {
        return Ok(ReboticaSettings::default());
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_yaml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

fn write_settings(settings: &ReboticaSettings) -> Result<()> {
    fs::create_dir_all(rebotica_runlog::root())?;
    fs::write(settings_path(), serde_yaml::to_string(settings)?)?;
    Ok(())
}

fn settings_path() -> PathBuf {
    rebotica_runlog::root().join("settings.yml")
}

fn comment_cards_root() -> PathBuf {
    rebotica_runlog::root().join("comment-cards")
}

fn comment_card_status_dir(status: &str) -> PathBuf {
    comment_cards_root().join(status)
}

fn find_comment_card(card_id: &str) -> Result<PathBuf> {
    for status in ["pending", "submitted", "dismissed"] {
        let path = comment_card_status_dir(status).join(format!("{card_id}.md"));
        if path.exists() {
            return Ok(path);
        }
    }
    Err(anyhow!("comment card not found: {card_id}"))
}

fn pending_comment_card_count() -> Result<usize> {
    let dir = comment_card_status_dir("pending");
    if !dir.exists() {
        return Ok(0);
    }
    Ok(fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("md")
        })
        .count())
}

fn comment_card_field(path: &Path, field: &str) -> Result<Option<String>> {
    let text = fs::read_to_string(path)?;
    let prefix = format!("{field}:");
    Ok(text
        .lines()
        .find_map(|line| line.strip_prefix(&prefix).map(str::trim))
        .map(|value| value.trim_matches('"').to_string()))
}

fn comment_card_labels_from_file(path: &Path) -> Result<Vec<String>> {
    let text = fs::read_to_string(path)?;
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

fn print_post_run_footer(run_id: &str, area: &str) {
    eprintln!();
    eprintln!("---");
    eprintln!("Rebotica run: {run_id}");
    eprintln!("Prime next steps:");
    eprintln!("  rbtc score {run_id} --rating 4 --accepted --label useful-{area}");
    eprintln!(
        "  rbtc comment-card new --from-run {run_id} --kind ux --area {area} --source prime --title \"...\""
    );
}

#[derive(Debug, Clone, Serialize)]
struct RetroData {
    run_id: String,
    path: String,
    written: bool,
}

fn retrospective(
    args: RetroArgs,
    reporter_mode: ReporterMode,
    started_at: DateTime<Utc>,
) -> Result<i32> {
    let mut reporter = Reporter::from_mode(reporter_mode);
    let run_dir = rebotica_runlog::runs_root().join(&args.run_id);
    if !run_dir.exists() {
        return Err(anyhow!("run not found: {}", args.run_id));
    }
    let output = run_dir.join("retrospective.md");
    let written = !output.exists() || args.force;
    if !output.exists() || args.force {
        fs::write(
            &output,
            rebotica_runlog::retrospective_template(&args.run_id),
        )?;
    }
    let data = RetroData {
        run_id: args.run_id,
        path: output.display().to_string(),
        written,
    };
    if reporter.is_json() {
        emit_success(&mut reporter, "retro", "retro", started_at, &data)?;
    } else {
        reporter.human(&data.path)?;
    }
    Ok(0)
}

async fn run_worker(
    loaded: &LoadedConfig,
    mode: WorkerMode,
    model_override: Option<String>,
    provider_args: ProviderArgs,
    temperature: f64,
    prompt: String,
) -> Result<(String, String)> {
    let model = resolve_model(loaded, mode, model_override)?;
    let settings = provider_settings(loaded, provider_args)?;
    let provider = OpenAICompatibleProvider::new(&settings)?;
    let text = provider
        .chat(
            &model,
            vec![
                ChatMessage::new(
                    "system",
                    "You are a local model operating under a scoped task contract. Follow the supplied contract exactly.",
                ),
                ChatMessage::new("user", prompt),
            ],
            temperature,
        )
        .await?;
    Ok((model, text))
}

fn resolve_model(
    loaded: &LoadedConfig,
    mode: WorkerMode,
    model_override: Option<String>,
) -> Result<String> {
    if let Some(model) = model_override
        .or_else(|| std::env::var("REBOTICA_MODEL").ok())
        .filter(|value| !value.is_empty())
    {
        return Ok(resolve_model_alias(&loaded.config, &model));
    }
    model_for_mode(&loaded.config, mode).ok_or_else(|| {
        anyhow!(
            "missing model. Pass --model, set REBOTICA_MODEL, run rbtc models configure --detect, or configure models.default in .rebotica.yml."
        )
    })
}

fn provider_settings(loaded: &LoadedConfig, args: ProviderArgs) -> Result<ProviderSettings> {
    ProviderSettings::resolve(
        loaded,
        ProviderOverrides {
            provider: args.provider,
            base_url: args.base_url,
        },
    )
}

const MODEL_ROUTE_KEYS: [&str; 5] = ["default", "review", "explain", "tests", "patch"];

#[derive(Debug, Clone, PartialEq, Eq)]
enum DetectedModelChoice {
    ProviderUnavailable(String),
    NoModels,
    One(String),
    Multiple(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ModelRouteUpdate {
    config_path: String,
    alias: String,
    model: String,
    routes_written: Vec<String>,
    routes_kept: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ModelConfigureReport {
    Configured {
        source: String,
        provider: Option<String>,
        base_url: Option<String>,
        update: ModelRouteUpdate,
    },
    ProviderUnavailable {
        provider: Option<String>,
        base_url: Option<String>,
        error: String,
        next_step: String,
    },
    NoModels {
        provider: String,
        base_url: String,
        next_step: String,
    },
    MultipleModels {
        provider: String,
        base_url: String,
        models: Vec<String>,
        next_step: String,
    },
}

fn normalize_model_alias(alias: &str) -> Result<String> {
    let alias = alias.trim();
    if alias.is_empty() {
        return Err(anyhow!("--alias must not be empty"));
    }
    Ok(alias.to_string())
}

fn normalize_model_id(model: &str) -> Result<String> {
    let model = model.trim();
    if model.is_empty() {
        return Err(anyhow!("model id must not be empty"));
    }
    Ok(model.to_string())
}

fn choose_model_from_detection(
    models: std::result::Result<Vec<String>, String>,
) -> DetectedModelChoice {
    let models = match models {
        Ok(models) => models,
        Err(error) => return DetectedModelChoice::ProviderUnavailable(error),
    };
    let candidates = suitable_model_candidates(models);
    match candidates.len() {
        0 => DetectedModelChoice::NoModels,
        1 => DetectedModelChoice::One(candidates[0].clone()),
        _ => DetectedModelChoice::Multiple(candidates),
    }
}

fn suitable_model_candidates(models: Vec<String>) -> Vec<String> {
    let mut candidates = Vec::new();
    for model in models {
        let model = model.trim();
        if model.is_empty() || candidates.iter().any(|candidate| candidate == model) {
            continue;
        }
        candidates.push(model.to_string());
    }
    candidates
}

fn write_model_routes(
    config_path: &Path,
    alias: &str,
    model: &str,
    force: bool,
) -> Result<ModelRouteUpdate> {
    let model = normalize_model_id(model)?;
    let raw = fs::read_to_string(config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    let mut value: serde_yaml::Value = serde_yaml::from_str(&raw)
        .with_context(|| format!("failed to parse {}", config_path.display()))?;
    let root = value.as_mapping_mut().ok_or_else(|| {
        anyhow!(
            "{} must be a YAML mapping before model routes can be configured",
            config_path.display()
        )
    })?;

    let models_key = serde_yaml::Value::String("models".to_string());
    if !root.contains_key(&models_key) {
        root.insert(
            models_key.clone(),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
    }
    let models = root
        .get_mut(&models_key)
        .and_then(serde_yaml::Value::as_mapping_mut)
        .ok_or_else(|| anyhow!("models must be a YAML mapping"))?;

    let aliases_key = serde_yaml::Value::String("aliases".to_string());
    if !models.contains_key(&aliases_key) {
        models.insert(
            aliases_key.clone(),
            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        );
    }
    let aliases = models
        .get_mut(&aliases_key)
        .and_then(serde_yaml::Value::as_mapping_mut)
        .ok_or_else(|| anyhow!("models.aliases must be a YAML mapping"))?;
    let alias_key = serde_yaml::Value::String(alias.to_string());
    if let Some(existing) = aliases
        .get(&alias_key)
        .and_then(serde_yaml::Value::as_str)
        .filter(|existing| {
            let existing = existing.trim();
            !existing.is_empty() && existing != model.as_str()
        })
    {
        if !force {
            return Err(anyhow!(
                "models.aliases.{alias} already points to {existing}. Pass --force or choose a different --alias."
            ));
        }
    }
    aliases.insert(alias_key, serde_yaml::Value::String(model.clone()));

    let mut routes_written = Vec::new();
    let mut routes_kept = Vec::new();
    for route in MODEL_ROUTE_KEYS {
        let route_key = serde_yaml::Value::String(route.to_string());
        let existing = models
            .get(&route_key)
            .and_then(serde_yaml::Value::as_str)
            .unwrap_or_default()
            .trim();
        if force || existing.is_empty() {
            models.insert(route_key, serde_yaml::Value::String(alias.to_string()));
            routes_written.push(route.to_string());
        } else {
            routes_kept.push(route.to_string());
        }
    }

    let rendered = serde_yaml::to_string(&value)
        .with_context(|| format!("failed to serialize {}", config_path.display()))?;
    fs::write(config_path, rendered)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    Ok(ModelRouteUpdate {
        config_path: config_path.display().to_string(),
        alias: alias.to_string(),
        model,
        routes_written,
        routes_kept,
    })
}

#[derive(Debug, Clone, Serialize)]
struct ModelConfigureData {
    status: String,
    source: Option<String>,
    provider: Option<String>,
    base_url: Option<String>,
    config_path: Option<String>,
    alias: Option<String>,
    model: Option<String>,
    routes_written: Vec<String>,
    routes_kept: Vec<String>,
    models: Vec<String>,
    error: Option<String>,
    next_step: String,
}

impl From<&ModelConfigureReport> for ModelConfigureData {
    fn from(report: &ModelConfigureReport) -> Self {
        match report {
            ModelConfigureReport::Configured {
                source,
                provider,
                base_url,
                update,
            } => Self {
                status: "configured".to_string(),
                source: Some(source.clone()),
                provider: provider.clone(),
                base_url: base_url.clone(),
                config_path: Some(update.config_path.clone()),
                alias: Some(update.alias.clone()),
                model: Some(update.model.clone()),
                routes_written: update.routes_written.clone(),
                routes_kept: update.routes_kept.clone(),
                models: Vec::new(),
                error: None,
                next_step: format!("rbtc smoke --model {}", update.alias),
            },
            ModelConfigureReport::ProviderUnavailable {
                provider,
                base_url,
                error,
                next_step,
            } => Self {
                status: "provider_unavailable".to_string(),
                source: None,
                provider: provider.clone(),
                base_url: base_url.clone(),
                config_path: None,
                alias: None,
                model: None,
                routes_written: Vec::new(),
                routes_kept: Vec::new(),
                models: Vec::new(),
                error: Some(error.clone()),
                next_step: next_step.clone(),
            },
            ModelConfigureReport::NoModels {
                provider,
                base_url,
                next_step,
            } => Self {
                status: "no_models".to_string(),
                source: None,
                provider: Some(provider.clone()),
                base_url: Some(base_url.clone()),
                config_path: None,
                alias: None,
                model: None,
                routes_written: Vec::new(),
                routes_kept: Vec::new(),
                models: Vec::new(),
                error: None,
                next_step: next_step.clone(),
            },
            ModelConfigureReport::MultipleModels {
                provider,
                base_url,
                models,
                next_step,
            } => Self {
                status: "multiple_models".to_string(),
                source: None,
                provider: Some(provider.clone()),
                base_url: Some(base_url.clone()),
                config_path: None,
                alias: None,
                model: None,
                routes_written: Vec::new(),
                routes_kept: Vec::new(),
                models: models.clone(),
                error: None,
                next_step: next_step.clone(),
            },
        }
    }
}

impl ModelConfigureData {
    fn error_message(&self) -> String {
        match self.status.as_str() {
            "provider_unavailable" => self
                .error
                .clone()
                .unwrap_or_else(|| "provider model detection unavailable".to_string()),
            "no_models" => "provider returned no models".to_string(),
            "multiple_models" => "multiple provider models found".to_string(),
            _ => "model configuration failed".to_string(),
        }
    }
}

fn print_model_configure_report(
    reporter: &mut Reporter,
    report: &ModelConfigureReport,
) -> Result<()> {
    if reporter.is_json() {
        return Ok(());
    }
    match report {
        ModelConfigureReport::Configured {
            source,
            provider,
            base_url,
            update,
        } => {
            reporter.human(&format!(
                "configured model routes in {}",
                update.config_path
            ))?;
            reporter.human(&format!("  source: {source}"))?;
            if let Some(provider) = provider {
                reporter.human(&format!("  provider: {provider}"))?;
            }
            if let Some(base_url) = base_url {
                reporter.human(&format!("  base_url: {base_url}"))?;
            }
            reporter.human(&format!("  alias: {} -> {}", update.alias, update.model))?;
            reporter.human(&format!(
                "  routes written: {}",
                comma_list_or_none(&update.routes_written)
            ))?;
            reporter.human(&format!(
                "  routes kept: {}",
                comma_list_or_none(&update.routes_kept)
            ))?;
            reporter.human(&format!("next: rbtc smoke --model {}", update.alias))?;
        }
        ModelConfigureReport::ProviderUnavailable {
            provider,
            base_url,
            error,
            next_step,
        } => {
            reporter.human("provider model detection unavailable; no changes written")?;
            if let Some(provider) = provider {
                reporter.human(&format!("  provider: {provider}"))?;
            }
            if let Some(base_url) = base_url {
                reporter.human(&format!("  base_url: {base_url}"))?;
            }
            reporter.human(&format!("  error: {error}"))?;
            reporter.human(&format!("next: {next_step}"))?;
        }
        ModelConfigureReport::NoModels {
            provider,
            base_url,
            next_step,
        } => {
            reporter.human("provider returned no models; no changes written")?;
            reporter.human(&format!("  provider: {provider}"))?;
            reporter.human(&format!("  base_url: {base_url}"))?;
            reporter.human(&format!("next: {next_step}"))?;
        }
        ModelConfigureReport::MultipleModels {
            provider,
            base_url,
            models,
            next_step,
        } => {
            reporter.human("multiple provider models found; no changes written")?;
            reporter.human(&format!("  provider: {provider}"))?;
            reporter.human(&format!("  base_url: {base_url}"))?;
            reporter.human("models:")?;
            for model in models {
                reporter.human(&format!("  {model}"))?;
            }
            reporter.human(&format!("next: {next_step}"))?;
        }
    }
    Ok(())
}

fn model_configure_next_step() -> String {
    "Start a provider with one loaded model and run rbtc models configure --detect, or run rbtc models configure --model MODEL_ID.".to_string()
}

fn comma_list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        values.join(", ")
    }
}

fn model_routes_empty(config: &ProjectConfig) -> bool {
    config.models.default.is_empty()
        && config.models.review.is_empty()
        && config.models.explain.is_empty()
        && config.models.tests.is_empty()
        && config.models.patch.is_empty()
}

#[derive(Debug, Serialize)]
struct Check {
    status: &'static str,
    id: String,
    message: String,
    detail: Option<String>,
}

impl Check {
    fn ok(id: impl Into<String>, message: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status: "ok",
            id: id.into(),
            message: message.into(),
            detail: Some(detail.into()),
        }
    }

    fn warn(id: impl Into<String>, message: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status: "warn",
            id: id.into(),
            message: message.into(),
            detail: Some(detail.into()),
        }
    }

    fn fail(id: impl Into<String>, message: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            status: "fail",
            id: id.into(),
            message: message.into(),
            detail: Some(detail.into()),
        }
    }

    fn from_result<T>(
        id: impl Into<String>,
        message: impl Into<String>,
        detail: Option<String>,
        result: &Result<T>,
    ) -> Self {
        match result {
            Ok(_) => Self {
                status: "ok",
                id: id.into(),
                message: message.into(),
                detail,
            },
            Err(error) => Self::fail(id, message, error.to_string()),
        }
    }
}

fn validate_config(loaded: &LoadedConfig) -> Vec<Check> {
    let mut checks = Vec::new();
    if loaded.path.is_none() {
        checks.push(Check::warn(
            "config.exists",
            "Project config exists",
            "missing; run rbtc init for shared project policy",
        ));
    } else {
        checks.push(Check::ok(
            "config.exists",
            "Project config exists",
            loaded
                .path
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        ));
    }

    if loaded.config.default_limits.max_files_changed == 0 {
        checks.push(Check::fail(
            "config.limits.max_files_changed",
            "max_files_changed is nonzero",
            "value is 0",
        ));
    }
    if loaded.config.default_limits.max_changed_lines == 0 {
        checks.push(Check::fail(
            "config.limits.max_changed_lines",
            "max_changed_lines is nonzero",
            "value is 0",
        ));
    }

    if loaded.config.providers.default.is_empty() {
        checks.push(Check::fail(
            "config.providers.default",
            "Default provider is configured",
            "providers.default is empty",
        ));
    } else {
        checks.push(Check::ok(
            "config.providers.default",
            "Default provider is configured",
            loaded.config.providers.default.clone(),
        ));
    }

    checks
}

fn model_routes_data(config: &ProjectConfig) -> ModelRoutesData {
    ModelRoutesData {
        default: config.models.default.clone(),
        review: config.models.review.clone(),
        explain: config.models.explain.clone(),
        tests: config.models.tests.clone(),
        patch: config.models.patch.clone(),
        aliases: config.models.aliases.clone(),
    }
}

fn provider_summary(config: &ProjectConfig) -> ProvidersData {
    let mut providers = Vec::new();
    if !config.providers.entries.contains_key("lmstudio") {
        providers.push(ProviderItemData {
            name: "lmstudio".to_string(),
            kind: "openai-compatible".to_string(),
            base_url: "http://127.0.0.1:1234/v1".to_string(),
            api_key_env: String::new(),
            api_key_present: false,
            headers_count: 0,
            implicit: true,
        });
    }
    for (name, provider) in &config.providers.entries {
        providers.push(ProviderItemData {
            name: name.clone(),
            kind: provider.kind.clone(),
            base_url: provider.base_url.clone(),
            api_key_env: provider.api_key_env.clone(),
            api_key_present: !provider.api_key_env.is_empty()
                && std::env::var(&provider.api_key_env)
                    .map(|value| !value.is_empty())
                    .unwrap_or(false),
            headers_count: provider.headers.len(),
            implicit: false,
        });
    }
    ProvidersData {
        default: config.providers.default.clone(),
        providers,
    }
}

fn print_model_route(
    reporter: &mut Reporter,
    route: &str,
    selected: &str,
    config: &ProjectConfig,
) -> Result<()> {
    if selected.is_empty() {
        reporter.human(&format!("  {route}: (not configured)"))?;
    } else {
        reporter.human(&format!(
            "  {route}: {} -> {}",
            selected,
            resolve_model_alias(config, selected)
        ))?;
    }
    Ok(())
}

fn installed_check(id: &str, relative: &str, message: &str) -> Check {
    let path = std::env::current_dir()
        .map(|cwd| cwd.join(relative))
        .unwrap_or_else(|_| PathBuf::from(relative));
    if path.exists() {
        Check::ok(id, message, path.display().to_string())
    } else {
        Check::warn(id, message, "not installed")
    }
}

fn installed_any_check(id: &str, relatives: &[&str], message: &str) -> Check {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    for relative in relatives {
        let path = cwd.join(relative);
        if path.exists() {
            return Check::ok(id, message, path.display().to_string());
        }
    }
    Check::warn(id, message, "not installed")
}

fn ensure_dir(path: &Path) -> Result<()> {
    match fs::create_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) => shell_mkdir_p(path).with_context(|| {
            format!(
                "failed to create directory {} after std create_dir_all failed: {error}",
                path.display()
            )
        }),
    }
}

#[cfg(unix)]
fn shell_mkdir_p(path: &Path) -> Result<()> {
    let shell_cwd = harness_root().unwrap_or_else(|_| PathBuf::from("/"));
    let status = ProcessCommand::new("mkdir")
        .current_dir(shell_cwd)
        .arg("-p")
        .arg(path)
        .status()
        .with_context(|| format!("failed to run mkdir for {}", path.display()))?;
    if !status.success() {
        return Err(anyhow!("mkdir failed for {}", path.display()));
    }
    Ok(())
}

#[cfg(not(unix))]
fn shell_mkdir_p(_path: &Path) -> Result<()> {
    Err(anyhow!(
        "std create_dir_all failed and shell fallback is only implemented on unix"
    ))
}

fn install_claude(copy: bool, force: bool) -> Result<Vec<InstallActionData>> {
    let cwd = std::env::current_dir()?;
    let harness = harness_root()?;
    let commands_target = cwd.join(".claude/commands");
    let skills_target = cwd.join(".claude/skills");
    ensure_dir(&commands_target)?;
    ensure_dir(&skills_target)?;
    install_directory_contents(
        &harness.join("claude/commands"),
        &commands_target,
        copy,
        force,
    )?;
    install_directory_contents(&harness.join("skills"), &skills_target, copy, force)?;
    Ok(vec![
        InstallActionData {
            action: if copy { "copied" } else { "linked" }.to_string(),
            subject: "Claude commands".to_string(),
            path: commands_target.display().to_string(),
        },
        InstallActionData {
            action: if copy { "copied" } else { "linked" }.to_string(),
            subject: "Rebotica skills".to_string(),
            path: skills_target.display().to_string(),
        },
    ])
}

fn install_codex(
    copy: bool,
    force: bool,
    target_dir: Option<String>,
) -> Result<Vec<InstallActionData>> {
    let cwd = std::env::current_dir()?;
    let harness = harness_root()?;
    let skills_target = target_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.join(".agents/skills"));
    ensure_dir(&skills_target)?;
    install_directory_contents(&harness.join("skills"), &skills_target, copy, force)?;
    Ok(vec![InstallActionData {
        action: if copy { "copied" } else { "linked" }.to_string(),
        subject: "Rebotica skills".to_string(),
        path: skills_target.display().to_string(),
    }])
}

fn install_github(force: bool) -> Result<Vec<InstallActionData>> {
    let cwd = std::env::current_dir()?;
    let harness = harness_root()?;
    let github_target = cwd.join(".github");
    ensure_dir(&github_target)?;
    install_directory_contents(&harness.join("github"), &github_target, true, force)?;
    Ok(vec![InstallActionData {
        action: "copied".to_string(),
        subject: "GitHub assets".to_string(),
        path: github_target.display().to_string(),
    }])
}

fn harness_root() -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var("REBOTICA_HOME") {
        let root = PathBuf::from(explicit);
        if root.join("prompts/system/delegated-run.md").exists() {
            return Ok(root);
        }
    }

    let cwd = std::env::current_dir()?;
    for candidate in cwd.ancestors() {
        if candidate.join("prompts/system/delegated-run.md").exists() {
            return Ok(candidate.to_path_buf());
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        for candidate in exe.ancestors() {
            if candidate.join("prompts/system/delegated-run.md").exists() {
                return Ok(candidate.to_path_buf());
            }
        }
    }

    Err(anyhow!(
        "could not locate Rebotica harness root. Set REBOTICA_HOME."
    ))
}

fn read_harness_file(relative: &str) -> Result<String> {
    let path = harness_root()?.join(relative);
    fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))
}

fn collect_instruction_files(cwd: &Path) -> Result<String> {
    let names = [
        "AGENTS.md",
        "CLAUDE.md",
        "STYLE.md",
        "ARCHITECTURE.md",
        "README.md",
    ];
    let blocks = names
        .iter()
        .filter_map(|name| {
            let path = cwd.join(name);
            if !path.exists() {
                return None;
            }
            let text = fs::read_to_string(&path).ok()?;
            Some(format!("# {name}\n{}", truncate(&text, 40_000)))
        })
        .collect::<Vec<_>>();
    if blocks.is_empty() {
        Ok("(none found)".to_string())
    } else {
        Ok(blocks.join("\n\n"))
    }
}

fn collect_files_for_envelope(cwd: &Path, files: &[String]) -> Result<String> {
    files
        .iter()
        .map(|file| {
            let path = cwd.join(file);
            if !path.exists() {
                return Ok(format!("## File: {file}\n(missing)"));
            }
            let text = fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            Ok(format!(
                "## File: {file}\n{}",
                fenced(&truncate(&text, 80_000), &language_for(file))
            ))
        })
        .collect::<Result<Vec<_>>>()
        .map(|blocks| blocks.join("\n\n"))
}

fn read_project_file(cwd: &Path, file: &str) -> Result<String> {
    let path = cwd.join(file);
    if !path.exists() {
        return Err(anyhow!("file not found: {file}"));
    }
    fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))
}

fn install_directory_contents(source: &Path, target: &Path, copy: bool, force: bool) -> Result<()> {
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if target_path.exists() || fs::symlink_metadata(&target_path).is_ok() {
            if force {
                if copy && source_path.is_dir() && target_path.is_dir() && !target_path.is_symlink()
                {
                    // Merge-copy directories on force. This avoids platform-specific
                    // remove_dir_all behavior on metadata-protected directories and
                    // still refreshes contained files.
                } else if target_path.is_dir() && !target_path.is_symlink() {
                    fs::remove_dir_all(&target_path)
                        .with_context(|| format!("failed to remove {}", target_path.display()))?;
                } else {
                    fs::remove_file(&target_path)
                        .with_context(|| format!("failed to remove {}", target_path.display()))?;
                }
            } else {
                continue;
            }
        }
        if copy {
            if source_path.is_dir() {
                if let Err(error) = copy_dir_all(&source_path, &target_path) {
                    shell_copy_dir(&source_path, &target_path, force).with_context(|| {
                        format!(
                            "failed to copy {} to {} after std copy failed: {error}",
                            source_path.display(),
                            target_path.display()
                        )
                    })?;
                }
            } else {
                fs::copy(&source_path, &target_path).with_context(|| {
                    format!(
                        "failed to copy {} to {}",
                        source_path.display(),
                        target_path.display()
                    )
                })?;
            }
        } else {
            symlink(&source_path, &target_path).with_context(|| {
                format!(
                    "failed to link {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn shell_copy_dir(source: &Path, target: &Path, force: bool) -> Result<()> {
    if force && target.exists() {
        let shell_cwd = harness_root().unwrap_or_else(|_| PathBuf::from("/"));
        let status = ProcessCommand::new("rm")
            .current_dir(&shell_cwd)
            .arg("-rf")
            .arg(target)
            .status()
            .with_context(|| format!("failed to run rm for {}", target.display()))?;
        if !status.success() {
            return Err(anyhow!("rm failed for {}", target.display()));
        }
    }
    let status = ProcessCommand::new("cp")
        .current_dir(harness_root().unwrap_or_else(|_| PathBuf::from("/")))
        .arg("-R")
        .arg(source)
        .arg(target)
        .status()
        .with_context(|| format!("failed to run cp for {}", source.display()))?;
    if !status.success() {
        return Err(anyhow!(
            "cp failed from {} to {}",
            source.display(),
            target.display()
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn shell_copy_dir(_source: &Path, _target: &Path, _force: bool) -> Result<()> {
    Err(anyhow!(
        "std directory copy failed and shell fallback is only implemented on unix"
    ))
}

fn copy_dir_all(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)
        .with_context(|| format!("failed to create directory {}", target.display()))?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir_all(&source_path, &target_path)?;
        } else {
            if target_path.exists() {
                fs::remove_file(&target_path).with_context(|| {
                    format!("failed to remove existing file {}", target_path.display())
                })?;
            }
            fs::copy(&source_path, &target_path).with_context(|| {
                format!(
                    "failed to copy file {} to {}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(source: &Path, target: &Path) -> Result<()> {
    std::os::unix::fs::symlink(source, target)?;
    Ok(())
}

#[cfg(windows)]
fn symlink(source: &Path, target: &Path) -> Result<()> {
    if source.is_dir() {
        std::os::windows::fs::symlink_dir(source, target)?;
    } else {
        std::os::windows::fs::symlink_file(source, target)?;
    }
    Ok(())
}

fn fenced(text: &str, language: &str) -> String {
    format!("```{language}\n{text}\n```")
}

fn truncate(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n[truncated {} chars]", &text[..end], text.len() - end)
}

fn language_for(file: &str) -> String {
    Path::new(file)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("text")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::ffi::OsString;
    use std::fs;
    use std::path::{Path, PathBuf};
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
                "rebotica-cli-{name}-{}-{suffix}",
                std::process::id()
            ));
            fs::create_dir_all(&path).expect("temp dir should be created");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
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

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn install_targets_parse_public_variants() {
        for (target, expected) in [
            ("claude", InstallTarget::Claude),
            ("codex", InstallTarget::Codex),
            ("github", InstallTarget::Github),
            ("all", InstallTarget::All),
        ] {
            let cli = Cli::try_parse_from(["rbtc", "install", target]).unwrap();
            let Some(Command::Install(args)) = cli.command else {
                panic!("expected install command for {target}");
            };
            assert_eq!(args.target, expected);
        }
    }

    #[test]
    fn version_is_a_flag_not_a_subcommand() {
        let subcommand_error = Cli::try_parse_from(["rbtc", "version"]).unwrap_err();
        assert_eq!(subcommand_error.kind(), ErrorKind::InvalidSubcommand);

        let flag_error = Cli::try_parse_from(["rbtc", "--version"]).unwrap_err();
        assert_eq!(flag_error.kind(), ErrorKind::DisplayVersion);
    }

    #[test]
    fn error_code_for_uses_typed_coded_errors_only() {
        let coded = coded_error(ErrorCode::ProviderUnavailable, "provider down");
        assert_eq!(error_code_for(&coded), ErrorCode::ProviderUnavailable);

        let uncoded = anyhow!("provider down");
        assert_eq!(error_code_for(&uncoded), ErrorCode::Internal);
    }

    #[test]
    fn delegated_modes_parse_under_run() {
        let Some(Command::Run(RunArgs {
            mode: RunMode::Review(args),
        })) = Cli::try_parse_from(["rbtc", "run", "review", "--base", "main"])
            .unwrap()
            .command
        else {
            panic!("expected run review command");
        };
        assert_eq!(args.base, Some("main".to_string()));

        let Some(Command::Run(RunArgs {
            mode: RunMode::Explain(args),
        })) = Cli::try_parse_from(["rbtc", "run", "explain", "src/main.rs"])
            .unwrap()
            .command
        else {
            panic!("expected run explain command");
        };
        assert_eq!(args.files, vec!["src/main.rs"]);

        let Some(Command::Run(RunArgs {
            mode: RunMode::Tests(args),
        })) = Cli::try_parse_from(["rbtc", "run", "tests", "src/main.rs"])
            .unwrap()
            .command
        else {
            panic!("expected run tests command");
        };
        assert_eq!(args.files, vec!["src/main.rs"]);

        let Some(Command::Run(RunArgs {
            mode: RunMode::Patch(args),
        })) = Cli::try_parse_from([
            "rbtc",
            "run",
            "patch",
            ".rebotica/tasks/task.yml",
            "--dry-run",
        ])
        .unwrap()
        .command
        else {
            panic!("expected run patch command");
        };
        assert_eq!(args.envelope, ".rebotica/tasks/task.yml");
        assert!(args.dry_run);
    }

    #[test]
    fn delegated_modes_are_not_top_level_subcommands() {
        for mode in ["review", "explain", "tests", "patch"] {
            let error = Cli::try_parse_from(["rbtc", mode]).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::InvalidSubcommand);
        }
    }

    #[test]
    fn models_configure_detect_command_parses() {
        let Some(Command::Models(args)) = Cli::try_parse_from([
            "rbtc",
            "models",
            "configure",
            "--detect",
            "--alias",
            "model",
        ])
        .unwrap()
        .command
        else {
            panic!("expected models command");
        };
        let Some(ModelsCommand::Configure(configure)) = args.command else {
            panic!("expected models configure command");
        };

        assert!(configure.detect);
        assert_eq!(configure.alias, "model");
        assert_eq!(configure.model, None);
    }

    #[test]
    fn model_detection_choice_handles_provider_unavailable_one_and_multiple() {
        assert_eq!(
            choose_model_from_detection(Err("connection refused".to_string())),
            DetectedModelChoice::ProviderUnavailable("connection refused".to_string())
        );
        assert_eq!(
            choose_model_from_detection(Ok(vec![" only-model ".to_string()])),
            DetectedModelChoice::One("only-model".to_string())
        );
        assert_eq!(
            choose_model_from_detection(Ok(vec![
                "first-model".to_string(),
                "second-model".to_string(),
                "first-model".to_string(),
            ])),
            DetectedModelChoice::Multiple(vec![
                "first-model".to_string(),
                "second-model".to_string(),
            ])
        );
    }

    #[test]
    fn write_model_routes_populates_empty_routes_without_overwriting_existing_routes() {
        let temp = TempDir::new("model-routes");
        let config_path = temp.path().join(".rebotica.yml");
        fs::write(
            &config_path,
            "project:\n  name: sample\nmodels:\n  default: \"\"\n  review: existing-reviewer\n  explain: \"\"\n  tests: \"\"\n  patch: \"\"\n  aliases: {}\n",
        )
        .unwrap();

        let update = write_model_routes(&config_path, "local-model", "raw-model-id", false)
            .expect("model routes should be written");

        assert_eq!(
            update.routes_written,
            vec!["default", "explain", "tests", "patch"]
        );
        assert_eq!(update.routes_kept, vec!["review"]);
        let loaded = LoadedConfig::read_from(temp.path()).unwrap();
        assert_eq!(loaded.config.models.default, "local-model");
        assert_eq!(loaded.config.models.review, "existing-reviewer");
        assert_eq!(
            loaded.config.models.aliases.get("local-model"),
            Some(&"raw-model-id".to_string())
        );
    }

    #[test]
    fn review_diff_source_flags_parse_public_variants() {
        let Some(Command::Run(RunArgs {
            mode: RunMode::Review(default_args),
        })) = Cli::try_parse_from(["rbtc", "run", "review"])
            .unwrap()
            .command
        else {
            panic!("expected run review command");
        };
        assert_eq!(
            review_diff_source(&default_args).unwrap(),
            rebotica_git::DiffSource::WorkingTree
        );

        let Some(Command::Run(RunArgs {
            mode: RunMode::Review(base_args),
        })) = Cli::try_parse_from(["rbtc", "run", "review", "--base", "origin/main"])
            .unwrap()
            .command
        else {
            panic!("expected run review command");
        };
        assert_eq!(
            review_diff_source(&base_args).unwrap(),
            rebotica_git::DiffSource::Base("origin/main".to_string())
        );

        let Some(Command::Run(RunArgs {
            mode: RunMode::Review(range_args),
        })) = Cli::try_parse_from(["rbtc", "run", "review", "--range", "main..HEAD"])
            .unwrap()
            .command
        else {
            panic!("expected run review command");
        };
        assert_eq!(
            review_diff_source(&range_args).unwrap(),
            rebotica_git::DiffSource::Range("main..HEAD".to_string())
        );

        let Some(Command::Run(RunArgs {
            mode: RunMode::Review(cached_args),
        })) = Cli::try_parse_from(["rbtc", "run", "review", "--cached"])
            .unwrap()
            .command
        else {
            panic!("expected run review command");
        };
        assert_eq!(
            review_diff_source(&cached_args).unwrap(),
            rebotica_git::DiffSource::Cached
        );
    }

    #[test]
    fn review_limit_overrides_parse() {
        let Some(Command::Run(RunArgs {
            mode: RunMode::Review(args),
        })) = Cli::try_parse_from([
            "rbtc",
            "run",
            "review",
            "--max-files",
            "6",
            "--max-lines",
            "450",
        ])
        .unwrap()
        .command
        else {
            panic!("expected run review command");
        };

        assert_eq!(args.max_files, Some(6));
        assert_eq!(args.max_lines, Some(450));
    }

    #[test]
    fn review_accepts_repeated_model_flags_for_side_by_side_runs() {
        let Some(Command::Run(RunArgs {
            mode: RunMode::Review(args),
        })) = Cli::try_parse_from([
            "rbtc", "run", "review", "--model", "gemma", "--model", "qwen",
        ])
        .unwrap()
        .command
        else {
            panic!("expected run review command");
        };

        assert_eq!(args.models, vec!["gemma", "qwen"]);
        assert_eq!(
            model_requests(args.models),
            vec![Some("gemma".to_string()), Some("qwen".to_string())]
        );
    }

    #[test]
    fn review_diff_source_flags_conflict() {
        let error = Cli::try_parse_from([
            "rbtc",
            "run",
            "review",
            "--base",
            "main",
            "--range",
            "main..HEAD",
        ])
        .unwrap_err();

        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn guard_diff_source_flags_parse_public_variants() {
        let Some(Command::GuardDiff(base_args)) =
            Cli::try_parse_from(["rbtc", "guard-diff", "--base", "origin/main"])
                .unwrap()
                .command
        else {
            panic!("expected guard-diff command");
        };
        assert_eq!(
            guard_diff_source(&base_args).unwrap(),
            rebotica_git::DiffSource::Base("origin/main".to_string())
        );

        let Some(Command::GuardDiff(range_args)) =
            Cli::try_parse_from(["rbtc", "guard-diff", "--range", "main..HEAD"])
                .unwrap()
                .command
        else {
            panic!("expected guard-diff command");
        };
        assert_eq!(
            guard_diff_source(&range_args).unwrap(),
            rebotica_git::DiffSource::Range("main..HEAD".to_string())
        );

        let Some(Command::GuardDiff(cached_args)) =
            Cli::try_parse_from(["rbtc", "guard-diff", "--cached"])
                .unwrap()
                .command
        else {
            panic!("expected guard-diff command");
        };
        assert_eq!(
            guard_diff_source(&cached_args).unwrap(),
            rebotica_git::DiffSource::Cached
        );
    }

    #[test]
    fn selected_skills_are_persisted_as_metadata() {
        let temp = TempDir::new("skills-metadata");
        let skill = ResolvedSkill {
            info: SkillInfo {
                id: "domain".to_string(),
                source: "project".to_string(),
                path: ".rebotica/skills/domain.md".to_string(),
                title: "Domain".to_string(),
                content_hash: "fnv1a64:test".to_string(),
            },
            text: "# Domain\n".to_string(),
        };

        persist_selected_skills(temp.path(), &[skill]).unwrap();

        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(temp.path().join("skills.json")).unwrap())
                .unwrap();
        assert_eq!(json[0]["id"], "domain");
        assert_eq!(json[0]["source"], "project");
        assert_eq!(json[0]["content_hash"], "fnv1a64:test");
    }

    #[test]
    fn score_records_feedback_event_and_updates_model_summary() {
        let _lock = env_lock();
        let temp = TempDir::new("score");
        let _home = EnvGuard::set("HOME", temp.path());
        let run_dir = rebotica_runlog::runs_root().join("run-1");
        fs::create_dir_all(&run_dir).unwrap();
        fs::write(
            run_dir.join("scorecard.yml"),
            "run_id: run-1\nproject: sample\nmodel: local-reviewer\nmode: review\n",
        )
        .unwrap();

        score_data(ScoreArgs {
            run_id: "run-1".to_string(),
            rating: Some(5),
            accepted: true,
            rejected: false,
            labels: vec!["useful-review".to_string()],
            notes: Some("caught a missing test".to_string()),
        })
        .unwrap();

        let feedback = fs::read_to_string(run_dir.join("feedback.yml")).unwrap();
        assert!(feedback.contains("rating: 5"));
        assert!(feedback.contains("useful-review"));
        let events = fs::read_to_string(temp.path().join(".rebotica/model-events.jsonl")).unwrap();
        assert!(events.contains("local-reviewer"));
        let summary =
            fs::read_to_string(temp.path().join(".rebotica/model-scorecards.yml")).unwrap();
        assert!(summary.contains("local-reviewer"));
        assert!(summary.contains("average_rating: 5.0"));
    }

    #[test]
    fn comment_cards_are_created_and_dismissed_locally() {
        let _lock = env_lock();
        let temp = TempDir::new("comment-card");
        let _home = EnvGuard::set("HOME", temp.path());

        create_comment_card(CommentCardNewArgs {
            from_run: Some("run-1".to_string()),
            kind: "ux".to_string(),
            area: "review".to_string(),
            source: "prime".to_string(),
            title: "Review needs clearer next steps".to_string(),
            body: Some("The Prime needed a stronger nudge.".to_string()),
            labels: vec!["area:review".to_string()],
        })
        .unwrap();

        assert_eq!(pending_comment_card_count().unwrap(), 1);
        let pending = comment_card_status_dir("pending");
        let card_path = fs::read_dir(&pending)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let card_id = card_path.file_stem().unwrap().to_string_lossy().to_string();
        let text = fs::read_to_string(&card_path).unwrap();
        assert!(text.contains("Review needs clearer next steps"));
        assert!(text.contains("source: \"prime\""));

        move_comment_card(&card_id, "pending", "dismissed").unwrap();
        assert_eq!(pending_comment_card_count().unwrap(), 0);
        assert!(comment_card_status_dir("dismissed")
            .join(format!("{card_id}.md"))
            .exists());
    }

    #[test]
    fn comment_card_consent_writes_settings() {
        let _lock = env_lock();
        let temp = TempDir::new("comment-card-consent");
        let _home = EnvGuard::set("HOME", temp.path());

        configure_comment_card_consent(CommentCardConsentArgs {
            allow_github: true,
            revoke_github: false,
            repo: Some("catalandres/rebotica".to_string()),
        })
        .unwrap();

        let settings = read_settings().unwrap();
        assert!(settings.comment_cards.github_submit_consent);
        assert_eq!(settings.comment_cards.default_repo, "catalandres/rebotica");
    }

    #[test]
    fn init_project_creates_config_and_private_project_state() {
        let temp = TempDir::new("init");
        let template = r#"project:
  name: example-project
  type: unknown

models:
  default: ""
"#;

        init_project_at(temp.path(), false, Some(template)).unwrap();

        let project_name = temp
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let config = fs::read_to_string(temp.path().join(".rebotica.yml")).unwrap();
        assert!(config.contains(&format!("name: {project_name}")));
        assert!(temp.path().join(".rebotica/tasks").is_dir());
        assert!(temp.path().join(".rebotica/runs").is_dir());
        assert_eq!(
            fs::read_to_string(temp.path().join(".rebotica/.gitignore")).unwrap(),
            "runs/\n"
        );
    }

    #[test]
    fn init_project_refuses_to_overwrite_existing_config_without_force() {
        let temp = TempDir::new("init-existing");
        fs::write(temp.path().join(".rebotica.yml"), "existing: true\n").unwrap();

        let error = init_project_at(temp.path(), false, Some("project: {}\n")).unwrap_err();

        assert!(error
            .to_string()
            .contains(".rebotica.yml already exists. Use --force to overwrite."));
        assert!(!temp.path().join(".rebotica/tasks").exists());
    }

    #[test]
    fn init_project_force_overwrites_config_and_state_gitignore() {
        let temp = TempDir::new("init-force");
        fs::create_dir_all(temp.path().join(".rebotica")).unwrap();
        fs::write(temp.path().join(".rebotica.yml"), "existing: true\n").unwrap();
        fs::write(temp.path().join(".rebotica/.gitignore"), "old\n").unwrap();

        init_project_at(
            temp.path(),
            true,
            Some("project:\n  name: example-project\n"),
        )
        .unwrap();

        let config = fs::read_to_string(temp.path().join(".rebotica.yml")).unwrap();
        assert!(config.contains("project:"));
        assert!(!config.contains("existing: true"));
        assert_eq!(
            fs::read_to_string(temp.path().join(".rebotica/.gitignore")).unwrap(),
            "runs/\n"
        );
    }

    #[test]
    fn provider_summary_includes_implicit_lmstudio_default() {
        let summary = provider_summary(&ProjectConfig::default());

        assert_eq!(summary.default, "lmstudio");
        assert!(summary.providers.iter().any(|provider| {
            provider.name == "lmstudio"
                && provider.base_url == "http://127.0.0.1:1234/v1"
                && provider.implicit
        }));
    }

    #[test]
    fn validate_config_fails_zero_limits() {
        let mut config = ProjectConfig::default();
        config.default_limits.max_files_changed = 0;
        config.default_limits.max_changed_lines = 0;
        let loaded = LoadedConfig {
            path: Some(PathBuf::from(".rebotica.yml")),
            raw: String::new(),
            config,
        };

        let checks = validate_config(&loaded);

        assert!(checks.iter().any(|check| {
            check.status == "fail" && check.id == "config.limits.max_files_changed"
        }));
        assert!(checks.iter().any(|check| {
            check.status == "fail" && check.id == "config.limits.max_changed_lines"
        }));
    }
}
