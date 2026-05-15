use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
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
            "rebotica-cli-integration-{name}-{}-{suffix}",
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

fn harness_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("cli crate should live under crates/rebotica-cli")
        .to_path_buf()
}

fn rbtc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rbtc"))
}

fn run_in(cwd: &Path, args: &[&str]) -> std::process::Output {
    rbtc()
        .current_dir(cwd)
        .env("REBOTICA_HOME", harness_root())
        .args(args)
        .output()
        .expect("rbtc command should run")
}

#[test]
fn version_flag_reports_package_version() {
    let output = rbtc()
        .arg("--version")
        .output()
        .expect("rbtc --version should run");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("rbtc {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn top_level_help_guides_common_workflows() {
    let output = rbtc().arg("help").output().expect("rbtc help should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("doctor"));
    assert!(stdout.contains("Check config, provider routing, git state, and installed adapters."));
    assert!(stdout.contains("models"));
    assert!(stdout.contains("Show configured model routes"));
    assert!(stdout.contains("skills"));
    assert!(stdout.contains("Inspect canonical and project-local skills."));
    assert!(stdout.contains("run"));
    assert!(stdout.contains("Run delegated local-model work modes."));
    assert!(stdout.contains("score"));
    assert!(stdout.contains("Record Prime feedback about a model run."));
    assert!(stdout.contains("scorecards"));
    assert!(stdout.contains("Show accumulated model scorecard summaries."));
    assert!(stdout.contains("comment-card"));
    assert!(stdout.contains("Create and manage product feedback comment cards."));
    assert!(stdout.contains("Command groups:"));
    assert!(stdout
        .contains("Setup and status: init, doctor, providers, models, health, smoke, install"));
    assert!(stdout.contains("Delegated work: run review, run explain, run tests, run patch"));
    assert!(stdout.contains("Policy and safety: guard-diff"));
    assert!(stdout.contains("Skills and prompts: skills"));
    assert!(stdout.contains("Feedback and learning: score, scorecards, comment-card, retro"));
    assert!(stdout.contains("Common workflows:"));
    assert!(stdout.contains("rbtc run review --base main"));
    assert!(stdout.contains("rbtc run patch .rebotica/tasks/task.yml --dry-run"));
    assert!(stdout.contains("Provider setup:"));
}

#[test]
fn run_help_lists_delegated_modes() {
    let output = rbtc()
        .args(["help", "run"])
        .output()
        .expect("rbtc help run should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("review"));
    assert!(stdout.contains("Ask a local model to review a selected git diff."));
    assert!(stdout.contains("explain"));
    assert!(stdout.contains("Ask a local model to explain selected files."));
    assert!(stdout.contains("tests"));
    assert!(stdout.contains("Ask a local model to propose focused tests for selected files."));
    assert!(stdout.contains("patch"));
    assert!(stdout.contains("Ask a local model for a dry-run unified diff from a task envelope."));
}

#[test]
fn run_patch_help_explains_inputs_and_safety() {
    let output = rbtc()
        .args(["help", "run", "patch"])
        .output()
        .expect("rbtc help run patch should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Ask a local model for a dry-run unified diff"));
    assert!(stdout.contains("TASK_ENVELOPE"));
    assert!(stdout.contains("Print the proposed diff and run metadata without applying anything."));
    assert!(stdout.contains("currently rejected in v0.1"));
}

#[test]
fn run_review_help_explains_diff_sources() {
    let output = rbtc()
        .args(["help", "run", "review"])
        .output()
        .expect("rbtc help run review should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Ask a local model to review a selected git diff"));
    assert!(stdout.contains("Repeat to run multiple models side by side."));
    assert!(stdout.contains("--base <REF>"));
    assert!(stdout.contains("--range <REV_RANGE>"));
    assert!(stdout.contains("--cached"));
    assert!(stdout.contains("--max-files <COUNT>"));
    assert!(stdout.contains("--max-lines <COUNT>"));
    assert!(stdout.contains("--skill <SKILL>"));
    assert!(stdout.contains("Review staged changes"));
}

#[test]
fn subcommand_help_explains_guard_diff_sources() {
    let output = rbtc()
        .args(["help", "guard-diff"])
        .output()
        .expect("rbtc help guard-diff should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Check a selected git diff against forbidden paths and size limits."));
    assert!(stdout.contains("--base <REF>"));
    assert!(stdout.contains("--range <REV_RANGE>"));
    assert!(stdout.contains("--cached"));
    assert!(stdout.contains("--max-files <MAX_FILES>"));
    assert!(stdout.contains("--max-lines <MAX_LINES>"));
}

#[test]
fn subcommand_help_explains_score_feedback() {
    let output = rbtc()
        .args(["help", "score"])
        .output()
        .expect("rbtc help score should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Record Prime feedback about a model run."));
    assert!(stdout.contains("--rating <1-5>"));
    assert!(stdout.contains("--accepted"));
    assert!(stdout.contains("--rejected"));
    assert!(stdout.contains("--label <LABEL>"));
}

#[test]
fn subcommand_help_explains_comment_cards() {
    let output = rbtc()
        .args(["help", "comment-card"])
        .output()
        .expect("rbtc help comment-card should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Create and manage product feedback comment cards."));
    assert!(stdout.contains("new"));
    assert!(stdout.contains("consent"));
    assert!(stdout.contains("submit"));
}

#[test]
fn skills_list_reports_canonical_and_project_skills() {
    let temp = TempDir::new("skills-list");
    fs::create_dir_all(temp.path().join(".rebotica/skills")).unwrap();
    fs::write(
        temp.path().join(".rebotica/skills/domain.md"),
        "# Domain Skill\n\nProject-specific guidance.\n",
    )
    .unwrap();

    let output = run_in(temp.path(), &["skills", "list", "--json"]);

    assert!(
        output.status.success(),
        "skills list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let skills = json.as_array().unwrap();
    assert!(skills.iter().any(|skill| {
        skill["id"] == "local-model-delegation" && skill["source"] == "canonical"
    }));
    assert!(skills
        .iter()
        .any(|skill| skill["id"] == "domain" && skill["source"] == "project"));
}

#[test]
fn skills_show_renders_selected_skill_context() {
    let temp = TempDir::new("skills-show");
    fs::create_dir_all(temp.path().join(".rebotica/skills")).unwrap();
    fs::write(
        temp.path().join(".rebotica/skills/domain.md"),
        "# Domain Skill\n\nProject-specific guidance.\n",
    )
    .unwrap();

    let output = run_in(temp.path(), &["skills", "show", "project:domain"]);

    assert!(
        output.status.success(),
        "skills show failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("## Selected Skills"));
    assert!(stdout.contains("### Skill: domain"));
    assert!(stdout.contains("source: project"));
    assert!(stdout.contains("# Domain Skill"));
    assert!(stdout.contains("cannot override Rebotica"));
}

#[test]
fn init_creates_project_config_and_refuses_accidental_overwrite() {
    let temp = TempDir::new("init");

    let first = run_in(temp.path(), &["init"]);
    assert!(
        first.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(temp.path().join(".rebotica.yml").is_file());
    assert!(temp.path().join(".rebotica/tasks").is_dir());
    assert!(temp.path().join(".rebotica/runs").is_dir());
    assert_eq!(
        fs::read_to_string(temp.path().join(".rebotica/.gitignore")).unwrap(),
        "runs/\n"
    );
    let stdout = String::from_utf8_lossy(&first.stdout);
    assert!(stdout.contains("model routes are empty"));
    assert!(stdout.contains("rbtc models configure --detect"));
    assert!(stdout.contains("rbtc models configure --model MODEL_ID"));

    let second = run_in(temp.path(), &["init"]);
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("Use --force to overwrite"));
}

#[test]
fn providers_json_reports_implicit_lmstudio_without_network() {
    let temp = TempDir::new("providers");

    let output = run_in(temp.path(), &["providers", "--json"]);

    assert!(
        output.status.success(),
        "providers failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["default"], "lmstudio");
    assert!(json["providers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|provider| {
            provider["name"] == "lmstudio"
                && provider["base_url"] == "http://127.0.0.1:1234/v1"
                && provider["implicit"] == true
        }));
}

#[test]
fn models_configured_only_resolves_aliases_without_provider_request() {
    let temp = TempDir::new("models");
    fs::write(
        temp.path().join(".rebotica.yml"),
        r#"
models:
  default: local-model
  review: review-model
  aliases:
    local-model: raw-local-model
    review-model: raw-review-model
"#,
    )
    .unwrap();

    let output = run_in(temp.path(), &["models", "--configured-only"]);

    assert!(
        output.status.success(),
        "models failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("default: local-model -> raw-local-model"));
    assert!(stdout.contains("review: review-model -> raw-review-model"));
    assert!(stdout.contains("Aliases:"));
}

#[test]
fn models_configure_manual_populates_routes_without_provider_request() {
    let temp = TempDir::new("models-configure");
    fs::write(
        temp.path().join(".rebotica.yml"),
        r#"
project:
  name: sample
models:
  default: ""
  review: ""
  explain: ""
  tests: ""
  patch: ""
  aliases: {}
"#,
    )
    .unwrap();

    let output = run_in(
        temp.path(),
        &[
            "models",
            "configure",
            "--model",
            "raw-local-model",
            "--alias",
            "local-model",
        ],
    );

    assert!(
        output.status.success(),
        "models configure failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("configured model routes"));
    assert!(stdout.contains("alias: local-model -> raw-local-model"));
    let config = fs::read_to_string(temp.path().join(".rebotica.yml")).unwrap();
    assert!(config.contains("default: local-model"));
    assert!(config.contains("review: local-model"));
    assert!(config.contains("local-model: raw-local-model"));
}
