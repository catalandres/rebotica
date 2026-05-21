use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    WorkingTree,
    Cached,
    Base(String),
    Range(String),
}

impl DiffSource {
    pub fn description(&self) -> String {
        match self {
            Self::WorkingTree => "unstaged working tree changes (git diff)".to_string(),
            Self::Cached => "staged changes (git diff --cached)".to_string(),
            Self::Base(base) => {
                format!("changes from merge-base({base}, HEAD) to HEAD (git diff {base}...HEAD)")
            }
            Self::Range(range) => format!("explicit git diff range (git diff {range})"),
        }
    }

    pub fn validate(&self) -> Result<()> {
        match self {
            Self::WorkingTree | Self::Cached => Ok(()),
            Self::Base(base) => {
                validate_revision_arg("base", base)?;
                if base.contains("..") {
                    return Err(anyhow!(
                        "--base accepts a single ref; use --range for rev ranges"
                    ));
                }
                Ok(())
            }
            Self::Range(range) => validate_revision_arg("range", range),
        }
    }
}

pub fn assert_repository() -> Result<()> {
    assert_repository_in(None)
}

pub fn status_short() -> Result<String> {
    status_short_in(None)
}

pub fn diff() -> Result<String> {
    diff_for(&DiffSource::WorkingTree)
}

pub fn diff_stat() -> Result<String> {
    diff_stat_for(&DiffSource::WorkingTree)
}

pub fn changed_files() -> Result<Vec<String>> {
    changed_files_for(&DiffSource::WorkingTree)
}

pub fn changed_line_count() -> Result<usize> {
    changed_line_count_for(&DiffSource::WorkingTree)
}

pub fn diff_for(source: &DiffSource) -> Result<String> {
    diff_for_in(None, source)
}

pub fn diff_stat_for(source: &DiffSource) -> Result<String> {
    diff_stat_for_in(None, source)
}

pub fn changed_files_for(source: &DiffSource) -> Result<Vec<String>> {
    changed_files_for_in(None, source)
}

pub fn changed_line_count_for(source: &DiffSource) -> Result<usize> {
    changed_line_count_for_in(None, source)
}

/// Detect this repository's trunk branch — the ref the working tree is
/// most likely diverging from. Tries, in order: the remote default branch
/// (`origin/HEAD`), then local `main`, then local `master`. Returns `None`
/// when none resolve (e.g. a fresh repo with no trunk yet).
pub fn detect_trunk() -> Option<String> {
    detect_trunk_in(None)
}

/// Count commits reachable from `HEAD` but not from `base` (`base..HEAD`).
/// This is the number of commits unique to the current branch since it
/// diverged from `base`.
pub fn commits_ahead_of(base: &str) -> Result<usize> {
    commits_ahead_of_in(None, base)
}

fn assert_repository_in(cwd: Option<&Path>) -> Result<()> {
    output_in(cwd, ["rev-parse", "--show-toplevel"]).map(|_| ())
}

fn detect_trunk_in(cwd: Option<&Path>) -> Option<String> {
    // Remote default branch, e.g. "origin/main".
    if let Ok(default) = output_in(cwd, ["rev-parse", "--abbrev-ref", "origin/HEAD"]) {
        let default = default.trim();
        if !default.is_empty() && default != "origin/HEAD" {
            return Some(default.to_string());
        }
    }
    for candidate in ["main", "master"] {
        if output_in(cwd, ["rev-parse", "--verify", "--quiet", candidate]).is_ok() {
            return Some(candidate.to_string());
        }
    }
    None
}

fn commits_ahead_of_in(cwd: Option<&Path>, base: &str) -> Result<usize> {
    let count = output_in(cwd, ["rev-list", "--count", &format!("{base}..HEAD")])?;
    Ok(count.trim().parse().unwrap_or(0))
}

fn status_short_in(cwd: Option<&Path>) -> Result<String> {
    output_in(cwd, ["status", "--short"])
}

fn diff_for_in(cwd: Option<&Path>, source: &DiffSource) -> Result<String> {
    output_in(cwd, diff_args(source, DiffOutput::Patch)?)
}

fn diff_stat_for_in(cwd: Option<&Path>, source: &DiffSource) -> Result<String> {
    output_in(cwd, diff_args(source, DiffOutput::Stat)?)
}

fn changed_files_for_in(cwd: Option<&Path>, source: &DiffSource) -> Result<Vec<String>> {
    Ok(output_in(cwd, diff_args(source, DiffOutput::NameOnly)?)?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

fn changed_line_count_for_in(cwd: Option<&Path>, source: &DiffSource) -> Result<usize> {
    let numstat = output_in(cwd, diff_args(source, DiffOutput::NumStat)?)?;
    Ok(numstat
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut parts = line.split_whitespace();
            let added = parts
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            let deleted = parts
                .next()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            added + deleted
        })
        .sum())
}

enum DiffOutput {
    Patch,
    Stat,
    NameOnly,
    NumStat,
}

fn diff_args(source: &DiffSource, output: DiffOutput) -> Result<Vec<String>> {
    source.validate()?;

    let mut args = vec!["diff".to_string()];
    match output {
        DiffOutput::Patch => args.push("--no-ext-diff".to_string()),
        DiffOutput::Stat => args.push("--stat".to_string()),
        DiffOutput::NameOnly => args.push("--name-only".to_string()),
        DiffOutput::NumStat => args.push("--numstat".to_string()),
    }

    match source {
        DiffSource::WorkingTree => {}
        DiffSource::Cached => args.push("--cached".to_string()),
        DiffSource::Base(base) => args.push(format!("{base}...HEAD")),
        DiffSource::Range(range) => args.push(range.to_string()),
    }

    Ok(args)
}

fn validate_revision_arg(kind: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(anyhow!("--{kind} must not be empty"));
    }
    if value.starts_with('-') {
        return Err(anyhow!("--{kind} must be a revision, not an option"));
    }
    if value.chars().any(char::is_whitespace) {
        return Err(anyhow!("--{kind} must not contain whitespace"));
    }
    if value.contains('\0') {
        return Err(anyhow!("--{kind} must not contain NUL bytes"));
    }
    Ok(())
}

pub fn output<I, S>(args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    output_in(None, args)
}

fn output_in<I, S>(cwd: Option<&Path>, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect();
    let mut command = Command::new("git");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command
        .args(&args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(anyhow!(
            "{}",
            if stderr.is_empty() {
                format!("git {} failed", args.join(" "))
            } else {
                stderr
            }
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(name: &str) -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos();
            let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rebotica-git-{name}-{}-{suffix}-{counter}",
                std::process::id(),
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

    fn run_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repo)
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

    fn commit_all(repo: &Path, message: &str) {
        run_git(repo, &["add", "."]);
        run_git(repo, &["commit", "-m", message]);
    }

    fn fixture_repo() -> TempDir {
        let repo = TempDir::new("repo");
        run_git(repo.path(), &["init"]);
        run_git(repo.path(), &["config", "user.name", "Rebotica Test"]);
        run_git(
            repo.path(),
            &["config", "user.email", "rebotica-test@example.invalid"],
        );
        run_git(repo.path(), &["config", "commit.gpgsign", "false"]);
        fs::create_dir_all(repo.path().join("src")).unwrap();
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 {\n    1\n}\n",
        )
        .unwrap();
        run_git(repo.path(), &["add", "."]);
        run_git(repo.path(), &["commit", "-m", "initial"]);
        run_git(repo.path(), &["branch", "-M", "main"]);
        repo
    }

    #[test]
    fn repository_checks_succeed_inside_git_repo_and_fail_outside() {
        let repo = fixture_repo();
        let outside = TempDir::new("outside");

        assert_repository_in(Some(repo.path())).unwrap();
        let error = assert_repository_in(Some(outside.path())).unwrap_err();
        assert!(error.to_string().contains("not a git repository"));
    }

    #[test]
    fn diff_helpers_report_changed_files_and_line_counts() {
        let repo = fixture_repo();
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 {\n    2\n}\n\npub fn extra() -> u8 {\n    3\n}\n",
        )
        .unwrap();

        assert_eq!(
            changed_files_for_in(Some(repo.path()), &DiffSource::WorkingTree).unwrap(),
            vec!["src/lib.rs"]
        );
        assert_eq!(
            changed_line_count_for_in(Some(repo.path()), &DiffSource::WorkingTree).unwrap(),
            6
        );
        assert!(status_short_in(Some(repo.path()))
            .unwrap()
            .contains("M src/lib.rs"));
        assert!(diff_for_in(Some(repo.path()), &DiffSource::WorkingTree)
            .unwrap()
            .contains("+pub fn extra()"));
        assert!(
            diff_stat_for_in(Some(repo.path()), &DiffSource::WorkingTree)
                .unwrap()
                .contains("src/lib.rs")
        );
    }

    #[test]
    fn cached_diff_source_reports_staged_changes() {
        let repo = fixture_repo();
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 {\n    2\n}\n",
        )
        .unwrap();
        run_git(repo.path(), &["add", "src/lib.rs"]);

        let source = DiffSource::Cached;

        assert_eq!(
            changed_files_for_in(Some(repo.path()), &source).unwrap(),
            vec!["src/lib.rs"]
        );
        assert!(diff_for_in(Some(repo.path()), &source)
            .unwrap()
            .contains("+    2"));
        assert!(diff_for_in(Some(repo.path()), &DiffSource::WorkingTree)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn range_diff_source_reports_committed_changes() {
        let repo = fixture_repo();
        let base = output_in(Some(repo.path()), ["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 {\n    2\n}\n",
        )
        .unwrap();
        commit_all(repo.path(), "change answer");

        let source = DiffSource::Range(format!("{base}..HEAD"));

        assert_eq!(
            changed_files_for_in(Some(repo.path()), &source).unwrap(),
            vec!["src/lib.rs"]
        );
        assert!(diff_for_in(Some(repo.path()), &source)
            .unwrap()
            .contains("+    2"));
    }

    #[test]
    fn base_diff_source_uses_merge_base_to_head() {
        let repo = fixture_repo();
        run_git(repo.path(), &["checkout", "-b", "feature"]);
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 {\n    2\n}\n",
        )
        .unwrap();
        commit_all(repo.path(), "feature change");

        run_git(repo.path(), &["checkout", "main"]);
        fs::write(
            repo.path().join("src/lib.rs"),
            "pub fn answer() -> u8 {\n    9\n}\n",
        )
        .unwrap();
        commit_all(repo.path(), "main change");

        run_git(repo.path(), &["checkout", "feature"]);
        let diff = diff_for_in(Some(repo.path()), &DiffSource::Base("main".to_string())).unwrap();

        assert!(diff.contains("+    2"));
        assert!(!diff.contains("-    9"));
    }

    #[test]
    fn commits_ahead_of_counts_branch_commits_since_divergence() {
        let repo = fixture_repo();
        run_git(repo.path(), &["checkout", "-b", "feature"]);
        // Two commits on the feature branch.
        fs::write(repo.path().join("src/lib.rs"), "pub fn answer() -> u8 {\n    2\n}\n").unwrap();
        commit_all(repo.path(), "feature one");
        fs::write(repo.path().join("src/lib.rs"), "pub fn answer() -> u8 {\n    3\n}\n").unwrap();
        commit_all(repo.path(), "feature two");

        assert_eq!(commits_ahead_of_in(Some(repo.path()), "main").unwrap(), 2);

        // On main itself there is nothing ahead.
        run_git(repo.path(), &["checkout", "main"]);
        assert_eq!(commits_ahead_of_in(Some(repo.path()), "main").unwrap(), 0);
    }

    #[test]
    fn commits_ahead_counts_branch_commits_even_when_base_advanced() {
        // The #68/#70 scenario: branch is behind main (main moved on) but
        // still has its own commits. The count must reflect the branch's
        // own work, not be confused by main's newer commits.
        let repo = fixture_repo();
        run_git(repo.path(), &["checkout", "-b", "feature"]);
        fs::write(repo.path().join("src/lib.rs"), "pub fn answer() -> u8 {\n    2\n}\n").unwrap();
        commit_all(repo.path(), "feature change");

        run_git(repo.path(), &["checkout", "main"]);
        fs::write(repo.path().join("src/other.rs"), "pub fn x() {}\n").unwrap();
        commit_all(repo.path(), "main advances");

        run_git(repo.path(), &["checkout", "feature"]);
        assert_eq!(commits_ahead_of_in(Some(repo.path()), "main").unwrap(), 1);
    }

    #[test]
    fn detect_trunk_finds_main() {
        let repo = fixture_repo();
        assert_eq!(detect_trunk_in(Some(repo.path())).as_deref(), Some("main"));
    }

    #[test]
    fn revision_validation_rejects_options_and_ranges_in_base() {
        let option_error = DiffSource::Range("--stat".to_string())
            .validate()
            .unwrap_err();
        assert!(option_error.to_string().contains("not an option"));

        let base_error = DiffSource::Base("main..HEAD".to_string())
            .validate()
            .unwrap_err();
        assert!(base_error.to_string().contains("use --range"));
    }

    #[test]
    fn output_reports_git_stderr_on_failure() {
        let outside = TempDir::new("not-a-repo");

        let error = output_in(Some(outside.path()), ["status", "--short"]).unwrap_err();

        assert!(error.to_string().contains("not a git repository"));
    }
}
