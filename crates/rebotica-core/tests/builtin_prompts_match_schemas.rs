//! Drift guard: each built-in `prompts/runs.d/<mode>/prompt.md` ships an
//! example JSON block that must validate against the mode's `schema.json`.
//!
//! If this test fails, the prompt and schema have drifted apart and any
//! well-formed model response following the prompt will be rejected by the
//! validator at runtime. Fix by aligning the prompt example with the schema
//! (schema is the source of truth) and re-running.

use rebotica_core::run::{extract_json_payload, SchemaValidator};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const MODES: &[&str] = &["review", "tests", "explain", "patch"];

fn runs_d_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("prompts")
        .join("runs.d")
        .canonicalize()
        .expect("prompts/runs.d/ should exist at workspace root")
}

fn load_common_schema() -> Value {
    let path = runs_d_dir().join("_common").join("runs-common.schema.json");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
}

#[test]
fn every_builtin_mode_prompt_example_validates_against_its_schema() {
    let common_schema = load_common_schema();
    let mut failures = Vec::new();

    for mode in MODES {
        let mode_dir = runs_d_dir().join(mode);
        let prompt_path = mode_dir.join("prompt.md");
        let schema_path = mode_dir.join("schema.json");

        let prompt = fs::read_to_string(&prompt_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", prompt_path.display()));
        let schema_text = fs::read_to_string(&schema_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", schema_path.display()));
        let schema: Value = serde_json::from_str(&schema_text)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", schema_path.display()));

        let extracted = match extract_json_payload(&prompt) {
            Ok(extracted) => extracted,
            Err(err) => {
                failures.push(format!(
                    "{mode}: no parseable JSON example in prompt.md ({err}). \
                     Add a fenced ```json``` block under \"Output format\" \
                     showing a valid example."
                ));
                continue;
            }
        };

        let validator = match SchemaValidator::new(schema, common_schema.clone()) {
            Ok(validator) => validator,
            Err(err) => {
                failures.push(format!("{mode}: schema failed to compile: {err}"));
                continue;
            }
        };

        match validator.validate(&extracted.value) {
            Ok(errors) if errors.is_empty() => {}
            Ok(errors) => {
                let detail = errors
                    .iter()
                    .map(|e| format!("    {} ({}): {}", e.instance_path, e.keyword, e.message))
                    .collect::<Vec<_>>()
                    .join("\n");
                failures.push(format!(
                    "{mode}: prompt example does not validate against schema.json:\n{detail}"
                ));
            }
            Err(err) => failures.push(format!("{mode}: validator error: {err}")),
        }
    }

    assert!(
        failures.is_empty(),
        "prompt/schema drift detected in built-in run.* modes:\n\n{}",
        failures.join("\n\n")
    );
}
