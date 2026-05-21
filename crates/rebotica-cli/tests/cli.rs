use serde_json::Value;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
        .env_remove("REBOTICA_JSON")
        .env_remove("REBOTICA_QUIET")
        .args(args)
        .output()
        .expect("rbtc command should run")
}

fn run_in_env(cwd: &Path, args: &[&str], envs: &[(&str, &str)]) -> std::process::Output {
    let mut command = rbtc();
    command
        .current_dir(cwd)
        .env("REBOTICA_HOME", harness_root())
        .env_remove("REBOTICA_JSON")
        .env_remove("REBOTICA_QUIET")
        .args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().expect("rbtc command should run")
}

fn assert_json_error(output: &std::process::Output, exit_code: i32, error_code: &str) -> Value {
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(exit_code));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], error_code);
    json
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("git command should run");
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_git_repo(cwd: &Path) {
    run_git(cwd, &["init"]);
    run_git(cwd, &["config", "user.name", "Rebotica Test"]);
    run_git(cwd, &["config", "user.email", "rebotica@example.test"]);
    run_git(cwd, &["config", "commit.gpgsign", "false"]);
    fs::write(cwd.join("README.md"), "initial\n").unwrap();
    run_git(cwd, &["add", "."]);
    run_git(cwd, &["commit", "-m", "initial"]);
}

fn one_shot_models_server(models: &[&str]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server addr should resolve");
    let body = format!(
        r#"{{"data":[{}]}}"#,
        models
            .iter()
            .map(|model| format!(r#"{{"id":"{model}"}}"#))
            .collect::<Vec<_>>()
            .join(",")
    );
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("test server should accept");
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer);
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("test server should respond");
    });
    format!("http://{addr}/v1")
}

fn one_shot_models_status_server(status: u16, body: &str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server addr should resolve");
    let body = body.to_string();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("test server should accept");
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer);
        let response = format!(
            "HTTP/1.1 {status} Provider Error\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("test server should respond");
    });
    format!("http://{addr}/v1")
}

fn one_shot_chat_server(response_text: &str) -> String {
    one_shot_chat_server_with_usage(response_text, None)
}

/// Like `one_shot_chat_server` but also embeds an OpenAI-style `usage`
/// block in the response, so tests can verify token-accounting capture
/// through the dispatch pipeline into the ledger row.
fn one_shot_chat_server_with_usage(
    response_text: &str,
    usage: Option<(u64, u64)>,
) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server addr should resolve");
    let usage_fragment = match usage {
        Some((prompt, completion)) => format!(
            r#","usage":{{"prompt_tokens":{prompt},"completion_tokens":{completion}}}"#
        ),
        None => String::new(),
    };
    let body = format!(
        r#"{{"choices":[{{"message":{{"content":{}}}}}]{usage_fragment}}}"#,
        serde_json::to_string(response_text).unwrap()
    );
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("test server should accept");
        let mut buffer = [0_u8; 2048];
        let _ = stream.read(&mut buffer);
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("test server should respond");
    });
    format!("http://{addr}/v1")
}

/// Serve a fixed sequence of canned chat responses, one per connection.
/// The listener keeps accepting until every response has been served, then
/// drops. Tests that want N sequential dispatches against a single base_url
/// (e.g. `rbtc compare`) use this so each model call sees the same provider.
fn n_shot_chat_server(responses: Vec<String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server addr should resolve");
    thread::spawn(move || {
        for response_text in responses {
            let (mut stream, _) = listener.accept().expect("test server should accept");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let body = format!(
                r#"{{"choices":[{{"message":{{"content":{}}}}}]}}"#,
                serde_json::to_string(&response_text).unwrap()
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .expect("test server should respond");
        }
    });
    format!("http://{addr}/v1")
}

fn one_shot_chat_capture_server(response_text: &str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server addr should resolve");
    let body = format!(
        r#"{{"choices":[{{"message":{{"content":{}}}}}]}}"#,
        serde_json::to_string(response_text).unwrap()
    );
    let (request_tx, request_rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("test server should accept");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        let mut expected_len = None;
        loop {
            let read = stream.read(&mut buffer).expect("test server should read");
            if read == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..read]);
            if expected_len.is_none() {
                if let Some(header_end) = find_headers_end(&request) {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_len = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    expected_len = Some(header_end + 4 + content_len);
                }
            }
            if expected_len
                .map(|expected| request.len() >= expected)
                .unwrap_or(false)
            {
                break;
            }
        }
        let _ = request_tx.send(String::from_utf8_lossy(&request).to_string());
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .expect("test server should respond");
    });
    (format!("http://{addr}/v1"), request_rx)
}

fn find_headers_end(request: &[u8]) -> Option<usize> {
    request.windows(4).position(|window| window == b"\r\n\r\n")
}

fn fenced_json(value: Value) -> String {
    format!("```json\n{}\n```", serde_json::to_string(&value).unwrap())
}

fn unavailable_base_url() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server addr should resolve");
    drop(listener);
    format!("http://{addr}/v1")
}

#[cfg(unix)]
fn blocking_models_server() -> (String, mpsc::Receiver<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server addr should resolve");
    let (accepted_tx, accepted_rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("test server should accept");
        let _ = accepted_tx.send(());
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer);
        thread::sleep(Duration::from_secs(30));
    });
    (format!("http://{addr}/v1"), accepted_rx)
}

fn wait_for_child(mut child: std::process::Child) -> std::process::Output {
    let started = SystemTime::now();
    loop {
        if child
            .try_wait()
            .expect("child status should be readable")
            .is_some()
        {
            return child
                .wait_with_output()
                .expect("child output should be readable");
        }
        assert!(
            started.elapsed().unwrap_or_default() < Duration::from_secs(10),
            "child did not exit before timeout"
        );
        thread::sleep(Duration::from_millis(25));
    }
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
    let temp = TempDir::new("run-help");
    let output = run_in(temp.path(), &["run", "missing-mode", "--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("unknown run mode: missing-mode"));
    assert!(stdout.contains("Available run modes:"));
    assert!(stdout.contains("review"));
    assert!(stdout.contains("Review a diff for correctness"));
    assert!(stdout.contains("explain"));
    assert!(stdout.contains("Explain selected files"));
    assert!(stdout.contains("tests"));
    assert!(stdout.contains("Propose focused missing tests"));
    assert!(stdout.contains("patch"));
    assert!(stdout.contains("Propose a dry-run unified diff"));
}

#[test]
fn run_patch_help_explains_inputs_and_safety() {
    let temp = TempDir::new("run-patch-help");
    let output = run_in(temp.path(), &["run", "patch", "--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Propose a dry-run unified diff"));
    assert!(stdout.contains("TASK_ENVELOPE"));
    assert!(stdout.contains("Preserve dry-run patch behavior"));
    assert!(stdout.contains("Rejected; direct application is disabled"));
}

#[test]
fn run_review_help_explains_diff_sources() {
    let temp = TempDir::new("run-review-help");
    let output = run_in(temp.path(), &["run", "review", "--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Review a diff for correctness"));
    assert!(stdout.contains("Model alias or raw provider model id"));
    assert!(stdout.contains("--base <REF>"));
    assert!(stdout.contains("--range <REV_RANGE>"));
    assert!(stdout.contains("--cached"));
    assert!(stdout.contains("--max-files <COUNT>"));
    assert!(stdout.contains("--max-lines <COUNT>"));
    assert!(stdout.contains("--skill <SKILL>"));
    assert!(stdout.contains("Review staged changes"));
}

#[test]
fn run_dynamic_help_bypasses_quiet_envelope() {
    let temp = TempDir::new("run-help-quiet");

    let output = run_in(temp.path(), &["run", "review", "--quiet", "--help"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Code Review"));
    assert!(!stdout.contains("\"rebotica\""));
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
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "skills.list");
    assert_eq!(json["command"], "skills list");
    assert_eq!(json["ok"], true);
    let skills = json["data"]["skills"].as_array().unwrap();
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
    assert_eq!(second.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&second.stderr).contains("Use --force to overwrite"));
}

#[test]
fn init_human_output_marks_existing_paths_as_skipped() {
    let temp = TempDir::new("init-skipped");
    fs::create_dir_all(temp.path().join(".rebotica/tasks")).unwrap();
    fs::create_dir_all(temp.path().join(".rebotica/runs")).unwrap();

    let output = run_in(temp.path(), &["init"]);

    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout
        .lines()
        .any(|line| { line.starts_with("skipped ") && line.contains(".rebotica/tasks") }));
    assert!(stdout
        .lines()
        .any(|line| { line.starts_with("skipped ") && line.contains(".rebotica/runs") }));
    assert!(!stdout
        .lines()
        .any(|line| { line.starts_with("created ") && line.contains(".rebotica/tasks") }));
    assert!(!stdout
        .lines()
        .any(|line| { line.starts_with("created ") && line.contains(".rebotica/runs") }));
}

#[test]
fn successful_parse_uses_clap_output_flags_not_raw_token_scan() {
    let temp = TempDir::new("title-like-json-flag");
    let home = temp.path().to_string_lossy().to_string();

    let output = run_in_env(
        temp.path(),
        &["comment-card", "new", "--title=--json"],
        &[("HOME", &home)],
    );

    assert!(
        output.status.success(),
        "comment-card new failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("created comment card:"));
    assert!(!stdout.contains("\"rebotica\""));
}

#[test]
fn init_json_emits_v1_envelope_with_written_paths() {
    let temp = TempDir::new("init-json");

    let output = run_in(temp.path(), &["init", "--json"]);

    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "init");
    assert_eq!(json["command"], "init");
    assert_eq!(json["ok"], true);
    assert!(json["data"]["written"]
        .as_array()
        .unwrap()
        .iter()
        .any(|path| path.as_str().unwrap().ends_with(".rebotica.yml")));
    assert_eq!(json["data"]["model_routes_empty"], true);
}

#[test]
fn install_human_output_groups_actions_by_target() {
    let temp = TempDir::new("install-human");

    let output = run_in(temp.path(), &["install", "codex", "--copy"]);

    assert!(
        output.status.success(),
        "install failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Installed codex adapter assets:"));
    assert!(stdout.contains("  copied Rebotica skills into"));
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
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "providers");
    assert_eq!(json["command"], "providers");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["default"], "lmstudio");
    assert!(json["data"]["providers"]
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
fn providers_quiet_emits_single_envelope_to_stdout_nothing_on_stderr() {
    let temp = TempDir::new("providers-quiet");

    let output = run_in(temp.path(), &["providers", "--quiet"]);

    assert!(
        output.status.success(),
        "providers failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("\"rebotica\"").count(), 1);
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "providers");
}

#[test]
fn health_quiet_emits_single_envelope_to_stdout_nothing_on_stderr() {
    let temp = TempDir::new("health-quiet");
    let base_url = one_shot_models_server(&["local-model"]);

    let output = run_in(temp.path(), &["health", "--quiet", "--base-url", &base_url]);

    assert!(
        output.status.success(),
        "health failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("\"rebotica\"").count(), 1);
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "health");
    assert_eq!(json["command"], "health");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["provider"], "lmstudio");
    assert_eq!(json["data"]["base_url"], base_url);
    assert_eq!(json["data"]["model_count"], 1);
    assert_eq!(json["data"]["models"], serde_json::json!(["local-model"]));
    assert!(json["error"].is_null());
}

#[test]
fn smoke_quiet_emits_single_envelope_to_stdout_nothing_on_stderr() {
    let temp = TempDir::new("smoke-quiet");
    let base_url = one_shot_chat_server("LOCAL_OK\n");

    let output = run_in(
        temp.path(),
        &[
            "smoke",
            "--quiet",
            "--base-url",
            &base_url,
            "--model",
            "local-model",
        ],
    );

    assert!(
        output.status.success(),
        "smoke failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("\"rebotica\"").count(), 1);
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "smoke");
    assert_eq!(json["command"], "smoke");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["provider"], "lmstudio");
    assert_eq!(json["data"]["base_url"], base_url);
    assert_eq!(json["data"]["model"], "local-model");
    assert_eq!(
        json["data"]["probe_prompt"],
        serde_json::json!([
            {
                "role": "system",
                "content": "Reply exactly with LOCAL_OK and no other text."
            },
            {
                "role": "user",
                "content": "Reply with LOCAL_OK only."
            }
        ])
    );
    assert_eq!(json["data"]["response"], "LOCAL_OK");
    assert!(json["error"].is_null());
}

#[test]
fn run_built_in_modes_emit_valid_envelopes_and_artifacts() {
    let cases = [
        (
            "review",
            vec!["review"],
            serde_json::json!({
                "assumptions": [],
                "confidence": 8,
                "risks": [],
                "next_action": "review findings",
                "findings": []
            }),
            "findings",
        ),
        (
            "explain",
            vec!["explain", "README.md"],
            serde_json::json!({
                "assumptions": [],
                "confidence": 8,
                "risks": [],
                "next_action": "use analysis",
                "analysis": "README responsibilities."
            }),
            "analysis",
        ),
        (
            "tests",
            vec!["tests", "README.md"],
            serde_json::json!({
                "assumptions": [],
                "confidence": 8,
                "risks": [],
                "next_action": "choose tests",
                "proposed_tests": []
            }),
            "proposed_tests",
        ),
        (
            "patch",
            vec!["patch", ".rebotica/tasks/task.yml", "--dry-run"],
            serde_json::json!({
                "assumptions": [],
                "confidence": 8,
                "risks": [],
                "next_action": "review patch",
                "patch": "",
                "files_touched": []
            }),
            "patch",
        ),
    ];

    for (mode, mut mode_args, response, expected_field) in cases {
        let temp = TempDir::new(&format!("run-{mode}"));
        init_git_repo(temp.path());
        fs::create_dir_all(temp.path().join(".rebotica/tasks")).unwrap();
        fs::write(
            temp.path().join(".rebotica/tasks/task.yml"),
            "task_id: test\nmode: patch\ngoal: Change README\nallowed_files:\n  - README.md\nforbidden_files: []\nsensitive_files: []\ncommands_to_run: []\nmax_changed_lines: 50\nmax_files_changed: 1\noutput_format: json\nacceptance_criteria: []\nrisk_level: low\n",
        )
        .unwrap();
        let home = temp.path().join("home");
        let home_s = home.to_string_lossy().to_string();
        let base_url = one_shot_chat_server(&fenced_json(response));
        let mut args = vec!["--quiet", "run"];
        args.append(&mut mode_args);
        args.extend(["--base-url", &base_url, "--model", "local-model"]);

        let output = run_in_env(temp.path(), &args, &[("HOME", &home_s)]);

        assert!(
            output.status.success(),
            "run {mode} failed: {}\nstdout: {}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
        assert_eq!(String::from_utf8_lossy(&output.stderr), "");
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["kind"], format!("run.{mode}"));
        assert_eq!(json["ok"], true);
        assert!(
            json["data"][expected_field].is_array() || json["data"][expected_field].is_string()
        );
        let run_id = json["run_id"].as_str().expect("run_id should be set");
        let run_dir = home.join(".rebotica/runs").join(run_id);
        assert!(run_dir.join("model-response.md").is_file());
        assert!(run_dir.join("parsed-output.json").is_file());
        assert!(run_dir.join("envelope.json").is_file());
    }
}

#[test]
fn run_prompt_assembly_preserves_adapter_block_order() {
    let temp = TempDir::new("run-prompt-order");
    init_git_repo(temp.path());
    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();
    let response = fenced_json(serde_json::json!({
        "assumptions": [],
        "confidence": 8,
        "risks": [],
        "next_action": "use analysis",
        "analysis": "README responsibilities."
    }));
    let (base_url, request_rx) = one_shot_chat_capture_server(&response);

    let output = run_in_env(
        temp.path(),
        &[
            "--quiet",
            "run",
            "explain",
            "README.md",
            "--base-url",
            &base_url,
            "--model",
            "local-model",
        ],
        &[("HOME", &home_s)],
    );

    assert!(
        output.status.success(),
        "run explain failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let request = request_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("captured chat request should arrive");
    let body = request
        .split("\r\n\r\n")
        .nth(1)
        .expect("request should contain a body");
    let body_json: Value = serde_json::from_str(body).unwrap();
    let prompt = body_json["messages"][1]["content"].as_str().unwrap();
    let project = prompt.find("## Project Config").unwrap();
    let envelope = prompt.find("## Task Envelope").unwrap();
    let file = prompt.find("## File: README.md").unwrap();
    let mode_prompt = prompt.find("# File Explanation Mode").unwrap();
    assert!(project < envelope);
    assert!(envelope < file);
    assert!(file < mode_prompt);
}

#[test]
fn run_output_invalid_persists_raw_and_failure_details() {
    let temp = TempDir::new("run-output-invalid-parse");
    init_git_repo(temp.path());
    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();
    let base_url = one_shot_chat_server("not json");

    let output = run_in_env(
        temp.path(),
        &[
            "--quiet",
            "run",
            "review",
            "--base-url",
            &base_url,
            "--model",
            "local-model",
        ],
        &[("HOME", &home_s)],
    );

    assert_eq!(output.status.code(), Some(21));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "output_invalid");
    assert_eq!(json["error"]["details"]["mode"], "review");
    assert_eq!(json["error"]["details"]["extraction"], "fallback");
    let run_id = json["run_id"].as_str().expect("run_id should be set");
    let run_dir = home.join(".rebotica/runs").join(run_id);
    assert!(run_dir.join("model-response.md").is_file());
    assert!(run_dir.join("parse-failure.json").is_file());
    assert!(run_dir.join("envelope.json").is_file());
    assert!(!run_dir.join("parsed-output.json").exists());
}

#[test]
fn run_validation_failure_reports_schema_errors() {
    let temp = TempDir::new("run-output-invalid-validation");
    init_git_repo(temp.path());
    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();
    let response = fenced_json(serde_json::json!({
        "assumptions": [],
        "confidence": 11,
        "risks": [],
        "next_action": "review findings",
        "findings": []
    }));
    let base_url = one_shot_chat_server(&response);

    let output = run_in_env(
        temp.path(),
        &[
            "--quiet",
            "run",
            "review",
            "--base-url",
            &base_url,
            "--model",
            "local-model",
        ],
        &[("HOME", &home_s)],
    );

    assert_eq!(output.status.code(), Some(21));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "output_invalid");
    assert_eq!(json["error"]["details"]["extraction"], "fence");
    assert!(json["error"]["details"]["validation_errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| { error["instance_path"] == "/confidence" && error["keyword"] == "maximum" }));
}

#[test]
fn run_provider_failure_persists_typed_failure_without_raw_response() {
    let temp = TempDir::new("run-provider-failure");
    init_git_repo(temp.path());
    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();
    let base_url = unavailable_base_url();

    let output = run_in_env(
        temp.path(),
        &[
            "--quiet",
            "run",
            "review",
            "--base-url",
            &base_url,
            "--model",
            "local-model",
        ],
        &[("HOME", &home_s)],
    );

    assert_eq!(output.status.code(), Some(10));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "provider_unavailable");
    let run_id = json["run_id"].as_str().expect("run_id should be set");
    let run_dir = home.join(".rebotica/runs").join(run_id);
    assert!(run_dir.join("provider-failure.json").is_file());
    assert!(run_dir.join("envelope.json").is_file());
    assert!(!run_dir.join("model-response.md").exists());
}

#[test]
fn broken_project_layer_falls_through_with_warning_suppressed_by_quiet() {
    let temp = TempDir::new("run-broken-layer");
    init_git_repo(temp.path());
    fs::create_dir_all(temp.path().join(".rebotica/runs.d/review")).unwrap();
    fs::write(
        temp.path().join(".rebotica/runs.d/review/manifest.yml"),
        "kind: run.review\n",
    )
    .unwrap();
    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();
    let response = fenced_json(serde_json::json!({
        "assumptions": [],
        "confidence": 8,
        "risks": [],
        "next_action": "review findings",
        "findings": []
    }));

    let base_url = one_shot_chat_server(&response);
    let output = run_in_env(
        temp.path(),
        &[
            "--json",
            "run",
            "review",
            "--base-url",
            &base_url,
            "--model",
            "local-model",
        ],
        &[("HOME", &home_s)],
    );
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("warning: plugin"));

    let base_url = one_shot_chat_server(&response);
    let output = run_in_env(
        temp.path(),
        &[
            "--quiet",
            "run",
            "review",
            "--base-url",
            &base_url,
            "--model",
            "local-model",
        ],
        &[("HOME", &home_s)],
    );
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn run_fallback_json_extraction_warns_under_json_mode() {
    let temp = TempDir::new("run-fallback-extraction");
    init_git_repo(temp.path());
    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();
    let response = "analysis first {\"assumptions\":[],\"confidence\":8,\"risks\":[],\"next_action\":\"review\",\"findings\":[]}";
    let base_url = one_shot_chat_server(response);

    let output = run_in_env(
        temp.path(),
        &[
            "--json",
            "run",
            "review",
            "--base-url",
            &base_url,
            "--model",
            "local-model",
        ],
        &[("HOME", &home_s)],
    );

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("used the last balanced"));

    let base_url = one_shot_chat_server(response);
    let output = run_in_env(
        temp.path(),
        &[
            "--quiet",
            "run",
            "review",
            "--base-url",
            &base_url,
            "--model",
            "local-model",
        ],
        &[("HOME", &home_s)],
    );

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn run_unknown_adapter_argument_is_usage_error() {
    let temp = TempDir::new("run-unknown-arg");
    init_git_repo(temp.path());

    let output = run_in(temp.path(), &["--quiet", "run", "review", "--typo"]);

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "usage");
    // Pre-persistence rejections record a `run_rejected` ledger event
    // (see #59) and surface its id so callers can pivot to `rbtc runs
    // show <id>` for context. The id must not be null.
    let run_id = json["run_id"]
        .as_str()
        .expect("rejection should populate run_id");
    assert!(!run_id.is_empty(), "run_id should be non-empty");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown argument for run review: --typo"));
}

#[test]
fn run_review_require_fresh_base_rejects_stale_branch() {
    // #26: a branch behind its base, reviewed with --require-fresh-base,
    // must be rejected before the model runs (exit 20, guard_rejected).
    let temp = TempDir::new("stale-base-strict");
    init_git_repo(temp.path());
    run_git(temp.path(), &["branch", "-M", "main"]);
    run_git(temp.path(), &["checkout", "-b", "feature"]);
    fs::write(temp.path().join("feature.rs"), "fn f() {}\n").unwrap();
    run_git(temp.path(), &["add", "."]);
    run_git(temp.path(), &["commit", "-m", "feature work"]);
    // main moves on after the branch forked.
    run_git(temp.path(), &["checkout", "main"]);
    fs::write(temp.path().join("landed.rs"), "fn l() {}\n").unwrap();
    run_git(temp.path(), &["add", "."]);
    run_git(temp.path(), &["commit", "-m", "landed on main"]);
    run_git(temp.path(), &["checkout", "feature"]);

    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();
    let output = run_in_env(
        temp.path(),
        &[
            "--json", "run", "review", "--base", "main", "--require-fresh-base", "--model",
            "local-model",
        ],
        &[("HOME", &home_s)],
    );

    assert_eq!(
        output.status.code(),
        Some(20),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["ok"], serde_json::json!(false));
    assert_eq!(json["error"]["code"], "guard_rejected");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("not in this branch"));
    assert_eq!(json["error"]["details"]["reason"], "stale_base");
}

#[test]
fn guard_diff_quiet_emits_single_envelope_to_stdout_nothing_on_stderr() {
    let temp = TempDir::new("guard-diff-quiet");
    init_git_repo(temp.path());

    let output = run_in(temp.path(), &["guard-diff", "--quiet"]);

    assert!(
        output.status.success(),
        "guard-diff failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("\"rebotica\"").count(), 1);
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "guard-diff");
    assert_eq!(json["command"], "guard-diff");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["changed_files"], 0);
    assert_eq!(json["data"]["changed_lines"], 0);
    assert_eq!(
        json["data"]["effective_forbidden_paths"],
        serde_json::json!([])
    );
    assert!(json["error"].is_null());
}

#[test]
fn health_quiet_provider_unavailable_emits_typed_error_envelope() {
    let temp = TempDir::new("health-provider-unavailable");
    let base_url = unavailable_base_url();

    let output = run_in(temp.path(), &["health", "--quiet", "--base-url", &base_url]);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(10));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "health");
    assert_eq!(json["command"], "health");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "provider_unavailable");
    assert_eq!(json["error"]["details"]["endpoint"], "models");
    assert!(json["error"]["details"]["reason"]
        .as_str()
        .unwrap()
        .contains("error sending request"));
    assert_eq!(json["data"]["provider"], "lmstudio");
    assert_eq!(json["data"]["base_url"], base_url);
    assert_eq!(json["data"]["model_count"], 0);
}

#[test]
fn health_quiet_provider_http_status_emits_provider_server_error_envelope() {
    let temp = TempDir::new("health-provider-server-error");
    let base_url = one_shot_models_status_server(503, "model service overloaded");

    let output = run_in(temp.path(), &["health", "--quiet", "--base-url", &base_url]);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(11));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "health");
    assert_eq!(json["command"], "health");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "provider_server_error");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("HTTP 503"));
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("model service overloaded"));
    assert_eq!(
        json["error"]["details"],
        serde_json::json!({
            "endpoint": "models",
            "http_status": 503,
            "body": "model service overloaded"
        })
    );
    assert_eq!(json["data"]["provider"], "lmstudio");
    assert_eq!(json["data"]["base_url"], base_url);
    assert_eq!(json["data"]["model_count"], 0);
}

#[test]
fn health_quiet_provider_http_status_4xx_emits_provider_client_error_envelope() {
    let temp = TempDir::new("health-provider-client-error");
    let base_url = one_shot_models_status_server(401, "missing api key");

    let output = run_in(temp.path(), &["health", "--quiet", "--base-url", &base_url]);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(12));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "health");
    assert_eq!(json["command"], "health");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "provider_client_error");
    assert_eq!(
        json["error"]["details"],
        serde_json::json!({
            "endpoint": "models",
            "http_status": 401,
            "body": "missing api key"
        })
    );
    assert_eq!(json["data"]["provider"], "lmstudio");
    assert_eq!(json["data"]["base_url"], base_url);
    assert_eq!(json["data"]["model_count"], 0);
}

#[test]
fn guard_diff_quiet_guard_rejected_emits_typed_error_envelope() {
    let temp = TempDir::new("guard-diff-rejected");
    fs::write(
        temp.path().join(".rebotica.yml"),
        "forbidden_paths:\n  - secrets/\n",
    )
    .unwrap();
    fs::create_dir_all(temp.path().join("secrets")).unwrap();
    fs::write(temp.path().join("secrets/key.txt"), "secret\n").unwrap();
    init_git_repo(temp.path());
    fs::write(temp.path().join("secrets/key.txt"), "changed secret\n").unwrap();

    let output = run_in(temp.path(), &["guard-diff", "--quiet"]);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(20));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "guard-diff");
    assert_eq!(json["command"], "guard-diff");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "guard_rejected");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("forbidden by pattern 'secrets/'"));
    assert_eq!(
        json["error"]["details"],
        serde_json::json!({
            "rejected_paths": ["secrets/key.txt"],
            "forbidden_pattern": "secrets/"
        })
    );
    assert_eq!(json["data"]["changed_files"], 1);
    assert_eq!(
        json["data"]["effective_forbidden_paths"],
        serde_json::json!(["secrets/"])
    );
}

#[test]
fn guard_diff_quiet_over_limit_emits_typed_error_envelope() {
    let temp = TempDir::new("guard-diff-over-limit");
    init_git_repo(temp.path());
    fs::write(temp.path().join("README.md"), "initial\nchanged\n").unwrap();

    let output = run_in(temp.path(), &["guard-diff", "--quiet", "--max-lines", "0"]);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(22));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "guard-diff");
    assert_eq!(json["command"], "guard-diff");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "over_limit");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("changed line count"));
    assert_eq!(json["data"]["max_lines"], 0);
    assert!(json["data"]["changed_lines"].as_u64().unwrap() > 0);
    assert_eq!(json["error"]["details"]["kind"], "lines");
    assert_eq!(json["error"]["details"]["limit"], 0);
    assert_eq!(
        json["error"]["details"]["actual"],
        json["data"]["changed_lines"]
    );
    assert_eq!(
        json["data"]["effective_forbidden_paths"],
        serde_json::json!([])
    );
}

#[test]
fn doctor_json_emits_v1_envelope() {
    let temp = TempDir::new("doctor-json");

    let output = run_in(temp.path(), &["doctor", "--json"]);

    assert!(
        output.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "doctor");
    assert_eq!(json["ok"], true);
    assert_eq!(json["command"], "doctor");
    assert!(json["data"]
        .as_array()
        .unwrap()
        .iter()
        .any(|check| { check["id"] == "config.parse" && check["status"] == "ok" }));
    assert!(json["error"].is_null());
    assert!(json["started_at"].as_str().unwrap().contains('T'));
    assert!(json["duration_ms"].as_u64().is_some());
}

#[test]
fn doctor_quiet_emits_single_envelope_to_stdout_nothing_on_stderr() {
    let temp = TempDir::new("doctor-quiet");

    let output = run_in(temp.path(), &["doctor", "--quiet"]);

    assert!(
        output.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("\"rebotica\"").count(), 1);
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "doctor");
    assert_eq!(json["ok"], true);
}

#[test]
fn doctor_quiet_failure_emits_error_envelope_no_stderr_noise() {
    let temp = TempDir::new("doctor-quiet-failure");
    fs::write(
        temp.path().join(".rebotica.yml"),
        r#"
default_limits:
  max_changed_lines: 0
  max_files_changed: 0
"#,
    )
    .unwrap();

    let output = run_in(temp.path(), &["doctor", "--quiet"]);

    assert!(!output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "doctor");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "config");
    assert!(json["data"].as_array().unwrap().iter().any(|check| {
        check["id"] == "config.limits.max_changed_lines" && check["status"] == "fail"
    }));
}

#[test]
fn global_json_before_subcommand() {
    let temp = TempDir::new("doctor-global-json");

    let output = run_in(temp.path(), &["--json", "doctor"]);

    assert!(
        output.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "doctor");
}

#[test]
fn global_json_without_subcommand_emits_usage_error_envelope() {
    let temp = TempDir::new("global-json-no-subcommand");

    let output = run_in(temp.path(), &["--json"]);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "error");
    assert_eq!(json["ok"], false);
    assert_eq!(json["command"], "rbtc");
    assert_eq!(json["data"], serde_json::json!({}));
    assert_eq!(json["error"]["code"], "usage");
    assert_eq!(json["error"]["message"], "missing subcommand");
}

#[test]
fn global_quiet_implies_json() {
    let temp = TempDir::new("doctor-global-quiet");

    let output = run_in(temp.path(), &["--quiet", "doctor"]);

    assert!(
        output.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
}

#[test]
fn env_rebotica_json_triggers_json_mode() {
    let temp = TempDir::new("doctor-env-json");

    let output = run_in_env(temp.path(), &["doctor"], &[("REBOTICA_JSON", "true")]);

    assert!(
        output.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "doctor");
}

#[test]
fn env_rebotica_quiet_triggers_quiet_mode() {
    let temp = TempDir::new("doctor-env-quiet");

    let output = run_in_env(temp.path(), &["doctor"], &[("REBOTICA_QUIET", "1")]);

    assert!(
        output.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "doctor");
}

#[test]
fn help_flag_bypasses_json_envelope() {
    // `--help` is not an error. clap's DisplayHelp variant is paired with exit
    // code 0; wrapping it in a `kind: "error"` envelope would produce a self-
    // contradicting `ok: false` + exit 0. Verify help text goes to stdout and
    // no envelope is emitted, even with the global --json flag set.
    let temp = TempDir::new("help-flag-json");

    let output = run_in(temp.path(), &["--json", "--help"]);

    assert!(output.status.success(), "exit code should be 0 for --help");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage: rbtc"),
        "help text should be on stdout"
    );
    assert!(
        !stdout.contains("\"rebotica\""),
        "no envelope should be emitted for --help, got: {stdout}"
    );
}

#[test]
fn version_flag_bypasses_json_envelope() {
    let temp = TempDir::new("version-flag-json");

    let output = run_in(temp.path(), &["--quiet", "--version"]);

    assert!(
        output.status.success(),
        "exit code should be 0 for --version"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.starts_with("rbtc "),
        "version text should be on stdout"
    );
    assert!(
        !stdout.contains("\"rebotica\""),
        "no envelope should be emitted for --version, got: {stdout}"
    );
}

#[test]
fn quiet_parse_failure_emits_error_envelope_no_stderr_noise() {
    let temp = TempDir::new("quiet-parse-failure");

    let output = run_in(temp.path(), &["--quiet", "--definitely-not-a-command"]);

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "error");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "usage");
}

#[test]
fn quiet_migrated_command_failure_emits_command_error_envelope() {
    let temp = TempDir::new("quiet-score-failure");

    let output = run_in(
        temp.path(),
        &["score", "missing-run", "--rating", "6", "--quiet"],
    );

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.matches("\"rebotica\"").count(), 1);
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "score");
    assert_eq!(json["command"], "score");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "usage");
    assert_eq!(json["data"], serde_json::json!({}));
}

#[test]
fn migrated_classifiable_failures_emit_typed_json_errors() {
    let temp = TempDir::new("typed-json-errors");
    let home = temp.path().to_string_lossy().to_string();
    let consent = run_in_env(
        temp.path(),
        &["comment-card", "consent", "--allow-github"],
        &[("HOME", &home)],
    );
    assert!(
        consent.status.success(),
        "comment-card consent failed: {}",
        String::from_utf8_lossy(&consent.stderr)
    );

    for (args, exit_code, error_code, kind, message) in [
        (
            &["skills", "show", "missing", "--json"][..],
            2,
            "usage",
            "skills.show",
            "skill not found: missing",
        ),
        (
            &["skills", "show", "workspace:domain", "--json"][..],
            2,
            "usage",
            "skills.show",
            "unknown skill source 'workspace'. Use canonical:<id> or project:<id>.",
        ),
        (
            &["retro", "missing-run", "--json"][..],
            3,
            "config",
            "retro",
            "run not found: missing-run",
        ),
        (
            &["comment-card", "show", "missing-card", "--json"][..],
            2,
            "usage",
            "comment-card.show",
            "comment card not found: missing-card",
        ),
        (
            &["comment-card", "dismiss", "missing-card", "--json"][..],
            2,
            "usage",
            "comment-card.dismiss",
            "comment card not found in pending: missing-card",
        ),
        (
            &["comment-card", "submit", "missing-card", "--json"][..],
            2,
            "usage",
            "comment-card.submit",
            "pending comment card not found: missing-card",
        ),
    ] {
        let output = run_in_env(temp.path(), args, &[("HOME", &home)]);
        let json = assert_json_error(&output, exit_code, error_code);
        assert_eq!(json["kind"], kind);
        assert_eq!(json["error"]["message"], message);
    }
}

#[test]
fn comment_card_submit_without_consent_json_is_config_error() {
    let temp = TempDir::new("comment-card-submit-no-consent");
    let home = temp.path().to_string_lossy().to_string();

    let output = run_in_env(
        temp.path(),
        &["comment-card", "submit", "missing-card", "--json"],
        &[("HOME", &home)],
    );

    let json = assert_json_error(&output, 3, "config");
    assert_eq!(json["kind"], "comment-card.submit");
    assert_eq!(
        json["error"]["message"],
        "GitHub comment-card submission needs consent. Run: rbtc comment-card consent --allow-github"
    );
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

#[test]
fn models_configure_detect_json_emits_configure_envelope() {
    let temp = TempDir::new("models-configure-detect-json");
    fs::write(
        temp.path().join(".rebotica.yml"),
        r#"
project:
  name: sample
providers:
  default: local
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
    let base_url = one_shot_models_server(&["detected-model"]);

    let output = run_in(
        temp.path(),
        &[
            "models",
            "configure",
            "--detect",
            "--base-url",
            &base_url,
            "--json",
        ],
    );

    assert!(
        output.status.success(),
        "models configure failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "models.configure");
    assert_eq!(json["command"], "models configure");
    assert_eq!(json["ok"], true);
    assert_eq!(json["data"]["status"], "configured");
    assert_eq!(json["data"]["source"], "detected");
    assert_eq!(json["data"]["model"], "detected-model");
}

#[test]
fn models_configure_detect_json_no_models_emits_error_envelope() {
    let temp = TempDir::new("models-configure-detect-no-models-json");
    fs::write(
        temp.path().join(".rebotica.yml"),
        r#"
project:
  name: sample
providers:
  default: local
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
    let base_url = one_shot_models_server(&[]);

    let output = run_in(
        temp.path(),
        &[
            "models",
            "configure",
            "--detect",
            "--base-url",
            &base_url,
            "--json",
        ],
    );

    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(10));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "models.configure");
    assert_eq!(json["command"], "models configure");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "provider_unavailable");
    assert_eq!(json["error"]["message"], "provider returned no models");
    assert_eq!(json["data"]["status"], "no_models");
}

#[cfg(unix)]
#[test]
fn json_command_cancellation_emits_cancelled_envelope_and_exit_code() {
    let temp = TempDir::new("cancel-json");
    let (base_url, accepted_rx) = blocking_models_server();

    let child = rbtc()
        .current_dir(temp.path())
        .env("REBOTICA_HOME", harness_root())
        .env_remove("REBOTICA_JSON")
        .env_remove("REBOTICA_QUIET")
        .args(["models", "--json", "--base-url", &base_url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("rbtc command should spawn");

    accepted_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("provider request should reach test server");
    let status = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("kill should run");
    assert!(status.success(), "kill -INT should succeed");

    let output = wait_for_child(child);

    assert_eq!(output.status.code(), Some(130));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "error");
    assert_eq!(json["command"], "models");
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["code"], "cancelled");
    assert_eq!(json["error"]["message"], "operation cancelled");
}

#[test]
fn comment_card_list_json_emits_nested_command_envelope() {
    let temp = TempDir::new("comment-card-list-json");

    let home = temp.path().to_string_lossy().to_string();
    let created = run_in_env(
        temp.path(),
        &[
            "comment-card",
            "new",
            "--title",
            "Review needs clearer next steps",
        ],
        &[("HOME", &home)],
    );
    assert!(
        created.status.success(),
        "comment-card new failed: {}",
        String::from_utf8_lossy(&created.stderr)
    );

    let output = run_in_env(
        temp.path(),
        &["comment-card", "list", "--json"],
        &[("HOME", &home)],
    );

    assert!(
        output.status.success(),
        "comment-card list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["rebotica"], "v1");
    assert_eq!(json["kind"], "comment-card.list");
    assert_eq!(json["command"], "comment-card list");
    assert_eq!(json["ok"], true);
    assert!(json["data"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .any(|card| card["status"] == "pending"
            && card["title"] == "Review needs clearer next steps"));
}

#[test]
fn compare_runs_each_model_and_emits_aggregated_envelope() {
    let temp = TempDir::new("compare-two-models");
    init_git_repo(temp.path());
    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();

    // Two distinct review responses so we can verify each model row carries
    // its own finding count and confidence — not just the same payload twice.
    let response_a = fenced_json(serde_json::json!({
        "assumptions": [],
        "confidence": 8,
        "risks": [],
        "next_action": "Prime should review the listed findings.",
        "findings": [
            {
                "severity": "major",
                "category": "correctness",
                "file": "src/lib.rs",
                "line": 10,
                "summary": "Off-by-one in loop.",
                "fix": "Use <= instead of <."
            }
        ]
    }));
    let response_b = fenced_json(serde_json::json!({
        "assumptions": [],
        "confidence": 5,
        "risks": [],
        "next_action": "Prime should consider whether the broader pattern needs work.",
        "findings": [
            { "severity": "minor", "summary": "Naming nit one." },
            { "severity": "minor", "summary": "Naming nit two." },
            { "severity": "nit",   "summary": "Style nit." }
        ]
    }));
    let base_url = n_shot_chat_server(vec![response_a, response_b]);

    let output = run_in_env(
        temp.path(),
        &[
            "--json",
            "compare",
            "review",
            "--model",
            "alpha",
            "--model",
            "beta",
            "--base-url",
            &base_url,
        ],
        &[("HOME", &home_s)],
    );

    assert!(
        output.status.success(),
        "compare failed: {}\nstdout: {}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["kind"], "compare.review");
    assert_eq!(json["command"], "compare review");
    assert_eq!(json["data"]["mode"], "review");

    let models = json["data"]["models"].as_array().expect("models array");
    assert_eq!(models.len(), 2, "one row per --model");

    assert_eq!(models[0]["model"], "alpha");
    assert_eq!(models[0]["ok"], true);
    assert_eq!(models[0]["confidence"], 8);
    assert_eq!(models[0]["n_findings"], 1);
    let alpha_id = models[0]["run_id"].as_str().expect("alpha run_id");
    assert!(!alpha_id.is_empty());

    assert_eq!(models[1]["model"], "beta");
    assert_eq!(models[1]["ok"], true);
    assert_eq!(models[1]["confidence"], 5);
    assert_eq!(models[1]["n_findings"], 3);
    let beta_id = models[1]["run_id"].as_str().expect("beta run_id");
    assert_ne!(alpha_id, beta_id, "each model gets its own run_id");

    // Each per-model dispatch persisted its own run directory and envelope.
    for run_id in [alpha_id, beta_id] {
        let run_dir = home.join(".rebotica/runs").join(run_id);
        assert!(
            run_dir.join("envelope.json").is_file(),
            "missing envelope for {run_id}"
        );
        let envelope_text = fs::read_to_string(run_dir.join("envelope.json")).unwrap();
        let envelope: Value = serde_json::from_str(&envelope_text).unwrap();
        // The per-model envelope's command records which slot it came from.
        assert!(envelope["command"]
            .as_str()
            .unwrap()
            .starts_with("compare review --model "));
    }
}

#[test]
fn compare_reports_per_model_failures_alongside_successes() {
    let temp = TempDir::new("compare-mixed");
    init_git_repo(temp.path());
    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();

    // First model returns schema-invalid JSON; second model returns a valid
    // review. The compare envelope should carry one failure row and one
    // success row — failures don't short-circuit subsequent models.
    let bad_response = fenced_json(serde_json::json!({ "totally": "wrong shape" }));
    let good_response = fenced_json(serde_json::json!({
        "assumptions": [],
        "confidence": 7,
        "risks": [],
        "next_action": "ok",
        "findings": []
    }));
    let base_url = n_shot_chat_server(vec![bad_response, good_response]);

    let output = run_in_env(
        temp.path(),
        &[
            "--json",
            "compare",
            "review",
            "--model",
            "broken",
            "--model",
            "ok",
            "--base-url",
            &base_url,
        ],
        &[("HOME", &home_s)],
    );

    assert!(output.status.success(), "compare itself should succeed even when individual models fail");
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let models = json["data"]["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);

    assert_eq!(models[0]["model"], "broken");
    assert_eq!(models[0]["ok"], false);
    assert_eq!(models[0]["error_code"], "output_invalid");
    assert!(models[0]["confidence"].is_null());
    assert!(models[0]["n_findings"].is_null());

    assert_eq!(models[1]["model"], "ok");
    assert_eq!(models[1]["ok"], true);
    assert_eq!(models[1]["error_code"], serde_json::Value::Null);
    assert_eq!(models[1]["confidence"], 7);
}

#[test]
fn compare_with_comma_separated_models_expands_correctly() {
    let temp = TempDir::new("compare-comma");
    init_git_repo(temp.path());
    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();

    let response = fenced_json(serde_json::json!({
        "assumptions": [],
        "confidence": 7,
        "risks": [],
        "next_action": "ok",
        "findings": []
    }));
    let base_url = n_shot_chat_server(vec![response.clone(), response]);

    let output = run_in_env(
        temp.path(),
        &[
            "--json",
            "compare",
            "review",
            "--model",
            "alpha,beta",
            "--base-url",
            &base_url,
        ],
        &[("HOME", &home_s)],
    );

    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let models = json["data"]["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert_eq!(models[0]["model"], "alpha");
    assert_eq!(models[1]["model"], "beta");
}

/// Read the most recent `run_completed` payload for `run_id` out of the
/// ledger db rooted at `home/.rebotica/`. Opens SQLite directly rather
/// than going through the runlog crate's `root()` helper, because that
/// helper consults `HOME` from the test thread's environment — which
/// would race with parallel tests.
fn read_run_completed_payload(home: &Path, run_id: &str) -> Value {
    let ledger_path = home.join(".rebotica/ledger.db");
    let conn = rusqlite::Connection::open(&ledger_path).expect("open ledger db");
    let payload_json: String = conn
        .query_row(
            "SELECT payload_json FROM ledger_events \
             WHERE run_id = ?1 AND event_type = 'run_completed' \
             ORDER BY id DESC LIMIT 1",
            rusqlite::params![run_id],
            |row| row.get(0),
        )
        .expect("run_completed row should exist for run_id");
    serde_json::from_str(&payload_json).expect("payload should be valid JSON")
}

#[test]
fn run_review_captures_apprentice_usage_and_envelope_bytes_in_ledger() {
    let temp = TempDir::new("usage-capture");
    init_git_repo(temp.path());
    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();

    let response = fenced_json(serde_json::json!({
        "assumptions": [],
        "confidence": 7,
        "risks": [],
        "next_action": "Prime should review.",
        "findings": []
    }));
    // Provider reports 1234 prompt tokens and 567 completion tokens —
    // distinct, recognisable numbers so the assertion is unambiguous.
    let base_url = one_shot_chat_server_with_usage(&response, Some((1234, 567)));

    let output = run_in_env(
        temp.path(),
        &[
            "--json",
            "run",
            "review",
            "--base-url",
            &base_url,
            "--model",
            "local-model",
        ],
        &[("HOME", &home_s)],
    );

    assert!(
        output.status.success(),
        "run review failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = json["run_id"].as_str().expect("run_id should be set");

    // Read the run_completed event directly from the ledger db and assert
    // the new fields landed verbatim. Querying the SQLite db is the
    // strongest possible check that the metrics threaded through.
    let payload: Value = read_run_completed_payload(&home, run_id);

    assert_eq!(payload["apprentice_prompt_tokens"], 1234);
    assert_eq!(payload["apprentice_completion_tokens"], 567);
    let envelope_bytes = payload["envelope_bytes"]
        .as_u64()
        .expect("envelope_bytes present");
    assert!(
        envelope_bytes > 20,
        "envelope_bytes implausibly small: {envelope_bytes}"
    );
}

#[test]
fn run_review_omits_usage_when_provider_reports_no_usage_block() {
    let temp = TempDir::new("usage-absent");
    init_git_repo(temp.path());
    let home = temp.path().join("home");
    let home_s = home.to_string_lossy().to_string();

    let response = fenced_json(serde_json::json!({
        "assumptions": [],
        "confidence": 6,
        "risks": [],
        "next_action": "ok",
        "findings": []
    }));
    // No usage block — exercises the `Option::None` codepath for
    // providers (or proxies) that strip `usage` from the response.
    let base_url = one_shot_chat_server(&response);

    let output = run_in_env(
        temp.path(),
        &[
            "--json",
            "run",
            "review",
            "--base-url",
            &base_url,
            "--model",
            "local-model",
        ],
        &[("HOME", &home_s)],
    );

    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let run_id = json["run_id"].as_str().expect("run_id should be set");

    let payload: Value = read_run_completed_payload(&home, run_id);

    // Token fields elided via `skip_serializing_if = Option::is_none`.
    // envelope_bytes is still present because it's computed locally from
    // the parsed value, not reported by the provider.
    assert!(payload.get("apprentice_prompt_tokens").is_none());
    assert!(payload.get("apprentice_completion_tokens").is_none());
    assert!(payload["envelope_bytes"].as_u64().unwrap() > 0);
}
