//! Apprentice ledger — SQLite-backed event log that lets v0.3+ commands
//! answer "what has the apprentice been doing and how is it going."
//!
//! The ledger lives at `~/.rebotica/ledger.db`. It is an append-only event
//! store; the [`Schema`] never alters existing rows. Schema changes ship as
//! a new `user_version` bump with a forward-only migration.
//!
//! Per-run files under `~/.rebotica/runs/<id>/` remain the audit trail; the
//! ledger is the queryable summary.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::{root, Disposition};

/// Current schema version. Bump and add a migration when the table or view
/// shape changes incompatibly.
pub const CURRENT_USER_VERSION: u32 = 1;

/// Path to the apprentice ledger database (`~/.rebotica/ledger.db`).
pub fn path() -> PathBuf {
    root().join("ledger.db")
}

/// Open the ledger, creating the database and applying migrations as needed.
///
/// Safe to call on every event write: schema setup is idempotent and runs
/// in microseconds once the file exists.
pub fn open() -> Result<Connection> {
    let db_path = path();
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open ledger at {}", db_path.display()))?;
    apply_migrations(&conn)?;
    Ok(conn)
}

fn apply_migrations(conn: &Connection) -> Result<()> {
    let version: u32 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .context("failed to read PRAGMA user_version")?;

    if version > CURRENT_USER_VERSION {
        anyhow::bail!(
            "ledger user_version is {version}, this build supports up to {CURRENT_USER_VERSION}; \
             upgrade rbtc to read this ledger",
        );
    }

    if version < 1 {
        apply_v1(conn)?;
    }

    Ok(())
}

fn apply_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS ledger_events (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            ts           TEXT    NOT NULL,
            run_id       TEXT,
            event_type   TEXT    NOT NULL,
            payload_json TEXT    NOT NULL
        );

        CREATE INDEX IF NOT EXISTS ledger_events_run_id_idx ON ledger_events(run_id);
        CREATE INDEX IF NOT EXISTS ledger_events_type_idx ON ledger_events(event_type);
        CREATE INDEX IF NOT EXISTS ledger_events_ts_idx ON ledger_events(ts);

        DROP VIEW IF EXISTS v_per_model_stats;
        CREATE VIEW v_per_model_stats AS
        SELECT
            json_extract(payload_json, '$.model')          AS model,
            json_extract(payload_json, '$.envelope_shape') AS envelope_shape,
            COUNT(*)                                       AS completed_runs,
            SUM(CASE WHEN json_extract(payload_json, '$.ok') = 1 THEN 1 ELSE 0 END) AS ok_runs,
            AVG(json_extract(payload_json, '$.confidence'))         AS avg_confidence,
            AVG(json_extract(payload_json, '$.hallucination_rate')) AS avg_hallucination_rate,
            MAX(ts)                                                  AS latest_ts
        FROM ledger_events
        WHERE event_type = 'run_completed'
        GROUP BY model, envelope_shape;

        DROP VIEW IF EXISTS v_per_envelope_stats;
        CREATE VIEW v_per_envelope_stats AS
        SELECT
            json_extract(payload_json, '$.envelope_shape') AS envelope_shape,
            COUNT(*)                                       AS completed_runs,
            SUM(CASE WHEN json_extract(payload_json, '$.ok') = 1 THEN 1 ELSE 0 END) AS ok_runs,
            AVG(json_extract(payload_json, '$.confidence'))         AS avg_confidence,
            AVG(json_extract(payload_json, '$.hallucination_rate')) AS avg_hallucination_rate
        FROM ledger_events
        WHERE event_type = 'run_completed'
        GROUP BY envelope_shape;

        DROP VIEW IF EXISTS v_disposition_breakdown;
        CREATE VIEW v_disposition_breakdown AS
        SELECT
            json_extract(payload_json, '$.disposition') AS disposition,
            COUNT(*)                                    AS rows_count
        FROM ledger_events
        WHERE event_type = 'prime_disposition'
        GROUP BY disposition;
        "#,
    )
    .context("failed to apply ledger v1 schema")?;

    conn.pragma_update(None, "user_version", 1u32)
        .context("failed to set user_version to 1")?;

    Ok(())
}

/// Event kinds written to the ledger. Each ships a typed payload via the
/// [`Event`] enum; new event types extend the enum (additive only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    RunStarted,
    RunCompleted,
    PrimeDisposition,
    ScoreRecorded,
}

impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RunStarted => "run_started",
            Self::RunCompleted => "run_completed",
            Self::PrimeDisposition => "prime_disposition",
            Self::ScoreRecorded => "score_recorded",
        }
    }
}

/// Logical input shape an event describes. One variant per MCP tool (when
/// MCP lands in #45) plus one per CLI `run.*` mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeShape {
    ReviewDiff,
    ProposeTests,
    ExplainFiles,
    HealthCheck,
    RunReview,
    RunTests,
    RunExplain,
    RunPatch,
}

impl EnvelopeShape {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReviewDiff => "review_diff",
            Self::ProposeTests => "propose_tests",
            Self::ExplainFiles => "explain_files",
            Self::HealthCheck => "health_check",
            Self::RunReview => "run_review",
            Self::RunTests => "run_tests",
            Self::RunExplain => "run_explain",
            Self::RunPatch => "run_patch",
        }
    }

    /// Map a `run.*` envelope kind (e.g. `"run.review"`) to its shape.
    pub fn from_run_kind(kind: &str) -> Option<Self> {
        match kind {
            "run.review" => Some(Self::RunReview),
            "run.tests" => Some(Self::RunTests),
            "run.explain" => Some(Self::RunExplain),
            "run.patch" => Some(Self::RunPatch),
            _ => None,
        }
    }
}

/// Strongly-typed event payloads. The serde JSON shape lands verbatim in
/// `ledger_events.payload_json`; the [`EventType`] matches the variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Event {
    RunStarted(RunStartedPayload),
    RunCompleted(RunCompletedPayload),
    PrimeDisposition(PrimeDispositionPayload),
    ScoreRecorded(ScoreRecordedPayload),
}

impl Event {
    pub fn event_type(&self) -> EventType {
        match self {
            Self::RunStarted(_) => EventType::RunStarted,
            Self::RunCompleted(_) => EventType::RunCompleted,
            Self::PrimeDisposition(_) => EventType::PrimeDisposition,
            Self::ScoreRecorded(_) => EventType::ScoreRecorded,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStartedPayload {
    pub kind: String,
    pub envelope_shape: EnvelopeShape,
    pub model: String,
    pub provider: String,
    pub contract_version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunCompletedPayload {
    pub kind: String,
    pub envelope_shape: EnvelopeShape,
    /// Denormalized from the matching `run_started` event so derived views
    /// can aggregate without a self-join.
    pub model: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_bytes: Option<u64>,
    /// Populated by a future hallucination-rate writer (deferred from #44).
    /// `None` until that writer ships.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hallucination_rate: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrimeDispositionPayload {
    pub disposition: Disposition,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoreRecordedPayload {
    pub axis: String,
    pub score: i64,
}

/// Append a typed event to the ledger. Returns the assigned `id`.
///
/// Use [`append_event_at`] when an explicit timestamp is required (tests,
/// batch backfills); the default uses the current UTC time.
pub fn append_event(run_id: Option<&str>, event: &Event) -> Result<i64> {
    append_event_at(run_id, event, Utc::now())
}

/// Append with an explicit timestamp.
pub fn append_event_at(run_id: Option<&str>, event: &Event, ts: DateTime<Utc>) -> Result<i64> {
    let conn = open()?;
    insert_event(&conn, run_id, event, ts)
}

fn insert_event(
    conn: &Connection,
    run_id: Option<&str>,
    event: &Event,
    ts: DateTime<Utc>,
) -> Result<i64> {
    let payload = serde_json::to_string(event).context("failed to serialize ledger payload")?;
    conn.execute(
        "INSERT INTO ledger_events (ts, run_id, event_type, payload_json) VALUES (?1, ?2, ?3, ?4)",
        params![
            ts.to_rfc3339(),
            run_id,
            event.event_type().as_str(),
            payload
        ],
    )
    .context("failed to insert ledger event")?;
    Ok(conn.last_insert_rowid())
}

/// Count rows in `ledger_events`. Cheap helper for tests and debugging.
pub fn count_events() -> Result<i64> {
    let conn = open()?;
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM ledger_events", [], |row| row.get(0))
        .context("failed to count ledger events")?;
    Ok(count)
}

/// Look up the model recorded for `run_id` by its `run_started` event.
///
/// Returns `Ok(None)` if no `run_started` event exists or its payload is
/// malformed. Used by failure paths in `dispatch_run` so `run_completed`
/// events emitted on error still carry the resolved model.
pub fn model_for_run(run_id: &str) -> Result<Option<String>> {
    let conn = open()?;
    let row: Option<String> = conn
        .query_row(
            "SELECT payload_json FROM ledger_events \
             WHERE run_id = ?1 AND event_type = 'run_started' \
             ORDER BY id DESC LIMIT 1",
            params![run_id],
            |row| row.get(0),
        )
        .optional()
        .context("failed to query run_started for model lookup")?;
    Ok(row.and_then(|payload| {
        serde_json::from_str::<serde_json::Value>(&payload)
            .ok()
            .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(String::from))
    }))
}

/// Fetch the most recent event of any type for a given `run_id`, if any.
pub fn latest_event_for_run(run_id: &str) -> Result<Option<(EventType, String)>> {
    let conn = open()?;
    let row = conn
        .query_row(
            "SELECT event_type, payload_json FROM ledger_events \
             WHERE run_id = ?1 ORDER BY id DESC LIMIT 1",
            params![run_id],
            |row| {
                let event_type: String = row.get(0)?;
                let payload: String = row.get(1)?;
                Ok((event_type, payload))
            },
        )
        .optional()
        .context("failed to query latest event")?;

    Ok(row.map(|(type_str, payload)| {
        let event_type = match type_str.as_str() {
            "run_started" => EventType::RunStarted,
            "run_completed" => EventType::RunCompleted,
            "prime_disposition" => EventType::PrimeDisposition,
            "score_recorded" => EventType::ScoreRecorded,
            _ => EventType::RunStarted, // unknown future variants fall through to a safe default
        };
        (event_type, payload)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{env_lock, EnvGuard, TempDir};

    fn fresh_home(name: &str) -> (TempDir, EnvGuard) {
        let temp = TempDir::new(name);
        let guard = EnvGuard::set("HOME", temp.path());
        (temp, guard)
    }

    #[test]
    fn fresh_ledger_applies_v1_schema() {
        let _lock = env_lock();
        let (_temp, _home) = fresh_home("ledger-fresh");

        let conn = open().unwrap();
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);

        // Views must exist.
        for view in [
            "v_per_model_stats",
            "v_per_envelope_stats",
            "v_disposition_breakdown",
        ] {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='view' AND name=?1",
                    params![view],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "view {view} should exist");
        }
    }

    #[test]
    fn append_event_round_trips_payload() {
        let _lock = env_lock();
        let (_temp, _home) = fresh_home("ledger-roundtrip");

        let event = Event::RunStarted(RunStartedPayload {
            kind: "run.review".to_string(),
            envelope_shape: EnvelopeShape::RunReview,
            model: "qwen-coder-32b".to_string(),
            provider: "lm-studio".to_string(),
            contract_version: 1,
        });
        let id = append_event(Some("run-1"), &event).unwrap();
        assert!(id > 0);
        assert_eq!(count_events().unwrap(), 1);

        let (kind, payload) = latest_event_for_run("run-1").unwrap().unwrap();
        assert_eq!(kind, EventType::RunStarted);
        let parsed: Event = serde_json::from_str(&payload).unwrap();
        match parsed {
            Event::RunStarted(p) => {
                assert_eq!(p.kind, "run.review");
                assert_eq!(p.envelope_shape, EnvelopeShape::RunReview);
            }
            _ => panic!("expected RunStarted"),
        }
    }

    #[test]
    fn views_aggregate_completed_runs() {
        let _lock = env_lock();
        let (_temp, _home) = fresh_home("ledger-views");

        for (model, ok, confidence) in [
            ("qwen", true, 7u8),
            ("qwen", true, 8),
            ("qwen", false, 4),
            ("gemma", true, 6),
        ] {
            append_event(
                Some("any"),
                &Event::RunCompleted(RunCompletedPayload {
                    kind: "run.review".to_string(),
                    envelope_shape: EnvelopeShape::RunReview,
                    model: model.to_string(),
                    ok,
                    error_code: if ok {
                        None
                    } else {
                        Some("output_invalid".to_string())
                    },
                    duration_ms: 100,
                    output_bytes: Some(256),
                    hallucination_rate: None,
                    confidence: Some(confidence),
                }),
            )
            .unwrap();
        }

        let conn = open().unwrap();
        let mut stmt = conn
            .prepare("SELECT model, completed_runs, ok_runs FROM v_per_model_stats ORDER BY model")
            .unwrap();
        let rows: Vec<(String, i64, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(
            rows,
            vec![("gemma".to_string(), 1, 1), ("qwen".to_string(), 3, 2),]
        );
    }

    #[test]
    fn migration_idempotent_when_called_twice() {
        let _lock = env_lock();
        let (_temp, _home) = fresh_home("ledger-idempotent");

        let _conn = open().unwrap();
        let _conn = open().unwrap();
        let _conn = open().unwrap();

        let conn = open().unwrap();
        let version: u32 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn future_user_version_is_rejected() {
        let _lock = env_lock();
        let (temp, _home) = fresh_home("ledger-future");

        std::fs::create_dir_all(temp.path().join(".rebotica")).unwrap();
        let conn = Connection::open(temp.path().join(".rebotica/ledger.db")).unwrap();
        conn.pragma_update(None, "user_version", 99u32).unwrap();
        drop(conn);

        let err = open().expect_err("future user_version should be rejected");
        assert!(err.to_string().contains("user_version is 99"));
    }
}
