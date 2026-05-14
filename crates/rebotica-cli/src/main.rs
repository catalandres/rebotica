use anyhow::{anyhow, Context, Result};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use rebotica_core::{
    model_for_mode, parse_allowed_files_from_envelope, parse_forbidden_files_from_envelope,
    resolve_model_alias, LoadedConfig, ProjectConfig, TaskEnvelope, WorkerMode,
};
use rebotica_provider::{
    ChatMessage, OpenAICompatibleProvider, ProviderOverrides, ProviderSettings,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

#[derive(Debug, Parser)]
#[command(name = "rbtc", version)]
#[command(about = "A governed local-worker harness for collaborative software craftsmanship.")]
#[command(after_help = "Common workflows:
  rbtc init
  rbtc doctor
  rbtc skills list
  rbtc models --configured-only
  rbtc models configure --detect
  rbtc review --base main
  rbtc patch .rebotica/tasks/task.yml --dry-run

Provider setup:
  export REBOTICA_BASE_URL=http://127.0.0.1:1234/v1
  export REBOTICA_MODEL=MODEL_ID")]
struct Cli {
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
    #[command(about = "Ask a bounded worker to review a selected git diff.")]
    Review(ReviewArgs),
    #[command(about = "Ask a bounded worker to explain selected files.")]
    Explain(FileWorkerArgs),
    #[command(about = "Ask a bounded worker to propose focused tests for selected files.")]
    Tests(FileWorkerArgs),
    #[command(about = "Ask a bounded worker for a dry-run unified diff from a task envelope.")]
    Patch(PatchArgs),
    #[command(about = "Check a selected git diff against forbidden paths and size limits.")]
    GuardDiff(GuardDiffArgs),
    #[command(about = "Record Prime feedback about a worker/model run.")]
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
    #[arg(long, help = "Emit machine-readable JSON checks.")]
    json: bool,
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
    #[arg(long, help = "Emit machine-readable JSON output.")]
    json: bool,
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
        default_value = "local-worker",
        help = "Alias to write under models.aliases and use for empty routes."
    )]
    alias: String,
    #[arg(
        long,
        help = "Replace existing route values and an existing alias target."
    )]
    force: bool,
    #[arg(long, help = "Emit machine-readable JSON output.")]
    json: bool,
}

#[derive(Debug, Parser)]
struct ProvidersArgs {
    #[arg(long, help = "Emit machine-readable JSON output.")]
    json: bool,
}

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
    #[command(about = "Print a skill exactly as it would be attached to a worker.")]
    Show(SkillsShowArgs),
}

#[derive(Debug, Parser)]
struct SkillsListArgs {
    #[arg(long, help = "Emit machine-readable JSON output.")]
    json: bool,
}

#[derive(Debug, Parser)]
struct SkillsShowArgs {
    #[arg(
        value_name = "SKILL",
        help = "Skill id, or canonical:<id> / project:<id> when disambiguating."
    )]
    skill: String,
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
        help = "Attach a canonical or project-local skill as worker context."
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
    #[arg(long, help = "Model alias or raw provider model id for this worker.")]
    model: Option<String>,
    #[arg(long, help = "Optional goal to put in the task envelope.")]
    goal: Option<String>,
    #[arg(
        long = "skill",
        value_name = "SKILL",
        help = "Attach a canonical or project-local skill as worker context."
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
        help = "Project file to include in the worker context."
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
        help = "Attach a canonical or project-local skill as worker context."
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
    #[arg(long, value_name = "1-5", help = "Prime rating for the worker output.")]
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
        help = "Feedback source, for example prime, human, or worker."
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
    if let Err(error) = run().await {
        eprintln!("rbtc: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    };
    match command {
        Command::Doctor(args) => doctor(args).await,
        Command::Models(args) => models(args).await,
        Command::Providers(args) => providers(args),
        Command::Health(args) => health(args).await,
        Command::Smoke(args) => smoke(args).await,
        Command::Init(args) => init_project(args),
        Command::Install(args) => install(args),
        Command::Skills(args) => skills(args),
        Command::Review(args) => review(args).await,
        Command::Explain(args) => explain(args).await,
        Command::Tests(args) => propose_tests(args).await,
        Command::Patch(args) => propose_patch(args).await,
        Command::GuardDiff(args) => guard_diff(args),
        Command::Score(args) => score(args),
        Command::Scorecards => scorecards(),
        Command::CommentCard(args) => comment_card(args),
        Command::Retro(args) => retrospective(args),
    }
}

async fn doctor(args: DoctorArgs) -> Result<()> {
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
    if args.json {
        println!("{}", serde_json::to_string_pretty(&checks)?);
    } else {
        for check in &checks {
            println!(
                "{:<5} {:<24} {}{}",
                check.status,
                check.id,
                check.message,
                check
                    .detail
                    .as_ref()
                    .map(|detail| format!(" ({detail})"))
                    .unwrap_or_default()
            );
        }
    }

    if failed {
        Err(anyhow!("doctor found failing checks"))
    } else {
        Ok(())
    }
}

async fn models(args: ModelsArgs) -> Result<()> {
    if let Some(command) = args.command {
        return match command {
            ModelsCommand::Configure(configure_args) => configure_models(configure_args).await,
        };
    }

    let loaded = LoadedConfig::read_from(&std::env::current_dir()?)?;
    let routes = serde_json::json!({
        "default": &loaded.config.models.default,
        "review": &loaded.config.models.review,
        "explain": &loaded.config.models.explain,
        "tests": &loaded.config.models.tests,
        "patch": &loaded.config.models.patch,
        "aliases": &loaded.config.models.aliases,
    });

    let provider_models = if args.configured_only {
        None
    } else {
        let settings = provider_settings(&loaded, args.provider)?;
        let provider = OpenAICompatibleProvider::new(&settings)?;
        Some(serde_json::json!({
            "provider": settings.name,
            "base_url": settings.base_url,
            "models": provider.models().await?
        }))
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "configured": routes,
                "provider": provider_models
            }))?
        );
    } else {
        println!("Configured routes:");
        print_model_route("default", &loaded.config.models.default, &loaded.config);
        print_model_route("review", &loaded.config.models.review, &loaded.config);
        print_model_route("explain", &loaded.config.models.explain, &loaded.config);
        print_model_route("tests", &loaded.config.models.tests, &loaded.config);
        print_model_route("patch", &loaded.config.models.patch, &loaded.config);
        if !loaded.config.models.aliases.is_empty() {
            println!("\nAliases:");
            for (alias, target) in &loaded.config.models.aliases {
                println!("  {alias} -> {target}");
            }
        }
        if let Some(provider_models) = provider_models {
            println!(
                "\nProvider models ({}):",
                provider_models["provider"].as_str().unwrap_or("provider")
            );
            for model in provider_models["models"].as_array().into_iter().flatten() {
                if let Some(model) = model.as_str() {
                    println!("  {model}");
                }
            }
        }
    }
    Ok(())
}

async fn configure_models(args: ModelConfigureArgs) -> Result<()> {
    let loaded = LoadedConfig::read_from(&std::env::current_dir()?)?;
    let Some(config_path) = loaded.path.clone() else {
        return Err(anyhow!(
            "no project config found. Run rbtc init before configuring model routes."
        ));
    };

    if args.model.is_none() && !args.detect {
        return Err(anyhow!(
            "pass --model MODEL_ID to configure manually, or --detect to inspect the provider."
        ));
    }

    let alias = normalize_model_alias(&args.alias)?;
    let report = if let Some(model) = args.model {
        let model = normalize_model_id(&model)?;
        let update = write_model_routes(&config_path, &alias, &model, args.force)?;
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
                print_model_configure_report(&report, args.json)?;
                return Ok(());
            }
        };
        let provider = OpenAICompatibleProvider::new(&settings)?;
        match choose_model_from_detection(provider.models().await.map_err(|error| error.to_string()))
        {
            DetectedModelChoice::One(model) => {
                let update = write_model_routes(&config_path, &alias, &model, args.force)?;
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

    print_model_configure_report(&report, args.json)?;
    Ok(())
}

fn providers(args: ProvidersArgs) -> Result<()> {
    let loaded = LoadedConfig::read_from(&std::env::current_dir()?)?;
    let summary = provider_summary(&loaded.config);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!("Default provider: {}", loaded.config.providers.default);
        for item in summary["providers"].as_array().into_iter().flatten() {
            let name = item["name"].as_str().unwrap_or("");
            let base_url = item["base_url"].as_str().unwrap_or("");
            let kind = item["kind"].as_str().unwrap_or("");
            let auth = if item["api_key_env"].as_str().unwrap_or("").is_empty() {
                "no api key env".to_string()
            } else if item["api_key_present"].as_bool().unwrap_or(false) {
                format!("{} present", item["api_key_env"].as_str().unwrap_or(""))
            } else {
                format!("{} missing", item["api_key_env"].as_str().unwrap_or(""))
            };
            println!("  {name}: {kind} {base_url} ({auth})");
        }
    }
    Ok(())
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

fn init_project(args: InitArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    init_project_at(&cwd, args.force, None)
}

fn init_project_at(cwd: &Path, force: bool, template_override: Option<&str>) -> Result<()> {
    let config_path = cwd.join(".rebotica.yml");
    let state_dir = cwd.join(".rebotica");
    if config_path.exists() && !force {
        return Err(anyhow!(
            ".rebotica.yml already exists. Use --force to overwrite."
        ));
    }

    ensure_dir(&state_dir.join("tasks"))?;
    ensure_dir(&state_dir.join("runs"))?;

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

    let state_ignore = state_dir.join(".gitignore");
    if !state_ignore.exists() || force {
        fs::write(&state_ignore, "runs/\n")?;
    }

    println!("created {}", config_path.display());
    println!("created {}", state_dir.join("tasks").display());
    println!("created {}", state_dir.join("runs").display());
    println!("created {}", state_ignore.display());
    let loaded = LoadedConfig::read_from(cwd)?;
    if model_routes_empty(&loaded.config) {
        println!();
        println!("model routes are empty.");
        println!("next: rbtc models configure --detect");
        println!("or:   rbtc models configure --model MODEL_ID");
    }
    Ok(())
}

fn install(args: InstallArgs) -> Result<()> {
    match args.target {
        InstallTarget::Claude => install_claude(args.copy, args.force),
        InstallTarget::Codex => install_codex(args.copy, args.force, args.target_dir),
        InstallTarget::Github => install_github(args.force),
        InstallTarget::All => {
            install_claude(args.copy, args.force)?;
            install_codex(args.copy, args.force, args.target_dir)?;
            install_github(args.force)
        }
    }
}

fn skills(args: SkillsArgs) -> Result<()> {
    let cwd = std::env::current_dir()?;
    match args.command {
        SkillsCommand::List(args) => {
            let skills = discover_skills(&cwd)?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&skills)?);
                return Ok(());
            }
            if skills.is_empty() {
                println!("No skills found.");
                return Ok(());
            }
            for skill in skills {
                println!(
                    "{}\t{}\t{}\t{}",
                    skill.source, skill.id, skill.content_hash, skill.path
                );
            }
            Ok(())
        }
        SkillsCommand::Show(args) => {
            let skill = resolve_skill(&cwd, &args.skill)?;
            println!("{}", render_selected_skills(&[skill]));
            Ok(())
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
        _ => "prompts/system/local-worker.md",
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
        read_harness_file("prompts/system/local-worker.md")?,
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

#[derive(Debug, Clone, Default, Serialize)]
struct ScorecardSummary {
    models: BTreeMap<String, BTreeMap<String, ModelModeSummary>>,
}

#[derive(Debug, Clone, Default, Serialize)]
struct ModelModeSummary {
    scored_runs: usize,
    rated_runs: usize,
    average_rating: Option<f64>,
    accepted: usize,
    rejected: usize,
    labels: BTreeMap<String, usize>,
}

fn score(args: ScoreArgs) -> Result<()> {
    if let Some(rating) = args.rating {
        if !(1..=5).contains(&rating) {
            return Err(anyhow!("--rating must be between 1 and 5"));
        }
    }

    let run_dir = rebotica_runlog::runs_root().join(&args.run_id);
    if !run_dir.exists() {
        return Err(anyhow!("run not found: {}", args.run_id));
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
    println!("recorded score feedback for run {}", event.run_id);
    Ok(())
}

fn scorecards() -> Result<()> {
    let path = rebotica_runlog::root().join("model-scorecards.yml");
    if path.exists() {
        print!("{}", fs::read_to_string(path)?);
    } else {
        println!("models: {{}}");
    }
    Ok(())
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

fn comment_card(args: CommentCardArgs) -> Result<()> {
    match args.command {
        CommentCardCommand::New(args) => create_comment_card(args),
        CommentCardCommand::List(args) => list_comment_cards(&args.status),
        CommentCardCommand::Show(args) => show_comment_card(&args.card_id),
        CommentCardCommand::Dismiss(args) => {
            move_comment_card(&args.card_id, "pending", "dismissed")
        }
        CommentCardCommand::Consent(args) => configure_comment_card_consent(args),
        CommentCardCommand::Submit(args) => submit_comment_card(args),
    }
}

fn create_comment_card(args: CommentCardNewArgs) -> Result<()> {
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
    println!("created comment card: {}", path.display());
    Ok(())
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

fn list_comment_cards(status: &str) -> Result<()> {
    let statuses = if status == "all" {
        vec!["pending", "submitted", "dismissed"]
    } else {
        vec![status]
    };
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
            println!("{status}\t{id}\t{title}");
        }
    }
    Ok(())
}

fn show_comment_card(card_id: &str) -> Result<()> {
    let path = find_comment_card(card_id)?;
    println!("{}", fs::read_to_string(path)?);
    Ok(())
}

fn move_comment_card(card_id: &str, from: &str, to: &str) -> Result<()> {
    let source = comment_card_status_dir(from).join(format!("{card_id}.md"));
    if !source.exists() {
        return Err(anyhow!("comment card not found in {from}: {card_id}"));
    }
    let target_dir = comment_card_status_dir(to);
    fs::create_dir_all(&target_dir)?;
    fs::rename(&source, target_dir.join(format!("{card_id}.md")))?;
    println!("moved comment card {card_id} to {to}");
    Ok(())
}

fn configure_comment_card_consent(args: CommentCardConsentArgs) -> Result<()> {
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
    println!(
        "comment-card github_submit_consent: {}",
        settings.comment_cards.github_submit_consent
    );
    println!(
        "comment-card default_repo: {}",
        settings.comment_cards.default_repo
    );
    Ok(())
}

fn submit_comment_card(args: CommentCardSubmitArgs) -> Result<()> {
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
    move_comment_card(&args.card_id, "pending", "submitted")?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    Ok(())
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

fn retrospective(args: RetroArgs) -> Result<()> {
    let run_dir = rebotica_runlog::runs_root().join(&args.run_id);
    if !run_dir.exists() {
        return Err(anyhow!("run not found: {}", args.run_id));
    }
    let output = run_dir.join("retrospective.md");
    if !output.exists() || args.force {
        fs::write(
            &output,
            rebotica_runlog::retrospective_template(&args.run_id),
        )?;
    }
    println!("{}", output.display());
    Ok(())
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
                    "You are a bounded local worker. Follow the supplied contract exactly.",
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

fn print_model_configure_report(report: &ModelConfigureReport, json: bool) -> Result<()> {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&model_configure_json(report))?
        );
        return Ok(());
    }

    match report {
        ModelConfigureReport::Configured {
            source,
            provider,
            base_url,
            update,
        } => {
            println!("configured model routes in {}", update.config_path);
            println!("  source: {source}");
            if let Some(provider) = provider {
                println!("  provider: {provider}");
            }
            if let Some(base_url) = base_url {
                println!("  base_url: {base_url}");
            }
            println!("  alias: {} -> {}", update.alias, update.model);
            println!(
                "  routes written: {}",
                comma_list_or_none(&update.routes_written)
            );
            println!("  routes kept: {}", comma_list_or_none(&update.routes_kept));
            println!("next: rbtc smoke --model {}", update.alias);
        }
        ModelConfigureReport::ProviderUnavailable {
            provider,
            base_url,
            error,
            next_step,
        } => {
            println!("provider model detection unavailable; no changes written");
            if let Some(provider) = provider {
                println!("  provider: {provider}");
            }
            if let Some(base_url) = base_url {
                println!("  base_url: {base_url}");
            }
            println!("  error: {error}");
            println!("next: {next_step}");
        }
        ModelConfigureReport::NoModels {
            provider,
            base_url,
            next_step,
        } => {
            println!("provider returned no models; no changes written");
            println!("  provider: {provider}");
            println!("  base_url: {base_url}");
            println!("next: {next_step}");
        }
        ModelConfigureReport::MultipleModels {
            provider,
            base_url,
            models,
            next_step,
        } => {
            println!("multiple provider models found; no changes written");
            println!("  provider: {provider}");
            println!("  base_url: {base_url}");
            println!("models:");
            for model in models {
                println!("  {model}");
            }
            println!("next: {next_step}");
        }
    }
    Ok(())
}

fn model_configure_json(report: &ModelConfigureReport) -> serde_json::Value {
    match report {
        ModelConfigureReport::Configured {
            source,
            provider,
            base_url,
            update,
        } => serde_json::json!({
            "status": "configured",
            "source": source,
            "provider": provider,
            "base_url": base_url,
            "config_path": update.config_path,
            "alias": update.alias,
            "model": update.model,
            "routes_written": update.routes_written,
            "routes_kept": update.routes_kept,
            "next_step": format!("rbtc smoke --model {}", update.alias)
        }),
        ModelConfigureReport::ProviderUnavailable {
            provider,
            base_url,
            error,
            next_step,
        } => serde_json::json!({
            "status": "provider_unavailable",
            "provider": provider,
            "base_url": base_url,
            "error": error,
            "next_step": next_step
        }),
        ModelConfigureReport::NoModels {
            provider,
            base_url,
            next_step,
        } => serde_json::json!({
            "status": "no_models",
            "provider": provider,
            "base_url": base_url,
            "next_step": next_step
        }),
        ModelConfigureReport::MultipleModels {
            provider,
            base_url,
            models,
            next_step,
        } => serde_json::json!({
            "status": "multiple_models",
            "provider": provider,
            "base_url": base_url,
            "models": models,
            "next_step": next_step
        }),
    }
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

fn provider_summary(config: &ProjectConfig) -> serde_json::Value {
    let mut providers = Vec::new();
    if !config.providers.entries.contains_key("lmstudio") {
        providers.push(serde_json::json!({
            "name": "lmstudio",
            "kind": "openai-compatible",
            "base_url": "http://127.0.0.1:1234/v1",
            "api_key_env": "",
            "api_key_present": false,
            "headers_count": 0,
            "implicit": true
        }));
    }
    for (name, provider) in &config.providers.entries {
        providers.push(serde_json::json!({
            "name": name,
            "kind": provider.kind,
            "base_url": provider.base_url,
            "api_key_env": provider.api_key_env,
            "api_key_present": !provider.api_key_env.is_empty()
                && std::env::var(&provider.api_key_env)
                    .map(|value| !value.is_empty())
                    .unwrap_or(false),
            "headers_count": provider.headers.len(),
            "implicit": false
        }));
    }
    serde_json::json!({
        "default": config.providers.default,
        "providers": providers
    })
}

fn print_model_route(route: &str, selected: &str, config: &ProjectConfig) {
    if selected.is_empty() {
        println!("  {route}: (not configured)");
    } else {
        println!(
            "  {route}: {} -> {}",
            selected,
            resolve_model_alias(config, selected)
        );
    }
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

fn install_claude(copy: bool, force: bool) -> Result<()> {
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
    println!(
        "{} Claude commands into {}",
        if copy { "copied" } else { "linked" },
        commands_target.display()
    );
    println!(
        "{} Rebotica skills into {}",
        if copy { "copied" } else { "linked" },
        skills_target.display()
    );
    Ok(())
}

fn install_codex(copy: bool, force: bool, target_dir: Option<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let harness = harness_root()?;
    let skills_target = target_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.join(".agents/skills"));
    ensure_dir(&skills_target)?;
    install_directory_contents(&harness.join("skills"), &skills_target, copy, force)?;
    println!(
        "{} Rebotica skills into {}",
        if copy { "copied" } else { "linked" },
        skills_target.display()
    );
    Ok(())
}

fn install_github(force: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let harness = harness_root()?;
    let github_target = cwd.join(".github");
    ensure_dir(&github_target)?;
    install_directory_contents(&harness.join("github"), &github_target, true, force)?;
    println!("copied GitHub assets into {}", github_target.display());
    Ok(())
}

fn harness_root() -> Result<PathBuf> {
    if let Ok(explicit) = std::env::var("REBOTICA_HOME") {
        let root = PathBuf::from(explicit);
        if root.join("prompts/system/local-worker.md").exists() {
            return Ok(root);
        }
    }

    let cwd = std::env::current_dir()?;
    for candidate in cwd.ancestors() {
        if candidate.join("prompts/system/local-worker.md").exists() {
            return Ok(candidate.to_path_buf());
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        for candidate in exe.ancestors() {
            if candidate.join("prompts/system/local-worker.md").exists() {
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
    fn models_configure_detect_command_parses() {
        let Some(Command::Models(args)) = Cli::try_parse_from([
            "rbtc",
            "models",
            "configure",
            "--detect",
            "--alias",
            "worker",
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
        assert_eq!(configure.alias, "worker");
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

        let update = write_model_routes(&config_path, "local-worker", "raw-model-id", false)
            .expect("model routes should be written");

        assert_eq!(
            update.routes_written,
            vec!["default", "explain", "tests", "patch"]
        );
        assert_eq!(update.routes_kept, vec!["review"]);
        let loaded = LoadedConfig::read_from(temp.path()).unwrap();
        assert_eq!(loaded.config.models.default, "local-worker");
        assert_eq!(loaded.config.models.review, "existing-reviewer");
        assert_eq!(
            loaded.config.models.aliases.get("local-worker"),
            Some(&"raw-model-id".to_string())
        );
    }

    #[test]
    fn review_diff_source_flags_parse_public_variants() {
        let Some(Command::Review(default_args)) =
            Cli::try_parse_from(["rbtc", "review"]).unwrap().command
        else {
            panic!("expected review command");
        };
        assert_eq!(
            review_diff_source(&default_args).unwrap(),
            rebotica_git::DiffSource::WorkingTree
        );

        let Some(Command::Review(base_args)) =
            Cli::try_parse_from(["rbtc", "review", "--base", "origin/main"])
                .unwrap()
                .command
        else {
            panic!("expected review command");
        };
        assert_eq!(
            review_diff_source(&base_args).unwrap(),
            rebotica_git::DiffSource::Base("origin/main".to_string())
        );

        let Some(Command::Review(range_args)) =
            Cli::try_parse_from(["rbtc", "review", "--range", "main..HEAD"])
                .unwrap()
                .command
        else {
            panic!("expected review command");
        };
        assert_eq!(
            review_diff_source(&range_args).unwrap(),
            rebotica_git::DiffSource::Range("main..HEAD".to_string())
        );

        let Some(Command::Review(cached_args)) =
            Cli::try_parse_from(["rbtc", "review", "--cached"])
                .unwrap()
                .command
        else {
            panic!("expected review command");
        };
        assert_eq!(
            review_diff_source(&cached_args).unwrap(),
            rebotica_git::DiffSource::Cached
        );
    }

    #[test]
    fn review_limit_overrides_parse() {
        let Some(Command::Review(args)) =
            Cli::try_parse_from(["rbtc", "review", "--max-files", "6", "--max-lines", "450"])
                .unwrap()
                .command
        else {
            panic!("expected review command");
        };

        assert_eq!(args.max_files, Some(6));
        assert_eq!(args.max_lines, Some(450));
    }

    #[test]
    fn review_accepts_repeated_model_flags_for_side_by_side_runs() {
        let Some(Command::Review(args)) =
            Cli::try_parse_from(["rbtc", "review", "--model", "gemma", "--model", "qwen"])
                .unwrap()
                .command
        else {
            panic!("expected review command");
        };

        assert_eq!(args.models, vec!["gemma", "qwen"]);
        assert_eq!(
            model_requests(args.models),
            vec![Some("gemma".to_string()), Some("qwen".to_string())]
        );
    }

    #[test]
    fn review_diff_source_flags_conflict() {
        let error =
            Cli::try_parse_from(["rbtc", "review", "--base", "main", "--range", "main..HEAD"])
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

        score(ScoreArgs {
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

        assert_eq!(summary["default"], "lmstudio");
        let providers = summary["providers"].as_array().unwrap();
        assert!(providers.iter().any(|provider| {
            provider["name"] == "lmstudio"
                && provider["base_url"] == "http://127.0.0.1:1234/v1"
                && provider["implicit"] == true
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
