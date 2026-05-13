use anyhow::{anyhow, Context, Result};
use atelier_core::{LoadedConfig, ProviderConfig};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
            .or_else(|| std::env::var("ATELIER_PROVIDER").ok())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| loaded.config.providers.default.clone());

        let default_url = "http://127.0.0.1:1234/v1";
        let (base_url, provider_config) = if let Some(base_url) = overrides
            .base_url
            .or_else(|| std::env::var("ATELIER_BASE_URL").ok())
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

    pub async fn models(&self) -> Result<Vec<String>> {
        let response: ModelsResponse = self
            .client
            .get(format!("{}/models", self.base_url))
            .send()
            .await
            .context("could not reach provider models endpoint")?
            .error_for_status()
            .context("provider models endpoint returned an error")?
            .json()
            .await
            .context("provider returned invalid models JSON")?;
        Ok(response.data.into_iter().map(|model| model.id).collect())
    }

    pub async fn chat(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
        temperature: f64,
    ) -> Result<String> {
        let request = ChatRequest {
            model,
            messages,
            temperature,
        };
        let response: ChatResponse = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&request)
            .send()
            .await
            .context("could not reach provider chat endpoint")?
            .error_for_status()
            .context("provider chat endpoint returned an error")?
            .json()
            .await
            .context("provider returned invalid chat JSON")?;
        response
            .choices
            .into_iter()
            .next()
            .map(|choice| choice.message.content)
            .ok_or_else(|| anyhow!("provider response did not include choices[0].message.content"))
    }
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
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Debug, Deserialize)]
struct ChoiceMessage {
    content: String,
}
