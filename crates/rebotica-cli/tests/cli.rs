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
  default: local-worker
  review: review-worker
  aliases:
    local-worker: raw-local-model
    review-worker: raw-review-model
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
    assert!(stdout.contains("default: local-worker -> raw-local-model"));
    assert!(stdout.contains("review: review-worker -> raw-review-model"));
    assert!(stdout.contains("Aliases:"));
}
