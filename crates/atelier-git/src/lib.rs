use anyhow::{anyhow, Context, Result};
use std::process::Command;

pub fn assert_repository() -> Result<()> {
    output(["rev-parse", "--show-toplevel"]).map(|_| ())
}

pub fn status_short() -> Result<String> {
    output(["status", "--short"])
}

pub fn diff() -> Result<String> {
    output(["diff", "--no-ext-diff"])
}

pub fn diff_stat() -> Result<String> {
    output(["diff", "--stat"])
}

pub fn changed_files() -> Result<Vec<String>> {
    Ok(output(["diff", "--name-only"])?
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect())
}

pub fn changed_line_count() -> Result<usize> {
    let numstat = output(["diff", "--numstat"])?;
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
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| arg.as_ref().to_string())
        .collect();
    let output = Command::new("git")
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
