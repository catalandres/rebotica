use anyhow::{anyhow, Result};

pub fn ensure_allowed(files: &[String], forbidden_patterns: &[String]) -> Result<()> {
    for file in files {
        let normalized = normalize(file);
        for pattern in forbidden_patterns {
            let clean = normalize(pattern).trim_matches('/').to_string();
            if clean.is_empty() {
                continue;
            }
            if normalized == clean
                || normalized.starts_with(&format!("{clean}/"))
                || normalized.contains(&clean)
            {
                return Err(anyhow!(
                    "file '{}' is forbidden by pattern '{}'",
                    file,
                    pattern
                ));
            }
        }
    }
    Ok(())
}

fn normalize(path: &str) -> String {
    path.strip_prefix("./").unwrap_or(path).to_string()
}
