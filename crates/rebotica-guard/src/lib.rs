use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardError {
    rejected_path: String,
    forbidden_pattern: String,
}

impl GuardError {
    pub fn new(rejected_path: impl Into<String>, forbidden_pattern: impl Into<String>) -> Self {
        Self {
            rejected_path: rejected_path.into(),
            forbidden_pattern: forbidden_pattern.into(),
        }
    }

    pub fn rejected_path(&self) -> &str {
        &self.rejected_path
    }

    pub fn forbidden_pattern(&self) -> &str {
        &self.forbidden_pattern
    }
}

impl fmt::Display for GuardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "file '{}' is forbidden by pattern '{}'",
            self.rejected_path, self.forbidden_pattern
        )
    }
}

impl std::error::Error for GuardError {}

pub fn ensure_allowed(files: &[String], forbidden_patterns: &[String]) -> Result<(), GuardError> {
    for file in files {
        let normalized = normalize(file);
        for pattern in forbidden_patterns {
            let directory_pattern = pattern.ends_with('/') || pattern.ends_with('\\');
            let clean = normalize(pattern).trim_matches('/').to_string();
            if clean.is_empty() {
                continue;
            }
            if is_forbidden(&normalized, &clean, directory_pattern) {
                return Err(GuardError::new(file, pattern));
            }
        }
    }
    Ok(())
}

fn is_forbidden(path: &str, pattern: &str, directory_pattern: bool) -> bool {
    if path == pattern || path.starts_with(&format!("{pattern}/")) {
        return true;
    }

    if directory_pattern {
        return path.contains(&format!("/{pattern}/")) || path.ends_with(&format!("/{pattern}"));
    }

    if pattern.contains('/') {
        return false;
    }

    path.split('/').any(|component| {
        component == pattern
            || component
                .strip_prefix(pattern)
                .map(|suffix| suffix.starts_with('.') || suffix.starts_with('-'))
                .unwrap_or(false)
    })
}

fn normalize(path: &str) -> String {
    path.strip_prefix("./")
        .unwrap_or(path)
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_exact_forbidden_file() {
        let files = vec![".env".to_string()];
        let forbidden = vec![".env".to_string()];

        let error = ensure_allowed(&files, &forbidden).unwrap_err();

        assert!(error.to_string().contains("forbidden by pattern '.env'"));
    }

    #[test]
    fn rejects_files_under_forbidden_directories() {
        let files = vec!["src/secrets/key.txt".to_string()];
        let forbidden = vec!["secrets/".to_string()];

        ensure_allowed(&files, &forbidden).unwrap_err();
    }

    #[test]
    fn allows_similarly_named_paths_that_are_not_forbidden_directories() {
        let files = vec!["src/no-secrets.rs".to_string()];
        let forbidden = vec!["secrets/".to_string()];

        ensure_allowed(&files, &forbidden).unwrap();
    }

    #[test]
    fn allows_same_prefix_when_pattern_is_not_a_complete_path_component() {
        let files = vec!["src/secretary.rs".to_string()];
        let forbidden = vec!["secret".to_string()];

        ensure_allowed(&files, &forbidden).unwrap();
    }

    #[test]
    fn normalizes_relative_and_windows_style_paths() {
        let files = vec!["./secrets\\key.txt".to_string()];
        let forbidden = vec!["secrets/".to_string()];

        ensure_allowed(&files, &forbidden).unwrap_err();
    }

    #[test]
    fn ignores_empty_forbidden_patterns() {
        let files = vec!["src/lib.rs".to_string()];
        let forbidden = vec!["".to_string(), "/".to_string()];

        ensure_allowed(&files, &forbidden).unwrap();
    }
}
