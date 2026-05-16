use chrono::{DateTime, Utc};
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use std::fmt;
use std::time::Duration;

const ENVELOPE_VERSION: &str = "v1";

#[derive(Debug, Clone, Serialize)]
pub struct Envelope<T: Serialize> {
    pub rebotica: &'static str,
    pub kind: &'static str,
    pub ok: bool,
    pub command: String,
    pub data: T,
    pub error: Option<EnvelopeError>,
    pub run_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvelopeError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Usage,
    Config,
    ProviderUnavailable,
    ProviderServerError,
    ProviderClientError,
    GuardRejected,
    OutputInvalid,
    OverLimit,
    Cancelled,
    Internal,
}

impl ErrorCode {
    pub fn all() -> &'static [ErrorCode] {
        &[
            Self::Internal,
            Self::Usage,
            Self::Config,
            Self::ProviderUnavailable,
            Self::ProviderServerError,
            Self::ProviderClientError,
            Self::GuardRejected,
            Self::OutputInvalid,
            Self::OverLimit,
            Self::Cancelled,
        ]
    }

    pub fn exit_code(self) -> i32 {
        match self {
            Self::Internal => 1,
            Self::Usage => 2,
            Self::Config => 3,
            Self::ProviderUnavailable => 10,
            Self::ProviderServerError => 11,
            Self::ProviderClientError => 12,
            Self::GuardRejected => 20,
            Self::OutputInvalid => 21,
            Self::OverLimit => 22,
            Self::Cancelled => 130,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodedCommandError {
    code: ErrorCode,
    message: String,
}

impl CodedCommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for CodedCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CodedCommandError {}

#[derive(Debug, Clone, Copy, Default)]
pub struct EmptyData;

impl Serialize for EmptyData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_map(Some(0))?.end()
    }
}

#[derive(Debug, Clone)]
pub struct EnvelopeBuilder<T> {
    kind: &'static str,
    command: String,
    data: T,
    error: Option<EnvelopeError>,
    run_id: Option<String>,
    started_at: DateTime<Utc>,
}

impl Envelope<EmptyData> {
    pub fn builder(kind: &'static str) -> EnvelopeBuilder<EmptyData> {
        EnvelopeBuilder {
            kind,
            command: kind.to_string(),
            data: EmptyData,
            error: None,
            run_id: None,
            started_at: Utc::now(),
        }
    }
}

impl<T> EnvelopeBuilder<T> {
    pub fn command(mut self, command: impl Into<String>) -> Self {
        self.command = command.into();
        self
    }

    pub fn data<U>(self, data: U) -> EnvelopeBuilder<U> {
        EnvelopeBuilder {
            kind: self.kind,
            command: self.command,
            data,
            error: self.error,
            run_id: self.run_id,
            started_at: self.started_at,
        }
    }

    pub fn error(mut self, error: EnvelopeError) -> Self {
        self.error = Some(error);
        self
    }

    pub fn run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn started_at(mut self, started_at: DateTime<Utc>) -> Self {
        self.started_at = started_at;
        self
    }

    pub fn build(self) -> Envelope<T>
    where
        T: Serialize,
    {
        self.build_at(Utc::now())
    }

    pub fn build_at(self, now: DateTime<Utc>) -> Envelope<T>
    where
        T: Serialize,
    {
        let duration_ms = now
            .signed_duration_since(self.started_at)
            .to_std()
            .unwrap_or_else(|_| Duration::from_millis(0))
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let ok = self.error.is_none();
        Envelope {
            rebotica: ENVELOPE_VERSION,
            kind: self.kind,
            ok,
            command: self.command,
            data: self.data,
            error: self.error,
            run_id: self.run_id,
            started_at: self.started_at,
            duration_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    #[test]
    fn envelope_serializes_v1_shape_for_success() {
        let started_at = Utc.with_ymd_and_hms(2026, 5, 15, 22, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 5, 15, 22, 0, 1).unwrap();

        let envelope = Envelope::builder("doctor")
            .command("doctor")
            .started_at(started_at)
            .data(json!({"checks": []}))
            .build_at(now);

        let value = serde_json::to_value(envelope).unwrap();
        assert_eq!(value["rebotica"], "v1");
        assert_eq!(value["kind"], "doctor");
        assert_eq!(value["ok"], true);
        assert_eq!(value["command"], "doctor");
        assert_eq!(value["data"], json!({"checks": []}));
        assert_eq!(value["error"], serde_json::Value::Null);
        assert_eq!(value["run_id"], serde_json::Value::Null);
        assert_eq!(value["duration_ms"], 1000);
    }

    #[test]
    fn envelope_serializes_v1_shape_for_failure() {
        let envelope = Envelope::builder("error")
            .command("doctor")
            .data(json!({"checks": []}))
            .error(EnvelopeError {
                code: ErrorCode::Config,
                message: "invalid config".to_string(),
                details: Some(json!({"path": ".rebotica.yml"})),
            })
            .build();

        let value = serde_json::to_value(envelope).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"]["code"], "config");
        assert_eq!(value["error"]["message"], "invalid config");
        assert_eq!(value["error"]["details"]["path"], ".rebotica.yml");
        assert_eq!(value["data"], json!({"checks": []}));
    }

    #[test]
    fn envelope_omits_details_when_none() {
        let envelope = Envelope::builder("error")
            .error(EnvelopeError {
                code: ErrorCode::Internal,
                message: "failed".to_string(),
                details: None,
            })
            .build();

        let value = serde_json::to_value(envelope).unwrap();
        assert!(value["error"].get("details").is_none());
    }

    #[test]
    fn error_code_exit_codes_are_stable() {
        assert_eq!(ErrorCode::Internal.exit_code(), 1);
        assert_eq!(ErrorCode::Usage.exit_code(), 2);
        assert_eq!(ErrorCode::Config.exit_code(), 3);
        assert_eq!(ErrorCode::ProviderUnavailable.exit_code(), 10);
        assert_eq!(ErrorCode::ProviderServerError.exit_code(), 11);
        assert_eq!(ErrorCode::ProviderClientError.exit_code(), 12);
        assert_eq!(ErrorCode::GuardRejected.exit_code(), 20);
        assert_eq!(ErrorCode::OutputInvalid.exit_code(), 21);
        assert_eq!(ErrorCode::OverLimit.exit_code(), 22);
        assert_eq!(ErrorCode::Cancelled.exit_code(), 130);
    }

    #[test]
    fn error_code_all_lists_the_public_contract_order() {
        assert_eq!(
            ErrorCode::all(),
            &[
                ErrorCode::Internal,
                ErrorCode::Usage,
                ErrorCode::Config,
                ErrorCode::ProviderUnavailable,
                ErrorCode::ProviderServerError,
                ErrorCode::ProviderClientError,
                ErrorCode::GuardRejected,
                ErrorCode::OutputInvalid,
                ErrorCode::OverLimit,
                ErrorCode::Cancelled,
            ]
        );
    }

    #[test]
    fn cancelled_error_code_serializes_as_snake_case() {
        let envelope = Envelope::builder("error")
            .error(EnvelopeError {
                code: ErrorCode::Cancelled,
                message: "operation cancelled".to_string(),
                details: None,
            })
            .build();

        let value = serde_json::to_value(envelope).unwrap();
        assert_eq!(value["error"]["code"], "cancelled");
    }

    #[test]
    fn coded_command_error_carries_code_and_message() {
        let error = CodedCommandError::new(ErrorCode::Config, "invalid config");

        assert_eq!(error.code(), ErrorCode::Config);
        assert_eq!(error.message(), "invalid config");
        assert_eq!(error.to_string(), "invalid config");
    }

    #[test]
    fn envelope_pretty_prints_by_default() {
        let envelope = Envelope::builder("doctor").build();
        let output = serde_json::to_string_pretty(&envelope).unwrap();

        assert!(output.starts_with("{\n  \"rebotica\": \"v1\""));
    }

    #[test]
    fn envelope_builder_computes_duration_ms() {
        let started_at = Utc.with_ymd_and_hms(2026, 5, 15, 22, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 5, 15, 22, 0, 0).unwrap()
            + chrono::Duration::milliseconds(42);

        let envelope = Envelope::builder("doctor")
            .started_at(started_at)
            .build_at(now);

        assert_eq!(envelope.duration_ms, 42);
    }

    #[test]
    fn envelope_with_empty_data_serializes_data_as_empty_object() {
        let envelope = Envelope::builder("doctor").build();
        let output = serde_json::to_string_pretty(&envelope).unwrap();

        assert!(output.contains("\"data\": {}"));
        assert!(!output.contains("\"data\": null"));
    }
}
