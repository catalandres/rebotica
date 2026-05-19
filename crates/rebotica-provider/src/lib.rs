use anyhow::{anyhow, Context, Result};
use rebotica_core::{LoadedConfig, ProviderConfig};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone)]
pub struct ProviderOverrides {
    pub provider: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderSettings {
    pub name: String,
    pub base_url: String,
    pub headers: BTreeMap<String, String>,
}

impl ProviderSettings {
    pub fn resolve(loaded: &LoadedConfig, overrides: ProviderOverrides) -> Result<Self> {
        let selected = overrides
            .provider
            .or_else(|| std::env::var("REBOTICA_PROVIDER").ok())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| loaded.config.providers.default.clone());

        let default_url = "http://127.0.0.1:1234/v1";
        let (base_url, provider_config) = if let Some(base_url) = overrides
            .base_url
            .or_else(|| std::env::var("REBOTICA_BASE_URL").ok())
            .filter(|value| !value.is_empty())
        {
            (base_url, loaded.config.providers.entries.get(&selected))
        } else if selected.starts_with("http://") || selected.starts_with("https://") {
            (selected.clone(), None)
        } else if selected == "lmstudio" {
            let config = loaded.config.providers.entries.get(&selected);
            let url = config
                .map(|config| config.base_url.as_str())
                .filter(|url| !url.is_empty())
                .unwrap_or(default_url)
                .to_string();
            (url, config)
        } else {
            let config = loaded
                .config
                .providers
                .entries
                .get(&selected)
                .ok_or_else(|| {
                    anyhow!(
                        "unknown provider '{}'. Add providers.{}.base_url or pass --base-url.",
                        selected,
                        selected
                    )
                })?;
            if config.base_url.is_empty() {
                return Err(anyhow!("provider '{}' is missing base_url", selected));
            }
            (config.base_url.clone(), Some(config))
        };

        let headers = provider_config
            .map(resolve_headers)
            .transpose()?
            .unwrap_or_default();

        Ok(Self {
            name: selected,
            base_url: base_url.trim_end_matches('/').to_string(),
            headers,
        })
    }
}

fn resolve_headers(config: &ProviderConfig) -> Result<BTreeMap<String, String>> {
    let mut headers = config.headers.clone();
    if !config.api_key_env.is_empty() {
        let key = std::env::var(&config.api_key_env).with_context(|| {
            format!(
                "provider requires environment variable {}",
                config.api_key_env
            )
        })?;
        let header = if config.api_key_header.is_empty() {
            "Authorization"
        } else {
            &config.api_key_header
        };
        let prefix = config.api_key_prefix.as_deref().unwrap_or("Bearer ");
        headers.insert(header.to_string(), format!("{prefix}{key}"));
    }
    Ok(headers)
}

#[derive(Debug, Clone)]
pub struct OpenAICompatibleProvider {
    base_url: String,
    client: reqwest::Client,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    Unavailable {
        endpoint: &'static str,
        message: String,
    },
    HttpStatus {
        endpoint: &'static str,
        status: u16,
        body: String,
    },
    InvalidResponse {
        endpoint: &'static str,
        message: String,
    },
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { endpoint, message } => {
                write!(
                    formatter,
                    "could not reach provider {endpoint} endpoint: {message}"
                )
            }
            Self::HttpStatus {
                endpoint,
                status,
                body,
            } => {
                if body.is_empty() {
                    write!(
                        formatter,
                        "provider {endpoint} endpoint returned HTTP {status}"
                    )
                } else {
                    write!(
                        formatter,
                        "provider {endpoint} endpoint returned HTTP {status}: {body}"
                    )
                }
            }
            Self::InvalidResponse { endpoint, message } => {
                write!(
                    formatter,
                    "provider returned invalid {endpoint} response: {message}"
                )
            }
        }
    }
}

impl std::error::Error for ProviderError {}

impl OpenAICompatibleProvider {
    pub fn new(settings: &ProviderSettings) -> Result<Self> {
        let mut headers = HeaderMap::new();
        for (name, value) in &settings.headers {
            headers.insert(
                HeaderName::from_bytes(name.as_bytes())
                    .with_context(|| format!("invalid header name '{}'", name))?,
                HeaderValue::from_str(value)
                    .with_context(|| format!("invalid header value for '{}'", name))?,
            );
        }
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .context("failed to build provider client")?;
        Ok(Self {
            base_url: settings.base_url.clone(),
            client,
        })
    }

    pub async fn models(&self) -> std::result::Result<Vec<String>, ProviderError> {
        let endpoint = "models";
        let response = self
            .client
            .get(format!("{}/models", self.base_url))
            .send()
            .await
            .map_err(|error| provider_unavailable(endpoint, error))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.map(truncate_body).unwrap_or_default();
            return Err(ProviderError::HttpStatus {
                endpoint,
                status: status.as_u16(),
                body,
            });
        }
        let response: ModelsResponse =
            response
                .json()
                .await
                .map_err(|error| ProviderError::InvalidResponse {
                    endpoint,
                    message: error.to_string(),
                })?;
        Ok(response.data.into_iter().map(|model| model.id).collect())
    }

    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        temperature: f64,
    ) -> std::result::Result<ChatCompletion, ProviderError> {
        let endpoint = "chat";
        let request = ChatRequest {
            model,
            messages,
            temperature,
        };
        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&request)
            .send()
            .await
            .map_err(|error| provider_unavailable(endpoint, error))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.map(truncate_body).unwrap_or_default();
            return Err(ProviderError::HttpStatus {
                endpoint,
                status: status.as_u16(),
                body,
            });
        }
        let response: ChatResponse =
            response
                .json()
                .await
                .map_err(|error| ProviderError::InvalidResponse {
                    endpoint,
                    message: error.to_string(),
                })?;
        let usage = response.usage.and_then(|u| {
            match (u.prompt_tokens, u.completion_tokens) {
                (Some(prompt_tokens), Some(completion_tokens)) => Some(ChatUsage {
                    prompt_tokens,
                    completion_tokens,
                }),
                _ => None,
            }
        });
        let content = response
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .ok_or_else(|| ProviderError::InvalidResponse {
                endpoint,
                message: "missing choices[0].message.content".to_string(),
            })?;
        Ok(ChatCompletion { content, usage })
    }
}

/// Per-call token accounting reported by the provider. Optional because
/// not every OpenAI-compatible endpoint returns `usage`; consumers that
/// depend on it must tolerate `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// Result of a successful `chat()` call. `content` is the message body;
/// `usage` carries the provider's token accounting when available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatCompletion {
    pub content: String,
    pub usage: Option<ChatUsage>,
}

fn provider_unavailable(endpoint: &'static str, error: reqwest::Error) -> ProviderError {
    ProviderError::Unavailable {
        endpoint,
        message: error.to_string(),
    }
}

fn truncate_body(body: String) -> String {
    const MAX_BODY_CHARS: usize = 2048;
    let trimmed = body.trim().to_string();
    if trimmed.chars().count() <= MAX_BODY_CHARS {
        return trimmed;
    }
    let mut truncated = trimmed.chars().take(MAX_BODY_CHARS).collect::<String>();
    truncated.push_str("...");
    truncated
}

#[derive(Debug, Clone, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn new(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage>,
    temperature: f64,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<Model>,
}

#[derive(Debug, Deserialize)]
struct Model {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    #[serde(default)]
    usage: Option<UsageDto>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct UsageDto {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rebotica_core::{LoadedConfig, ProjectConfig, ProviderConfig};
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct EnvGuard {
        previous: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvGuard {
        fn clear(keys: &[&'static str]) -> Self {
            let previous = keys
                .iter()
                .map(|key| (*key, std::env::var_os(key)))
                .collect::<Vec<_>>();
            for key in keys {
                std::env::remove_var(key);
            }
            Self { previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in &self.previous {
                if let Some(value) = value {
                    std::env::set_var(key, value);
                } else {
                    std::env::remove_var(key);
                }
            }
        }
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("env lock should not be poisoned")
    }

    fn loaded(config: ProjectConfig) -> LoadedConfig {
        LoadedConfig {
            path: None,
            raw: String::new(),
            config,
        }
    }

    #[test]
    fn resolves_implicit_lmstudio_defaults() {
        let _lock = env_lock();
        let _env = EnvGuard::clear(&["REBOTICA_PROVIDER", "REBOTICA_BASE_URL"]);
        let settings = ProviderSettings::resolve(
            &loaded(ProjectConfig::default()),
            ProviderOverrides {
                provider: None,
                base_url: None,
            },
        )
        .unwrap();

        assert_eq!(settings.name, "lmstudio");
        assert_eq!(settings.base_url, "http://127.0.0.1:1234/v1");
        assert!(settings.headers.is_empty());
    }

    #[test]
    fn resolves_configured_provider_and_trims_base_url() {
        let _lock = env_lock();
        let _env = EnvGuard::clear(&["REBOTICA_PROVIDER", "REBOTICA_BASE_URL"]);
        let mut config = ProjectConfig::default();
        config.providers.default = "openai".to_string();
        config.providers.entries.insert(
            "openai".to_string(),
            ProviderConfig {
                kind: "openai-compatible".to_string(),
                base_url: "https://api.openai.com/v1/".to_string(),
                ..ProviderConfig::default()
            },
        );

        let settings = ProviderSettings::resolve(
            &loaded(config),
            ProviderOverrides {
                provider: None,
                base_url: None,
            },
        )
        .unwrap();

        assert_eq!(settings.name, "openai");
        assert_eq!(settings.base_url, "https://api.openai.com/v1");
    }

    #[test]
    fn command_line_overrides_take_precedence_over_provider_env() {
        let _lock = env_lock();
        let _env = EnvGuard::clear(&["REBOTICA_PROVIDER", "REBOTICA_BASE_URL"]);
        std::env::set_var("REBOTICA_PROVIDER", "env-provider");
        std::env::set_var("REBOTICA_BASE_URL", "http://env.example/v1");

        let settings = ProviderSettings::resolve(
            &loaded(ProjectConfig::default()),
            ProviderOverrides {
                provider: Some("cli-provider".to_string()),
                base_url: Some("http://cli.example/v1".to_string()),
            },
        )
        .unwrap();

        assert_eq!(settings.name, "cli-provider");
        assert_eq!(settings.base_url, "http://cli.example/v1");
    }

    #[test]
    fn resolves_api_key_headers_from_configured_env_var() {
        let _lock = env_lock();
        let _env = EnvGuard::clear(&[
            "REBOTICA_PROVIDER",
            "REBOTICA_BASE_URL",
            "REBOTICA_TEST_PROVIDER_KEY",
        ]);
        std::env::set_var("REBOTICA_TEST_PROVIDER_KEY", "secret-token");
        let mut config = ProjectConfig::default();
        config.providers.default = "remote".to_string();
        config.providers.entries.insert(
            "remote".to_string(),
            ProviderConfig {
                kind: "openai-compatible".to_string(),
                base_url: "https://remote.example/v1".to_string(),
                api_key_env: "REBOTICA_TEST_PROVIDER_KEY".to_string(),
                ..ProviderConfig::default()
            },
        );

        let settings = ProviderSettings::resolve(
            &loaded(config),
            ProviderOverrides {
                provider: None,
                base_url: None,
            },
        )
        .unwrap();

        assert_eq!(
            settings.headers.get("Authorization").map(String::as_str),
            Some("Bearer secret-token")
        );
    }

    #[test]
    fn unknown_provider_reports_actionable_error() {
        let _lock = env_lock();
        let _env = EnvGuard::clear(&["REBOTICA_PROVIDER", "REBOTICA_BASE_URL"]);

        let error = ProviderSettings::resolve(
            &loaded(ProjectConfig::default()),
            ProviderOverrides {
                provider: Some("missing".to_string()),
                base_url: None,
            },
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("Add providers.missing.base_url or pass --base-url"));
    }
}
