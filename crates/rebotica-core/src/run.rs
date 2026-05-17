use crate::output::ErrorCode;
use anyhow::{anyhow, Context, Result};
use jsonschema::{Retrieve, Uri};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};

pub const COMMON_SCHEMA_ID: &str = "https://rebotica/runs-common.schema.json";
pub const BUILTIN_INPUT_ADAPTERS: &[&str] = &["diff", "files", "task_envelope", "skills", "guard"];

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub kind: String,
    pub display_name: String,
    pub description: String,
    pub schema_version: u64,
    pub inputs: Vec<String>,
    #[serde(default)]
    pub prompt_file: Option<String>,
    #[serde(default)]
    pub schema_file: Option<String>,
    #[serde(default)]
    pub exit_codes: Vec<String>,
}

impl Manifest {
    pub fn prompt_file(&self) -> &str {
        self.prompt_file.as_deref().unwrap_or("prompt.md")
    }

    pub fn schema_file(&self) -> &str {
        self.schema_file.as_deref().unwrap_or("schema.json")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLayer {
    Project,
    User,
    Builtin,
}

impl RunLayer {
    pub fn label(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Builtin => "built-in",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RegistryRoots {
    pub project: PathBuf,
    pub user: PathBuf,
    pub builtin: PathBuf,
    pub common_schema: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ResolvedPlugin {
    pub mode: String,
    pub layer: RunLayer,
    pub directory: PathBuf,
    pub manifest: Manifest,
    pub prompt_path: PathBuf,
    pub schema_path: PathBuf,
    pub prompt: String,
    pub schema: Value,
    pub common_schema: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunModeListing {
    pub mode: String,
    pub kind: String,
    pub display_name: String,
    pub description: String,
    pub schema_version: u64,
    pub inputs: Vec<String>,
    pub exit_codes: Vec<String>,
    pub layer: RunLayer,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BrokenPluginLayer {
    pub mode: String,
    pub layer: RunLayer,
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct Registry {
    plugins: BTreeMap<String, ResolvedPlugin>,
    broken_by_mode: BTreeMap<String, Vec<BrokenPluginLayer>>,
}

impl Registry {
    pub fn load(roots: RegistryRoots) -> Result<Self> {
        let common_schema_text = std::fs::read_to_string(&roots.common_schema)
            .with_context(|| format!("failed to read {}", roots.common_schema.display()))?;
        let common_schema: Value = serde_json::from_str(&common_schema_text)
            .with_context(|| format!("failed to parse {}", roots.common_schema.display()))?;
        validate_common_schema(&common_schema)?;

        let layer_roots = [
            (RunLayer::Project, roots.project),
            (RunLayer::User, roots.user),
            (RunLayer::Builtin, roots.builtin),
        ];

        let mut discovered_modes = BTreeSet::new();
        let mut candidates: BTreeMap<(RunLayer, String), LayerCandidate> = BTreeMap::new();

        for (layer, root) in &layer_roots {
            if !root.exists() {
                continue;
            }
            for entry in std::fs::read_dir(root)
                .with_context(|| format!("failed to read {}", root.display()))?
            {
                let entry = entry?;
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let Some(raw_name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if raw_name == "_common" {
                    continue;
                }
                let mode = raw_name.to_ascii_lowercase();
                discovered_modes.insert(mode.clone());
                let candidate = load_layer_candidate(*layer, &path, raw_name, &common_schema);
                candidates.insert((*layer, mode), candidate);
            }
        }

        let mut plugins = BTreeMap::new();
        let mut broken_by_mode: BTreeMap<String, Vec<BrokenPluginLayer>> = BTreeMap::new();
        for mode in discovered_modes {
            for (layer, _) in &layer_roots {
                let Some(candidate) = candidates.remove(&(*layer, mode.clone())) else {
                    continue;
                };
                match candidate {
                    LayerCandidate::Complete(plugin) => {
                        plugins.entry(mode.clone()).or_insert(plugin);
                    }
                    LayerCandidate::Broken(broken) => {
                        broken_by_mode.entry(mode.clone()).or_default().push(broken);
                    }
                }
            }
        }

        Ok(Self {
            plugins,
            broken_by_mode,
        })
    }

    pub fn resolve(&self, mode: &str) -> std::result::Result<&ResolvedPlugin, RunError> {
        if let Some(plugin) = self.plugins.get(mode) {
            return Ok(plugin);
        }
        let broken = self.broken_by_mode.get(mode).cloned().unwrap_or_default();
        if broken.is_empty() {
            Err(RunError::UnknownMode {
                mode: mode.to_string(),
                available: self.available_mode_names(),
            })
        } else {
            Err(RunError::AllLayersBroken {
                mode: mode.to_string(),
                broken,
            })
        }
    }

    pub fn available_modes(&self) -> Vec<RunModeListing> {
        self.plugins
            .values()
            .map(|plugin| RunModeListing {
                mode: plugin.mode.clone(),
                kind: plugin.manifest.kind.clone(),
                display_name: plugin.manifest.display_name.clone(),
                description: plugin.manifest.description.clone(),
                schema_version: plugin.manifest.schema_version,
                inputs: plugin.manifest.inputs.clone(),
                exit_codes: plugin.manifest.exit_codes.clone(),
                layer: plugin.layer,
                path: plugin.directory.display().to_string(),
            })
            .collect()
    }

    pub fn available_mode_names(&self) -> Vec<String> {
        self.plugins.keys().cloned().collect()
    }

    pub fn broken_layers(&self) -> Vec<BrokenPluginLayer> {
        self.broken_by_mode
            .values()
            .flat_map(|layers| layers.iter().cloned())
            .collect()
    }

    pub fn broken_layers_for_mode(&self, mode: &str) -> Vec<BrokenPluginLayer> {
        self.broken_by_mode.get(mode).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub enum RunError {
    UnknownMode {
        mode: String,
        available: Vec<String>,
    },
    AllLayersBroken {
        mode: String,
        broken: Vec<BrokenPluginLayer>,
    },
    InvalidPlugin {
        mode: String,
        reason: String,
    },
}

impl fmt::Display for RunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMode { mode, available } => {
                if available.is_empty() {
                    write!(formatter, "unknown run mode: {mode}")
                } else {
                    write!(
                        formatter,
                        "unknown run mode: {mode}. available modes: {}",
                        available.join(", ")
                    )
                }
            }
            Self::AllLayersBroken { mode, .. } => {
                write!(formatter, "all plugin layers for {mode} are broken")
            }
            Self::InvalidPlugin { mode, reason } => {
                write!(formatter, "invalid plugin {mode}: {reason}")
            }
        }
    }
}

impl std::error::Error for RunError {}

enum LayerCandidate {
    Complete(ResolvedPlugin),
    Broken(BrokenPluginLayer),
}

fn broken(
    layer: RunLayer,
    directory: &Path,
    mode: impl Into<String>,
    reason: impl Into<String>,
) -> LayerCandidate {
    LayerCandidate::Broken(BrokenPluginLayer {
        mode: mode.into(),
        layer,
        path: directory.display().to_string(),
        reason: reason.into(),
    })
}

fn load_layer_candidate(
    layer: RunLayer,
    directory: &Path,
    raw_mode_name: &str,
    common_schema: &Value,
) -> LayerCandidate {
    let mode = raw_mode_name.to_ascii_lowercase();
    if !is_valid_mode_name(raw_mode_name) {
        return broken(
            layer,
            directory,
            mode,
            format!("mode directory name must match ^[a-z0-9][a-z0-9_-]*$: {raw_mode_name}"),
        );
    }

    let manifest_path = directory.join("manifest.yml");
    if !manifest_path.is_file() {
        return broken(layer, directory, mode, "manifest.yml missing");
    }

    let manifest_text = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(error) => {
            return broken(
                layer,
                directory,
                mode,
                format!("manifest.yml unreadable: {error}"),
            )
        }
    };
    let manifest: Manifest = match serde_yaml::from_str(&manifest_text) {
        Ok(manifest) => manifest,
        Err(error) => {
            return broken(
                layer,
                directory,
                mode,
                format!("manifest.yml invalid: {error}"),
            )
        }
    };

    let expected_kind = format!("run.{mode}");
    if manifest.kind != expected_kind {
        return broken(
            layer,
            directory,
            mode,
            format!(
                "manifest kind must be {expected_kind}, got {}",
                manifest.kind
            ),
        );
    }
    if !manifest
        .kind
        .strip_prefix("run.")
        .map(is_valid_mode_name)
        .unwrap_or(false)
    {
        return broken(
            layer,
            directory,
            mode,
            format!("manifest kind has malformed run mode: {}", manifest.kind),
        );
    }
    for input in &manifest.inputs {
        if !BUILTIN_INPUT_ADAPTERS.contains(&input.as_str()) {
            return broken(
                layer,
                directory,
                mode,
                format!("unknown input adapter: {input}"),
            );
        }
    }
    for exit_code in &manifest.exit_codes {
        if !is_known_error_code(exit_code) {
            return broken(
                layer,
                directory,
                mode,
                format!("unknown exit code: {exit_code}"),
            );
        }
    }

    let prompt_path = match manifest_file_path(directory, manifest.prompt_file()) {
        Ok(path) => path,
        Err(error) => return broken(layer, directory, mode, error.to_string()),
    };
    if !prompt_path.is_file() {
        return broken(
            layer,
            directory,
            mode,
            format!("{} missing", manifest.prompt_file()),
        );
    }
    let schema_path = match manifest_file_path(directory, manifest.schema_file()) {
        Ok(path) => path,
        Err(error) => return broken(layer, directory, mode, error.to_string()),
    };
    if !schema_path.is_file() {
        return broken(
            layer,
            directory,
            mode,
            format!("{} missing", manifest.schema_file()),
        );
    }

    let prompt = match std::fs::read_to_string(&prompt_path) {
        Ok(prompt) => prompt,
        Err(error) => {
            return broken(
                layer,
                directory,
                mode,
                format!("prompt unreadable: {error}"),
            )
        }
    };
    let schema_text = match std::fs::read_to_string(&schema_path) {
        Ok(text) => text,
        Err(error) => {
            return broken(
                layer,
                directory,
                mode,
                format!("schema unreadable: {error}"),
            )
        }
    };
    let schema: Value = match serde_json::from_str(&schema_text) {
        Ok(schema) => schema,
        Err(error) => return broken(layer, directory, mode, format!("schema invalid: {error}")),
    };
    if !schema_refs_common(&schema) {
        return broken(
            layer,
            directory,
            mode,
            "schema.json must allOf + $ref https://rebotica/runs-common.schema.json",
        );
    }
    if let Err(error) = SchemaValidator::new(schema.clone(), common_schema.clone()) {
        return broken(
            layer,
            directory,
            mode,
            format!("schema validator failed: {error:#}"),
        );
    }

    LayerCandidate::Complete(ResolvedPlugin {
        mode,
        layer,
        directory: directory.to_path_buf(),
        manifest,
        prompt_path,
        schema_path,
        prompt,
        schema,
        common_schema: common_schema.clone(),
    })
}

fn validate_common_schema(common_schema: &Value) -> Result<()> {
    if common_schema.get("$id").and_then(Value::as_str) != Some(COMMON_SCHEMA_ID) {
        return Err(anyhow!(
            "runs-common.schema.json must declare $id {COMMON_SCHEMA_ID}"
        ));
    }
    let required = common_schema
        .get("required")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("runs-common.schema.json missing required array"))?;
    for field in ["assumptions", "confidence", "risks", "next_action"] {
        if !required.iter().any(|value| value.as_str() == Some(field)) {
            return Err(anyhow!(
                "runs-common.schema.json missing required field {field}"
            ));
        }
    }
    Ok(())
}

fn manifest_file_path(directory: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    if path.is_absolute() {
        return Err(anyhow!("manifest file paths must be relative: {relative}"));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(anyhow!(
            "manifest file paths must not contain parent components: {relative}"
        ));
    }
    Ok(directory.join(path))
}

fn is_known_error_code(value: &str) -> bool {
    ErrorCode::all()
        .iter()
        .any(|code| code.as_str() == value.trim())
}

pub fn is_valid_mode_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
}

fn schema_refs_common(schema: &Value) -> bool {
    schema
        .get("allOf")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .any(|item| item.get("$ref").and_then(Value::as_str) == Some(COMMON_SCHEMA_ID))
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionRule {
    Fence,
    Fallback,
}

impl ExtractionRule {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fence => "fence",
            Self::Fallback => "fallback",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtractedJson {
    pub extraction: ExtractionRule,
    pub source: String,
    pub value: Value,
    pub fallback_used: bool,
}

#[derive(Debug, Clone)]
pub struct JsonExtractionError {
    pub extraction: ExtractionRule,
    pub parse_error: String,
}

impl fmt::Display for JsonExtractionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.parse_error)
    }
}

impl std::error::Error for JsonExtractionError {}

pub fn extract_json_payload(
    response: &str,
) -> std::result::Result<ExtractedJson, JsonExtractionError> {
    if let Some(fence) = first_json_fence(response) {
        match serde_json::from_str::<Value>(fence) {
            Ok(value) => {
                return Ok(ExtractedJson {
                    extraction: ExtractionRule::Fence,
                    source: fence.to_string(),
                    value,
                    fallback_used: false,
                })
            }
            Err(error) => {
                if let Some((source, value)) = last_balanced_json_object(response) {
                    return Ok(ExtractedJson {
                        extraction: ExtractionRule::Fallback,
                        source,
                        value,
                        fallback_used: true,
                    });
                }
                return Err(JsonExtractionError {
                    extraction: ExtractionRule::Fence,
                    parse_error: error.to_string(),
                });
            }
        }
    }

    if let Some((source, value)) = last_balanced_json_object(response) {
        return Ok(ExtractedJson {
            extraction: ExtractionRule::Fallback,
            source,
            value,
            fallback_used: true,
        });
    }

    Err(JsonExtractionError {
        extraction: ExtractionRule::Fallback,
        parse_error: "no parseable JSON object found in model response".to_string(),
    })
}

fn first_json_fence(response: &str) -> Option<&str> {
    let mut search_from = 0;
    while let Some(relative_start) = response[search_from..].find("```") {
        let fence_start = search_from + relative_start;
        let language_start = fence_start + 3;
        let line_end = response[language_start..]
            .find('\n')
            .map(|offset| language_start + offset)?;
        let language = response[language_start..line_end].trim();
        if language.is_empty() || language.eq_ignore_ascii_case("json") {
            let content_start = line_end + 1;
            let closing = response[content_start..].find("```")?;
            let content_end = content_start + closing;
            return Some(response[content_start..content_end].trim());
        }
        search_from = language_start;
    }
    None
}

fn last_balanced_json_object(response: &str) -> Option<(String, Value)> {
    let mut opens = Vec::new();
    let mut closes = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in response.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => opens.push(index),
            '}' => closes.push(index + ch.len_utf8()),
            _ => {}
        }
    }

    for end in closes.into_iter().rev() {
        for start in opens.iter().rev().copied().filter(|start| *start < end) {
            let candidate = response[start..end].trim();
            if let Ok(value) = serde_json::from_str::<Value>(candidate) {
                return Some((candidate.to_string(), value));
            }
        }
    }
    None
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemaValidationError {
    pub instance_path: String,
    pub schema_path: String,
    pub keyword: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct SchemaValidator {
    schema: Value,
    common_schema: Value,
}

impl SchemaValidator {
    pub fn new(schema: Value, common_schema: Value) -> Result<Self> {
        let validator = Self {
            schema,
            common_schema,
        };
        validator.build()?;
        Ok(validator)
    }

    pub fn validate(&self, value: &Value) -> Result<Vec<SchemaValidationError>> {
        let compiled = self.build()?;
        Ok(compiled
            .iter_errors(value)
            .map(|error| SchemaValidationError {
                instance_path: error.instance_path().to_string(),
                schema_path: error.schema_path().to_string(),
                keyword: keyword_from_schema_path(&error.schema_path().to_string()),
                message: error.to_string(),
            })
            .collect())
    }

    fn build(&self) -> Result<jsonschema::Validator> {
        jsonschema::draft202012::options()
            .with_retriever(CommonSchemaRetriever {
                common_schema: self.common_schema.clone(),
            })
            .build(&self.schema)
            .context("failed to build JSON schema validator")
    }
}

#[derive(Clone)]
struct CommonSchemaRetriever {
    common_schema: Value,
}

impl Retrieve for CommonSchemaRetriever {
    fn retrieve(
        &self,
        uri: &Uri<String>,
    ) -> std::result::Result<Value, Box<dyn std::error::Error + Send + Sync>> {
        if uri.as_str() == COMMON_SCHEMA_ID {
            Ok(self.common_schema.clone())
        } else {
            Err(format!("schema not found: {uri}").into())
        }
    }
}

fn keyword_from_schema_path(schema_path: &str) -> String {
    schema_path
        .rsplit('/')
        .find(|part| !part.is_empty())
        .map(unescape_json_pointer)
        .unwrap_or_default()
}

fn unescape_json_pointer(value: &str) -> String {
    value.replace("~1", "/").replace("~0", "~")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn common_schema() -> Value {
        json!({
            "$id": COMMON_SCHEMA_ID,
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["assumptions", "confidence", "risks", "next_action"],
            "properties": {
                "assumptions": { "type": "array", "items": { "type": "string" } },
                "confidence": { "type": "integer", "minimum": 0, "maximum": 10 },
                "risks": { "type": "array", "items": { "type": "string" } },
                "next_action": { "type": "string" }
            }
        })
    }

    #[test]
    fn extracts_first_parseable_json_fence() {
        let extracted = extract_json_payload("before\n```json\n{\"ok\":true}\n```\nafter").unwrap();

        assert_eq!(extracted.extraction, ExtractionRule::Fence);
        assert_eq!(extracted.value, json!({"ok": true}));
        assert!(!extracted.fallback_used);
    }

    #[test]
    fn falls_back_to_last_balanced_object_when_fence_is_missing() {
        let extracted =
            extract_json_payload("notes {not json}\n{\"message\":\"brace } inside\",\"n\":1}")
                .unwrap();

        assert_eq!(extracted.extraction, ExtractionRule::Fallback);
        assert_eq!(
            extracted.value,
            json!({"message": "brace } inside", "n": 1})
        );
    }

    #[test]
    fn falls_back_when_fenced_block_is_not_json() {
        let extracted = extract_json_payload("```json\nnot json\n```\nthen {\"ok\":true}").unwrap();

        assert_eq!(extracted.extraction, ExtractionRule::Fallback);
        assert!(extracted.fallback_used);
        assert_eq!(extracted.value, json!({"ok": true}));
    }

    #[test]
    fn balanced_scan_ignores_unbalanced_prose() {
        let extracted = extract_json_payload("bad { prose { here }\nfinal {\"ok\":true}").unwrap();

        assert_eq!(extracted.value, json!({"ok": true}));
    }

    #[test]
    fn schema_validator_resolves_common_schema_locally() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "allOf": [
                { "$ref": COMMON_SCHEMA_ID },
                {
                    "type": "object",
                    "required": ["analysis"],
                    "properties": {
                        "analysis": { "type": "string" }
                    }
                }
            ]
        });
        let validator = SchemaValidator::new(schema, common_schema()).unwrap();

        let errors = validator
            .validate(&json!({
                "assumptions": [],
                "confidence": 11,
                "risks": [],
                "next_action": "retry",
                "analysis": "x"
            }))
            .unwrap();

        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].instance_path, "/confidence");
        assert_eq!(errors[0].keyword, "maximum");
    }
}
