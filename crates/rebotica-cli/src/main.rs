use anyhow::{anyhow, Context, Result};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use rebotica_core::{
    model_for_mode, parse_allowed_files_from_envelope, parse_forbidden_files_from_envelope,
    resolve_model_alias, LoadedConfig, ProjectConfig, TaskEnvelope, WorkerMode,
};
use rebotica_provider::{
    ChatMessage, OpenAICompatibleProvider, ProviderOverrides, ProviderSettings,
};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

#[derive(Debug, Parser)]
#[command(name = "rbtc", version)]
#[command(about = "A governed local-worker harness for collaborative software craftsmanship.")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Doctor(DoctorArgs),
    Models(ModelsArgs),
    Providers(ProvidersArgs),
    Health(ProviderArgs),
    Smoke(SmokeArgs),
    Init(InitArgs),
    Install(InstallArgs),
    Review(ReviewArgs),
    Explain(FileWorkerArgs),
    Tests(FileWorkerArgs),
    Patch(PatchArgs),
    GuardDiff(GuardDiffArgs),
    Retro(RetroArgs),
}

#[derive(Debug, Parser, Clone, Default)]
struct ProviderArgs {
    #[arg(long)]
    provider: Option<String>,
    #[arg(long)]
    base_url: Option<String>,
}

#[derive(Debug, Parser)]
struct SmokeArgs {
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, default_value_t = 0.0)]
    temperature: f64,
}

#[derive(Debug, Parser)]
struct InitArgs {
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Parser)]
struct DoctorArgs {
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct ModelsArgs {
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(long)]
    configured_only: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct ProvidersArgs {
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Parser)]
struct InstallArgs {
    target: InstallTarget,
    #[arg(long)]
    copy: bool,
    #[arg(long)]
    force: bool,
    #[arg(long)]
    target_dir: Option<String>,
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
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    goal: Option<String>,
    #[arg(long, default_value = "medium")]
    risk: String,
    #[arg(long, default_value_t = 0.0)]
    temperature: f64,
}

#[derive(Debug, Parser)]
struct FileWorkerArgs {
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(long)]
    model: Option<String>,
    #[arg(long)]
    goal: Option<String>,
    #[arg(long, default_value_t = 0.0)]
    temperature: f64,
    files: Vec<String>,
}

#[derive(Debug, Parser)]
struct PatchArgs {
    #[command(flatten)]
    provider: ProviderArgs,
    #[arg(long)]
    model: Option<String>,
    #[arg(long, default_value_t = 0.0)]
    temperature: f64,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    apply: bool,
    envelope: String,
}

#[derive(Debug, Parser)]
struct GuardDiffArgs {
    #[arg(long)]
    max_files: Option<usize>,
    #[arg(long)]
    max_lines: Option<usize>,
}

#[derive(Debug, Parser)]
struct RetroArgs {
    #[arg(long)]
    force: bool,
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
        Command::Review(args) => review(args).await,
        Command::Explain(args) => explain(args).await,
        Command::Tests(args) => propose_tests(args).await,
        Command::Patch(args) => propose_patch(args).await,
        Command::GuardDiff(args) => guard_diff(args),
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
                "missing; configure models.default or pass --model",
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

async fn review(args: ReviewArgs) -> Result<()> {
    rebotica_git::assert_repository()?;
    let cwd = std::env::current_dir()?;
    let loaded = LoadedConfig::read_from(&cwd)?;
    let changed_files = rebotica_git::changed_files()?;
    let envelope = TaskEnvelope::for_config(
        rebotica_runlog::make_id(),
        "review",
        args.goal.unwrap_or_else(|| {
            "Review the current git diff for correctness, risk, and missing tests.".to_string()
        }),
        &loaded,
        changed_files,
        "json",
        args.risk,
    );
    let envelope_yaml = envelope.to_yaml()?;
    let prompt = [
        read_harness_file("prompts/system/local-reviewer.md")?,
        read_harness_file("prompts/contracts/review-only.md")?,
        format!("## Task Envelope\n{envelope_yaml}"),
        format!("## Project Config\n{}", loaded.raw_or_placeholder()),
        format!(
            "## Repository Instructions\n{}",
            collect_instruction_files(&cwd)?
        ),
        format!(
            "## Git Status\n{}",
            fenced(&rebotica_git::status_short()?, "text")
        ),
        format!(
            "## Git Diff Stat\n{}",
            fenced(&rebotica_git::diff_stat()?, "text")
        ),
        format!(
            "## Git Diff\n{}",
            fenced(&truncate(&rebotica_git::diff()?, 120_000), "diff")
        ),
    ]
    .join("\n\n");
    let (model, text) = run_worker(
        &loaded,
        WorkerMode::Review,
        args.model,
        args.provider,
        args.temperature,
        prompt.clone(),
    )
    .await?;
    rebotica_runlog::persist("review", &model, &envelope_yaml, &prompt, &text)?;
    println!("{text}");
    Ok(())
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
    let prompt = [
        read_harness_file(system_prompt)?,
        format!("## Task Envelope\n{envelope_yaml}"),
        file_blocks,
    ]
    .join("\n\n");
    let (model, text) = run_worker(
        &loaded,
        mode,
        args.model,
        args.provider,
        args.temperature,
        prompt.clone(),
    )
    .await?;
    rebotica_runlog::persist(envelope_mode, &model, &envelope_yaml, &prompt, &text)?;
    println!("{text}");
    Ok(())
}

async fn propose_patch(args: PatchArgs) -> Result<()> {
    rebotica_git::assert_repository()?;
    let cwd = std::env::current_dir()?;
    let loaded = LoadedConfig::read_from(&cwd)?;
    let envelope_path = cwd.join(&args.envelope);
    let envelope_text = fs::read_to_string(&envelope_path)
        .with_context(|| format!("failed to read {}", envelope_path.display()))?;
    let allowed_files = parse_allowed_files_from_envelope(&envelope_text)?;
    let mut forbidden = loaded.config.forbidden_paths.clone();
    forbidden.extend(parse_forbidden_files_from_envelope(&envelope_text)?);
    rebotica_guard::ensure_allowed(&allowed_files, &forbidden)?;
    let prompt = [
        read_harness_file("prompts/system/local-worker.md")?,
        read_harness_file("prompts/contracts/patch-only.md")?,
        format!("## Task Envelope\n{envelope_text}"),
        format!("## Project Config\n{}", loaded.raw_or_placeholder()),
        format!(
            "## Current Context\n{}",
            collect_files_for_envelope(&cwd, &allowed_files)?
        ),
    ]
    .join("\n\n");
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
    if args.dry_run || !args.apply {
        println!("{text}");
        println!(
            "\ndry_run: true\nrun_id: {}\nnext_step: review the unified diff before applying it",
            run.id
        );
        return Ok(());
    }
    Err(anyhow!(
        "direct patch application is intentionally disabled in v0.1. Review the run output and apply manually."
    ))
}

fn guard_diff(args: GuardDiffArgs) -> Result<()> {
    rebotica_git::assert_repository()?;
    let loaded = LoadedConfig::read_from(&std::env::current_dir()?)?;
    let changed = rebotica_git::changed_files()?;
    let changed_lines = rebotica_git::changed_line_count()?;
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
            "changed_files": changed.len(),
            "changed_lines": changed_lines
        }))?
    );
    Ok(())
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
            "missing model. Pass --model, set REBOTICA_MODEL, or configure models.default in .rebotica.yml."
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
    use std::fs;
    use std::path::{Path, PathBuf};
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
