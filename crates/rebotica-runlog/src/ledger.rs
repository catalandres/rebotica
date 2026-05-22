//! Apprentice ledger — DuckDB-backed event log that lets v0.3+ commands
//! answer "what has the apprentice been doing and how is it going."
//!
//! The ledger lives at `~/.rebotica/ledger.duckdb`. It is an append-only
//! event store; the schema never alters existing rows. Schema changes ship
//! as a `schema_version` bump (tracked in `ledger_meta`) with a
//! forward-only migration.
//!
//! Ledgers created before v0.3's DuckDB switch live at `~/.rebotica/ledger.db`
//! (SQLite). On first open, [`open`] migrates that file forward via DuckDB's
//! sqlite extension and backs the original up as `ledger.db.sqlite-backup`.
//!
//! Per-run files under `~/.rebotica/runs/<id>/` remain the audit trail; the
//! ledger is the queryable summary.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use duckdb::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::{make_id, root, Disposition};

/// Current schema version. Bump and add a migration when the table or view
/// shape changes incompatibly.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

/// Path to the apprentice ledger database (`~/.rebotica/ledger.duckdb`).
pub fn path() -> PathBuf {
    root().join("ledger.duckdb")
}

/// Path to the pre-v0.3 SQLite ledger, migrated forward on first open.
fn legacy_sqlite_path() -> PathBuf {
    root().join("ledger.db")
}

/// Open the ledger, creating the database and applying migrations as needed.
///
/// Safe to call on every event write: schema setup is idempotent and runs
/// in microseconds once the file exists. On the very first open after the
/// DuckDB switch, a pre-existing SQLite `ledger.db` is migrated forward
/// (best-effort: a failure logs a warning and starts with an empty DuckDB
/// ledger, leaving the SQLite file untouched for manual recovery).
pub fn open() -> Result<Connection> {
    let db_path = path();
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let legacy = legacy_sqlite_path();
    let needs_migration = !db_path.exists() && legacy.is_file();

    let conn = open_with_retry(&db_path)?;
    ensure_schema(&conn)?;

    if needs_migration {
        match migrate_from_sqlite(&conn, &legacy) {
            Ok(n) => {
                let backup = root().join("ledger.db.sqlite-backup");
                match std::fs::rename(&legacy, &backup) {
                    Ok(()) => eprintln!(
                        "rebotica: migrated {n} ledger event(s) from SQLite to DuckDB; \
                         original preserved at {}",
                        backup.display()
                    ),
                    Err(error) => eprintln!(
                        "rebotica: migrated {n} ledger event(s) from SQLite to DuckDB, but could \
                         not rename the original ({error}); it remains at {} and will not be \
                         re-imported (DuckDB ledger now exists).",
                        legacy.display()
                    ),
                }
            }
            Err(error) => {
                eprintln!(
                    "warning: could not migrate the SQLite ledger to DuckDB ({error:#}); \
                     starting with an empty DuckDB ledger. The old data is preserved at {} \
                     — retry once the duckdb sqlite extension is reachable.",
                    legacy.display()
                );
            }
        }
    }

    Ok(conn)
}

/// Number of times to retry acquiring the DuckDB file lock before giving up.
const OPEN_RETRY_ATTEMPTS: usize = 6;

/// Open the DuckDB ledger, retrying with exponential backoff while another
/// process holds the file lock.
///
/// DuckDB takes an exclusive lock per read-write connection (unlike SQLite's
/// WAL, which allowed concurrent readers). rbtc opens the ledger in short
/// write-then-close bursts, so brief contention — a `compare` loop, the MCP
/// server writing while the CLI reads — resolves within a few milliseconds.
fn open_with_retry(db_path: &Path) -> Result<Connection> {
    let mut delay = std::time::Duration::from_millis(25);
    for attempt in 0..OPEN_RETRY_ATTEMPTS {
        match Connection::open(db_path) {
            Ok(conn) => return Ok(conn),
            Err(error) if attempt + 1 < OPEN_RETRY_ATTEMPTS && is_lock_contention(&error) => {
                std::thread::sleep(delay);
                delay = delay.saturating_mul(2);
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to open ledger at {}", db_path.display()));
            }
        }
    }
    unreachable!("loop returns on the final attempt")
}

/// Whether a DuckDB open error is transient file-lock contention worth
/// retrying (as opposed to a real corruption/permission failure).
fn is_lock_contention(error: &duckdb::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("lock")
        || message.contains("conflict")
        || message.contains("being used")
        || message.contains("busy")
}

fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS ledger_meta (key VARCHAR PRIMARY KEY, value BIGINT);",
    )
    .context("failed to ensure ledger_meta table")?;

    let version = schema_version(conn)?;
    if version > CURRENT_SCHEMA_VERSION {
        anyhow::bail!(
            "ledger schema_version is {version}, this build supports up to {CURRENT_SCHEMA_VERSION}; \
             upgrade rbtc to read this ledger",
        );
    }
    if version < 1 {
        apply_v1(conn)?;
        set_schema_version(conn, 1)?;
    }
    Ok(())
}

/// Read the recorded schema version, or 0 when the ledger is brand new.
fn schema_version(conn: &Connection) -> Result<i64> {
    let mut stmt = conn
        .prepare("SELECT value FROM ledger_meta WHERE key = 'schema_version'")
        .context("failed to prepare schema_version query")?;
    let mut rows = stmt
        .query_map([], |row| row.get::<_, i64>(0))
        .context("failed to query schema_version")?;
    let value = rows
        .next()
        .transpose()
        .context("failed to read schema_version row")?;
    Ok(value.unwrap_or(0))
}

fn set_schema_version(conn: &Connection, version: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO ledger_meta (key, value) VALUES ('schema_version', ?) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        params![version],
    )
    .context("failed to set schema_version")?;
    Ok(())
}

fn apply_v1(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        CREATE SEQUENCE IF NOT EXISTS ledger_events_id_seq START 1;

        CREATE TABLE IF NOT EXISTS ledger_events (
            id           BIGINT  PRIMARY KEY DEFAULT nextval('ledger_events_id_seq'),
            ts           VARCHAR NOT NULL,
            run_id       VARCHAR,
            event_type   VARCHAR NOT NULL,
            payload_json VARCHAR NOT NULL
        );

        CREATE INDEX IF NOT EXISTS ledger_events_run_id_idx ON ledger_events(run_id);
        CREATE INDEX IF NOT EXISTS ledger_events_type_idx ON ledger_events(event_type);
        CREATE INDEX IF NOT EXISTS ledger_events_ts_idx ON ledger_events(ts);

        CREATE OR REPLACE VIEW v_per_model_stats AS
        SELECT
            json_extract_string(payload_json, '$.model')          AS model,
            json_extract_string(payload_json, '$.envelope_shape') AS envelope_shape,
            COUNT(*)                                               AS completed_runs,
            SUM(CASE WHEN json_extract_string(payload_json, '$.ok') = 'true' THEN 1 ELSE 0 END) AS ok_runs,
            AVG(CAST(json_extract_string(payload_json, '$.confidence') AS DOUBLE))         AS avg_confidence,
            AVG(CAST(json_extract_string(payload_json, '$.hallucination_rate') AS DOUBLE)) AS avg_hallucination_rate,
            MAX(ts)                                                AS latest_ts
        FROM ledger_events
        WHERE event_type = 'run_completed'
        GROUP BY model, envelope_shape;

        CREATE OR REPLACE VIEW v_per_envelope_stats AS
        SELECT
            json_extract_string(payload_json, '$.envelope_shape') AS envelope_shape,
            COUNT(*)                                               AS completed_runs,
            SUM(CASE WHEN json_extract_string(payload_json, '$.ok') = 'true' THEN 1 ELSE 0 END) AS ok_runs,
            AVG(CAST(json_extract_string(payload_json, '$.confidence') AS DOUBLE))         AS avg_confidence,
            AVG(CAST(json_extract_string(payload_json, '$.hallucination_rate') AS DOUBLE)) AS avg_hallucination_rate
        FROM ledger_events
        WHERE event_type = 'run_completed'
        GROUP BY envelope_shape;

        CREATE OR REPLACE VIEW v_disposition_breakdown AS
        SELECT
            json_extract_string(payload_json, '$.disposition') AS disposition,
            COUNT(*)                                            AS rows_count
        FROM ledger_events
        WHERE event_type = 'prime_disposition'
        GROUP BY disposition;
        "#,
    )
    .context("failed to apply ledger v1 schema")?;

    Ok(())
}

/// Copy events forward from a pre-v0.3 SQLite ledger using DuckDB's sqlite
/// extension. Returns the post-import row count. Errors propagate so the
/// caller can degrade gracefully (the SQLite file is left in place).
fn migrate_from_sqlite(conn: &Connection, sqlite_path: &Path) -> Result<usize> {
    conn.execute_batch("INSTALL sqlite; LOAD sqlite;")
        .context("failed to load the duckdb sqlite extension")?;
    // `sqlite_path` is always rebotica's own `~/.rebotica/ledger.db`, never
    // user input. The single-quote escape guards against an unusual home dir.
    let escaped = sqlite_path.display().to_string().replace('\'', "''");
    conn.execute_batch(&format!(
        "ATTACH '{escaped}' AS legacy_sqlite (TYPE SQLITE, READ_ONLY);"
    ))
    .context("failed to attach the legacy SQLite ledger")?;
    conn.execute_batch(
        "INSERT INTO ledger_events (ts, run_id, event_type, payload_json) \
         SELECT ts, run_id, event_type, payload_json \
         FROM legacy_sqlite.ledger_events ORDER BY id;",
    )
    .context("failed to copy events from the legacy SQLite ledger")?;
    let imported = count_with_conn(conn)?;
    conn.execute_batch("DETACH legacy_sqlite;")
        .context("failed to detach the legacy SQLite ledger")?;
    Ok(imported as usize)
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
    /// A dispatch attempt that failed before reaching the
    /// run-allocation stage (e.g. over_limit, guard_rejected, missing
    /// model). Has no matching `run_started`/`run_completed` pair.
    RunRejected,
}

impl EventType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RunStarted => "run_started",
            Self::RunCompleted => "run_completed",
            Self::PrimeDisposition => "prime_disposition",
            Self::ScoreRecorded => "score_recorded",
            Self::RunRejected => "run_rejected",
        }
    }
}

/// Logical input shape an event describes. One variant per MCP tool (when
/// MCP lands in #45) plus one per CLI `run.*` mode.
///
/// The `*_Freeform` variants tag runs invoked with `--no-schema`: their
/// envelope kind is suffixed `.freeform` and they bypass extraction and
/// validation entirely. Keeping them as distinct shapes lets per-mode
/// aggregations (#66) avoid mixing structured and freeform corpora.
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
    RunReviewFreeform,
    RunTestsFreeform,
    RunExplainFreeform,
    RunPatchFreeform,
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
            Self::RunReviewFreeform => "run_review_freeform",
            Self::RunTestsFreeform => "run_tests_freeform",
            Self::RunExplainFreeform => "run_explain_freeform",
            Self::RunPatchFreeform => "run_patch_freeform",
        }
    }

    /// Map a `run.*` envelope kind (e.g. `"run.review"`) to its shape.
    /// Recognises the `.freeform` suffix that `--no-schema` runs carry.
    pub fn from_run_kind(kind: &str) -> Option<Self> {
        match kind {
            "run.review" => Some(Self::RunReview),
            "run.tests" => Some(Self::RunTests),
            "run.explain" => Some(Self::RunExplain),
            "run.patch" => Some(Self::RunPatch),
            "run.review.freeform" => Some(Self::RunReviewFreeform),
            "run.tests.freeform" => Some(Self::RunTestsFreeform),
            "run.explain.freeform" => Some(Self::RunExplainFreeform),
            "run.patch.freeform" => Some(Self::RunPatchFreeform),
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
    RunRejected(RunRejectedPayload),
}

impl Event {
    pub fn event_type(&self) -> EventType {
        match self {
            Self::RunStarted(_) => EventType::RunStarted,
            Self::RunCompleted(_) => EventType::RunCompleted,
            Self::PrimeDisposition(_) => EventType::PrimeDisposition,
            Self::ScoreRecorded(_) => EventType::ScoreRecorded,
            Self::RunRejected(_) => EventType::RunRejected,
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
    /// Apprentice-side `usage.prompt_tokens` reported by the provider
    /// (e.g. LM Studio). `None` when the provider does not report `usage`
    /// or when the run failed before any provider response arrived.
    /// Together with `apprentice_completion_tokens`, this is the exact
    /// local-model cost — the denominator for net-savings analyses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apprentice_prompt_tokens: Option<u64>,
    /// Apprentice-side `usage.completion_tokens`. See
    /// `apprentice_prompt_tokens` for semantics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apprentice_completion_tokens: Option<u64>,
    /// Byte length of the `data` field returned to Prime (the structured
    /// envelope payload). Proxy for Prime's roundtrip context cost
    /// (~bytes/4 tokens). `None` when the run failed before a parsed
    /// envelope existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope_bytes: Option<u64>,
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

/// Payload for a [`Event::RunRejected`] event — a dispatch attempt that
/// bailed before reaching run-allocation. Carries enough context for
/// `rbtc runs show` to render an abbreviated rejection card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRejectedPayload {
    /// Best-effort kind (e.g. `run.review`). Falls back to `"run"` when
    /// the rejection happened before plugin resolution.
    pub kind: String,
    /// Snake-case `ErrorCode` name (e.g. `over_limit`, `config`).
    pub error_code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Append a [`Event::RunRejected`] for a pre-persistence failure and
/// return the freshly-generated `run_id` so callers can surface it
/// (e.g. CLI prints it in the error envelope; the ledger has the row).
///
/// Best-effort: a ledger write failure logs a warning to stderr and
/// returns an empty string. Callers should treat the returned id as
/// optional information, not as a hard guarantee.
pub fn record_rejection(payload: RunRejectedPayload) -> String {
    let run_id = make_id();
    let event = Event::RunRejected(payload);
    if let Err(error) = append_event(Some(&run_id), &event) {
        eprintln!("warning: failed to record run_rejected in ledger: {error:#}");
        return String::new();
    }
    run_id
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
    // DuckDB has no `last_insert_rowid`; `RETURNING` hands back the
    // sequence-assigned id directly.
    let id: i64 = conn
        .query_row(
            "INSERT INTO ledger_events (ts, run_id, event_type, payload_json) \
             VALUES (?, ?, ?, ?) RETURNING id",
            params![
                ts.to_rfc3339(),
                run_id,
                event.event_type().as_str(),
                payload
            ],
            |row| row.get(0),
        )
        .context("failed to insert ledger event")?;
    Ok(id)
}

/// Count rows in `ledger_events`. Cheap helper for tests and debugging.
pub fn count_events() -> Result<i64> {
    let conn = open()?;
    count_with_conn(&conn)
}

fn count_with_conn(conn: &Connection) -> Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM ledger_events", [], |row| row.get(0))
        .context("failed to count ledger events")
}

/// Summary row returned by [`list_recent_runs`] and [`run_summary`].
///
/// Aggregates the latest `run_started` / `run_completed` /
/// `prime_disposition` rows for a given `run_id` so callers don't need
/// to re-join the ledger themselves. Pre-persistence rejections
/// (recorded as standalone `run_rejected` events) are surfaced with
/// `rejected = true`; for those rows `ok = Some(false)` and
/// `started_at` is the rejection timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: String,
    /// The `run.*` envelope kind (e.g. `run.review`). Falls back to the
    /// scorecard mode if no `run_completed` event is present.
    pub kind: String,
    pub envelope_shape: Option<EnvelopeShape>,
    pub model: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    /// Whether the run completed successfully. `None` if no
    /// `run_completed` event has been written (e.g. SIGKILL mid-run).
    pub ok: Option<bool>,
    pub error_code: Option<String>,
    pub duration_ms: Option<u64>,
    pub disposition: Disposition,
    /// `true` when this summary describes a dispatch attempt that was
    /// rejected before allocating a persisted run (no `run_started` /
    /// `run_completed` pair). Surfaces over_limit, guard_rejected,
    /// missing-model, and config failures in `rbtc runs list`.
    #[serde(default)]
    pub rejected: bool,
}

/// List recent runs from the ledger, newest first.
///
/// Includes both started runs (`run_started`) and pre-persistence
/// rejections (`run_rejected`). Optionally filter by `kind` (e.g.
/// `"run.review"`) and/or `model`. Note: rejections never have a model
/// recorded, so a `model_filter` will exclude them by construction.
/// If `limit` is `None`, all rows are returned.
pub fn list_recent_runs(
    limit: Option<usize>,
    kind_filter: Option<&str>,
    model_filter: Option<&str>,
) -> Result<Vec<RunSummary>> {
    let conn = open()?;
    // Fetch candidate run ids newest-first, then apply kind/model filters
    // in Rust against the assembled summary. The ledger is small (hundreds
    // of rows), so this is cheaper than threading dynamic JSON predicates
    // through DuckDB's parameter binding.
    let mut stmt = conn
        .prepare(
            "SELECT run_id, MAX(ts) AS latest_ts FROM ledger_events \
             WHERE run_id IS NOT NULL \
               AND event_type IN ('run_started', 'run_rejected') \
             GROUP BY run_id ORDER BY latest_ts DESC",
        )
        .context("failed to prepare runs query")?;
    let run_ids: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .context("failed to query runs")?
        .collect::<duckdb::Result<Vec<_>>>()
        .context("failed to read runs result set")?;

    let mut summaries = Vec::new();
    for run_id in run_ids {
        let Some(summary) = run_summary_with_conn(&conn, &run_id)? else {
            continue;
        };
        if let Some(kind) = kind_filter {
            if summary.kind != kind {
                continue;
            }
        }
        if let Some(model) = model_filter {
            // `run_rejected` events never carry a model, so a model filter
            // excludes rejections by construction.
            if summary.model.as_deref() != Some(model) {
                continue;
            }
        }
        summaries.push(summary);
        if let Some(max) = limit {
            if summaries.len() >= max {
                break;
            }
        }
    }
    Ok(summaries)
}

/// Summarize a single run. Returns `Ok(None)` if no events exist for
/// `run_id` in the ledger.
pub fn run_summary(run_id: &str) -> Result<Option<RunSummary>> {
    let conn = open()?;
    run_summary_with_conn(&conn, run_id)
}

fn run_summary_with_conn(conn: &Connection, run_id: &str) -> Result<Option<RunSummary>> {
    let started = latest_payload(conn, run_id, EventType::RunStarted)?;
    let completed = latest_payload(conn, run_id, EventType::RunCompleted)?;
    let disposition_payload = latest_payload(conn, run_id, EventType::PrimeDisposition)?;
    let rejected = latest_payload(conn, run_id, EventType::RunRejected)?;

    if started.is_none() && completed.is_none() && rejected.is_none() {
        return Ok(None);
    }

    let mut summary = RunSummary {
        run_id: run_id.to_string(),
        kind: String::new(),
        envelope_shape: None,
        model: None,
        started_at: None,
        ok: None,
        error_code: None,
        duration_ms: None,
        disposition: Disposition::Unscored,
        rejected: false,
    };

    if let Some((payload, ts)) = started {
        if let Ok(parsed) = serde_json::from_str::<RunStartedPayload>(&payload) {
            summary.kind = parsed.kind;
            summary.envelope_shape = Some(parsed.envelope_shape);
            summary.model = Some(parsed.model);
        }
        summary.started_at = Some(ts);
    }

    if let Some((payload, _ts)) = completed {
        if let Ok(parsed) = serde_json::from_str::<RunCompletedPayload>(&payload) {
            if summary.kind.is_empty() {
                summary.kind = parsed.kind;
            }
            if summary.envelope_shape.is_none() {
                summary.envelope_shape = Some(parsed.envelope_shape);
            }
            if summary.model.is_none() {
                summary.model = Some(parsed.model);
            }
            summary.ok = Some(parsed.ok);
            summary.error_code = parsed.error_code;
            summary.duration_ms = Some(parsed.duration_ms);
        }
    }

    if let Some((payload, ts)) = rejected {
        if let Ok(parsed) = serde_json::from_str::<RunRejectedPayload>(&payload) {
            if summary.kind.is_empty() {
                summary.kind = parsed.kind.clone();
            }
            if summary.envelope_shape.is_none() {
                summary.envelope_shape = EnvelopeShape::from_run_kind(&parsed.kind);
            }
            if summary.error_code.is_none() {
                summary.error_code = Some(parsed.error_code);
            }
            summary.rejected = true;
            if summary.ok.is_none() {
                summary.ok = Some(false);
            }
            if summary.started_at.is_none() {
                summary.started_at = Some(ts);
            }
        }
    }

    if let Some((payload, _ts)) = disposition_payload {
        if let Ok(parsed) = serde_json::from_str::<PrimeDispositionPayload>(&payload) {
            summary.disposition = parsed.disposition;
        }
    }

    Ok(Some(summary))
}

fn latest_payload(
    conn: &Connection,
    run_id: &str,
    event_type: EventType,
) -> Result<Option<(String, DateTime<Utc>)>> {
    let mut stmt = conn
        .prepare(
            "SELECT payload_json, ts FROM ledger_events \
             WHERE run_id = ? AND event_type = ? \
             ORDER BY id DESC LIMIT 1",
        )
        .with_context(|| format!("failed to prepare {} query", event_type.as_str()))?;
    let mut rows = stmt
        .query_map(params![run_id, event_type.as_str()], |row| {
            let payload: String = row.get(0)?;
            let ts: String = row.get(1)?;
            Ok((payload, ts))
        })
        .with_context(|| format!("failed to query {} for {run_id}", event_type.as_str()))?;
    let row = rows
        .next()
        .transpose()
        .context("failed to read event row")?;
    Ok(row.and_then(|(payload, ts)| {
        DateTime::parse_from_rfc3339(&ts)
            .ok()
            .map(|dt| (payload, dt.with_timezone(&Utc)))
    }))
}

/// Look up the model recorded for `run_id` by its `run_started` event.
///
/// Returns `Ok(None)` if no `run_started` event exists or its payload is
/// malformed. Used by failure paths in `dispatch_run` so `run_completed`
/// events emitted on error still carry the resolved model.
pub fn model_for_run(run_id: &str) -> Result<Option<String>> {
    let conn = open()?;
    let mut stmt = conn
        .prepare(
            "SELECT payload_json FROM ledger_events \
             WHERE run_id = ? AND event_type = 'run_started' \
             ORDER BY id DESC LIMIT 1",
        )
        .context("failed to prepare run_started model lookup")?;
    let mut rows = stmt
        .query_map(params![run_id], |row| row.get::<_, String>(0))
        .context("failed to query run_started for model lookup")?;
    let row: Option<String> = rows
        .next()
        .transpose()
        .context("failed to read run_started row")?;
    Ok(row.and_then(|payload| {
        serde_json::from_str::<serde_json::Value>(&payload)
            .ok()
            .and_then(|v| v.get("model").and_then(|m| m.as_str()).map(String::from))
    }))
}

/// Fetch the most recent event of any type for a given `run_id`, if any.
pub fn latest_event_for_run(run_id: &str) -> Result<Option<(EventType, String)>> {
    let conn = open()?;
    let mut stmt = conn
        .prepare(
            "SELECT event_type, payload_json FROM ledger_events \
             WHERE run_id = ? ORDER BY id DESC LIMIT 1",
        )
        .context("failed to prepare latest event query")?;
    let mut rows = stmt
        .query_map(params![run_id], |row| {
            let event_type: String = row.get(0)?;
            let payload: String = row.get(1)?;
            Ok((event_type, payload))
        })
        .context("failed to query latest event")?;
    let row = rows
        .next()
        .transpose()
        .context("failed to read latest event row")?;

    Ok(row.map(|(type_str, payload)| {
        let event_type = match type_str.as_str() {
            "run_started" => EventType::RunStarted,
            "run_completed" => EventType::RunCompleted,
            "prime_disposition" => EventType::PrimeDisposition,
            "score_recorded" => EventType::ScoreRecorded,
            "run_rejected" => EventType::RunRejected,
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
        assert_eq!(schema_version(&conn).unwrap(), 1);

        // Views must exist.
        for view in [
            "v_per_model_stats",
            "v_per_envelope_stats",
            "v_disposition_breakdown",
        ] {
            let exists: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM duckdb_views() WHERE view_name = ?",
                    params![view],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(exists, 1, "view {view} should exist");
        }
    }

    #[test]
    fn envelope_shape_recognises_freeform_kinds() {
        // `--no-schema` runs (#66) carry `.freeform`-suffixed kinds. The
        // ledger must distinguish them from structured runs so per-mode
        // aggregations don't mix corpora.
        assert_eq!(
            EnvelopeShape::from_run_kind("run.review.freeform"),
            Some(EnvelopeShape::RunReviewFreeform)
        );
        assert_eq!(
            EnvelopeShape::from_run_kind("run.tests.freeform"),
            Some(EnvelopeShape::RunTestsFreeform)
        );
        assert_eq!(
            EnvelopeShape::from_run_kind("run.explain.freeform"),
            Some(EnvelopeShape::RunExplainFreeform)
        );
        assert_eq!(
            EnvelopeShape::from_run_kind("run.patch.freeform"),
            Some(EnvelopeShape::RunPatchFreeform)
        );
        // Structured kinds are unchanged.
        assert_eq!(
            EnvelopeShape::from_run_kind("run.review"),
            Some(EnvelopeShape::RunReview)
        );
        // Unknown suffix does not match.
        assert_eq!(EnvelopeShape::from_run_kind("run.review.bogus"), None);
        // Round-trip through as_str must be stable.
        assert_eq!(
            EnvelopeShape::RunReviewFreeform.as_str(),
            "run_review_freeform"
        );
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
                    apprentice_prompt_tokens: None,
                    apprentice_completion_tokens: None,
                    envelope_bytes: None,
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
        assert_eq!(schema_version(&conn).unwrap(), 1);
    }

    #[test]
    fn record_rejection_surfaces_in_summary_and_listing() {
        let _lock = env_lock();
        let (_temp, _home) = fresh_home("ledger-rejection");

        let id = record_rejection(RunRejectedPayload {
            kind: "run.review".to_string(),
            error_code: "over_limit".to_string(),
            message: "diff exceeds 50000 lines".to_string(),
            details: Some(serde_json::json!({ "lines": 60000 })),
        });
        assert!(!id.is_empty(), "rejection should return a run_id");

        // run_summary recognises the rejected run despite the absence of
        // any run_started / run_completed events.
        let summary = run_summary(&id).unwrap().expect("summary should exist");
        assert!(summary.rejected, "rejected flag should be set");
        assert_eq!(summary.kind, "run.review");
        assert_eq!(summary.envelope_shape, Some(EnvelopeShape::RunReview));
        assert_eq!(summary.ok, Some(false));
        assert_eq!(summary.error_code.as_deref(), Some("over_limit"));
        assert!(summary.model.is_none());
        assert!(summary.started_at.is_some());

        // list_recent_runs includes rejections (it now scans both
        // run_started and run_rejected event types).
        let summaries = list_recent_runs(Some(10), None, None).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].run_id, id);
        assert!(summaries[0].rejected);

        // model filter excludes rejections by construction (no model on
        // the rejection payload).
        let filtered = list_recent_runs(Some(10), None, Some("any")).unwrap();
        assert!(filtered.is_empty());

        // kind filter still matches.
        let kind_match = list_recent_runs(Some(10), Some("run.review"), None).unwrap();
        assert_eq!(kind_match.len(), 1);
    }

    #[test]
    fn future_schema_version_is_rejected() {
        let _lock = env_lock();
        let (_temp, _home) = fresh_home("ledger-future");

        // Open once to create the DuckDB ledger at v1, then bump the
        // recorded schema_version past what this build supports.
        {
            let conn = open().unwrap();
            set_schema_version(&conn, 99).unwrap();
        }

        let err = open().expect_err("future schema_version should be rejected");
        assert!(
            err.to_string().contains("schema_version is 99"),
            "got: {err}"
        );
    }
}
