use crate::output::Envelope;
use serde::Serialize;
use std::io::{self, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReporterMode {
    Human,
    Json,
    Quiet,
}

pub struct Reporter {
    mode: ReporterMode,
    stdout: Box<dyn Write + Send>,
    stderr: Box<dyn Write + Send>,
}

impl Reporter {
    pub fn from_flags(json: bool, quiet: bool) -> Self {
        Self::from_mode(ReporterMode::from_flags(json, quiet))
    }

    pub fn from_env_and_flags(json: bool, quiet: bool) -> Self {
        let json = json || env_truthy("REBOTICA_JSON");
        let quiet = quiet || env_truthy("REBOTICA_QUIET");
        Self::from_flags(json, quiet)
    }

    pub fn from_mode(mode: ReporterMode) -> Self {
        Self {
            mode,
            stdout: Box::new(io::stdout()),
            stderr: Box::new(io::stderr()),
        }
    }

    pub fn with_writers(
        mode: ReporterMode,
        stdout: Box<dyn Write + Send>,
        stderr: Box<dyn Write + Send>,
    ) -> Self {
        Self {
            mode,
            stdout,
            stderr,
        }
    }

    pub fn mode(&self) -> ReporterMode {
        self.mode
    }

    pub fn is_json(&self) -> bool {
        self.mode.is_json()
    }

    pub fn progress(&mut self, msg: &str) -> io::Result<()> {
        if self.mode == ReporterMode::Quiet {
            return Ok(());
        }
        writeln!(self.stderr, "{msg}")
    }

    pub fn warn(&mut self, msg: &str) -> io::Result<()> {
        self.progress(msg)
    }

    pub fn human(&mut self, msg: &str) -> io::Result<()> {
        if self.mode != ReporterMode::Human {
            return Ok(());
        }
        writeln!(self.stdout, "{msg}")
    }

    pub fn emit<T: Serialize>(&mut self, env: &Envelope<T>) -> io::Result<()> {
        if !self.mode.is_json() {
            return Ok(());
        }
        serde_json::to_writer_pretty(&mut self.stdout, env)?;
        writeln!(self.stdout)?;
        self.stdout.flush()
    }
}

impl Drop for Reporter {
    fn drop(&mut self) {
        let _ = self.stdout.flush();
        let _ = self.stderr.flush();
    }
}

impl ReporterMode {
    pub fn from_flags(json: bool, quiet: bool) -> Self {
        if quiet {
            Self::Quiet
        } else if json {
            Self::Json
        } else {
            Self::Human
        }
    }

    pub fn is_json(self) -> bool {
        matches!(self, Self::Json | Self::Quiet)
    }
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::Envelope;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct SharedWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedWriter {
        fn output(&self) -> String {
            String::from_utf8(self.bytes.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for SharedWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn reporter(mode: ReporterMode) -> (Reporter, SharedWriter, SharedWriter) {
        let stdout = SharedWriter::default();
        let stderr = SharedWriter::default();
        (
            Reporter::with_writers(mode, Box::new(stdout.clone()), Box::new(stderr.clone())),
            stdout,
            stderr,
        )
    }

    #[test]
    fn reporter_human_writes_human_to_stdout_progress_to_stderr() {
        let (mut reporter, stdout, stderr) = reporter(ReporterMode::Human);

        reporter.human("hello").unwrap();
        reporter.progress("working").unwrap();
        reporter.emit(&Envelope::builder("doctor").build()).unwrap();

        assert_eq!(stdout.output(), "hello\n");
        assert_eq!(stderr.output(), "working\n");
    }

    #[test]
    fn reporter_json_writes_envelope_to_stdout_progress_to_stderr() {
        let (mut reporter, stdout, stderr) = reporter(ReporterMode::Json);

        reporter.human("hello").unwrap();
        reporter.progress("working").unwrap();
        reporter.emit(&Envelope::builder("doctor").build()).unwrap();

        let value: serde_json::Value = serde_json::from_str(&stdout.output()).unwrap();
        assert_eq!(value["rebotica"], "v1");
        assert_eq!(stderr.output(), "working\n");
    }

    #[test]
    fn reporter_quiet_writes_envelope_to_stdout_silent_stderr() {
        let (mut reporter, stdout, stderr) = reporter(ReporterMode::Quiet);

        reporter.progress("working").unwrap();
        reporter.warn("careful").unwrap();
        reporter.emit(&Envelope::builder("doctor").build()).unwrap();

        let value: serde_json::Value = serde_json::from_str(&stdout.output()).unwrap();
        assert_eq!(value["rebotica"], "v1");
        assert_eq!(stderr.output(), "");
    }

    #[test]
    fn reporter_quiet_implies_json() {
        let reporter = Reporter::from_flags(false, true);

        assert_eq!(reporter.mode(), ReporterMode::Quiet);
        assert!(reporter.is_json());
    }

    #[test]
    fn reporter_from_env_respects_rebotica_json() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _json = EnvGuard::set("REBOTICA_JSON", "yes");
        let _quiet = EnvGuard::clear("REBOTICA_QUIET");
        let reporter = Reporter::from_env_and_flags(false, false);

        assert_eq!(reporter.mode(), ReporterMode::Json);
    }

    #[test]
    fn reporter_from_env_respects_rebotica_quiet() {
        let _lock = ENV_LOCK.lock().unwrap();
        let _json = EnvGuard::clear("REBOTICA_JSON");
        let _quiet = EnvGuard::set("REBOTICA_QUIET", "true");
        let reporter = Reporter::from_env_and_flags(false, false);

        assert_eq!(reporter.mode(), ReporterMode::Quiet);
    }

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        key: &'static str,
        original: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let original = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, original }
        }

        fn clear(key: &'static str) -> Self {
            let original = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.original {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}
