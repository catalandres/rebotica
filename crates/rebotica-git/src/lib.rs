use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Command;

pub fn assert_repository() -> Result<()> {
    assert_repository_in(None)
}

pub fn status_short() -> Result<String> {
    status_short_in(None)
}

pub fn diff() -> Result<String> {
    diff_in(None)
}

pub fn diff_stat() -> Result<String> {
    diff_stat_in(None)
}

pub fn changed_files() -> Result<Vec<String>> {
    changed_files_in(None)
}

pub fn changed_line_count() -> Result<usize> {
    changed_line_count_in(None)
}

fn assert_repository_in(cwd: Option<&Path>) -> Result<()> {
    output_in(cwd, ["rev-parse", "--show-toplevel"]).map(|_| ())
}

fn status_short_in(cwd: Option<&Path>) -> Result<String> {
    output_in(cwd, ["status", "--short"])
}

fn diff_in(cwd: Option<&Path>) -> Result<String> {
    output_in(cwd, ["diff", "--no-ext-diff"])
}

fn diff_stat_in(cwd: Option<&Path>) -> Result<String> {
    output_in(cwd, ["diff", "--stat"])
}

fn changed_files_in(cwd: Option<&Path>) -> Result<Vec<String>> {
    Ok(output_in(cwd, ["diff", "--name-only"])?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

fn changed_line_count_in(cwd: Option<&Path>) -> Result<usize> {
    let numstat = output_in(cwd, ["diff", "--numstat"])?;
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
                "rebotica-git-{name}-{}-{suffix}",
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
            changed_files_in(Some(repo.path())).unwrap(),
            vec!["src/lib.rs"]
        );
        assert_eq!(changed_line_count_in(Some(repo.path())).unwrap(), 6);
        assert!(status_short_in(Some(repo.path()))
            .unwrap()
            .contains("M src/lib.rs"));
        assert!(diff_in(Some(repo.path()))
            .unwrap()
            .contains("+pub fn extra()"));
        assert!(diff_stat_in(Some(repo.path()))
            .unwrap()
            .contains("src/lib.rs"));
    }

    #[test]
    fn output_reports_git_stderr_on_failure() {
        let outside = TempDir::new("not-a-repo");

        let error = output_in(Some(outside.path()), ["status", "--short"]).unwrap_err();

        assert!(error.to_string().contains("not a git repository"));
    }
}
