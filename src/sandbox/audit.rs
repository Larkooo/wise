// Append-only JSONL audit log writer.
//
// Every sandboxed call writes a "started" line *before* the network round
// trip and a "completed" / "failed" line after. The pre-call line guarantees
// we have a record even if the process crashes mid-call. The post-call line
// carries the result + duration so the operator can audit success rates.
//
// File format:
//
//   {"ts":"2026-04-07T20:00:00Z","sandbox":"coding-agent","seq":1,
//    "phase":"started","command":"agent.fetch","args":{"--card":"tok_..."},
//    "justify":"Stripe checkout"}
//   {"ts":"2026-04-07T20:00:01Z","sandbox":"coding-agent","seq":1,
//    "phase":"completed","duration_ms":342}
//
// File mode is 0600. Writes are O_APPEND so concurrent writers from
// different processes interleave at line boundaries (POSIX append guarantee
// for writes ≤PIPE_BUF on local filesystems).

use anyhow::{Context as _, Result};
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;

/// Handle to one ongoing audit entry — call `complete()` or `fail()` on
/// drop completion. The Drop impl writes a `phase=dropped` line if neither
/// was called, so a panic between pre and post still leaves a trail.
///
/// `id` is a UUID-v4 string so the started/completed pair can be correlated
/// across processes. A previous version used a process-local AtomicU64 seq
/// which collided whenever two CLI invocations wrote to the same log.
pub struct AuditEntry {
    sandbox: String,
    id: String,
    started: Instant,
    log_path: PathBuf,
    finalized: bool,
}

impl AuditEntry {
    pub fn complete(mut self, summary: Value) -> Result<()> {
        let line = json!({
            "ts": now_iso(),
            "sandbox": self.sandbox,
            "id": self.id,
            "phase": "completed",
            "duration_ms": self.started.elapsed().as_millis() as u64,
            "summary": summary,
        });
        write_line(&self.log_path, &line)?;
        self.finalized = true;
        Ok(())
    }

    pub fn fail(mut self, error: &str) -> Result<()> {
        let line = json!({
            "ts": now_iso(),
            "sandbox": self.sandbox,
            "id": self.id,
            "phase": "failed",
            "duration_ms": self.started.elapsed().as_millis() as u64,
            "error": error,
        });
        write_line(&self.log_path, &line)?;
        self.finalized = true;
        Ok(())
    }
}

impl Drop for AuditEntry {
    fn drop(&mut self) {
        if self.finalized {
            return;
        }
        let line = json!({
            "ts": now_iso(),
            "sandbox": self.sandbox,
            "id": self.id,
            "phase": "dropped",
            "duration_ms": self.started.elapsed().as_millis() as u64,
        });
        // Best-effort: dropping in error paths shouldn't double-error.
        let _ = write_line(&self.log_path, &line);
    }
}

#[derive(Debug, Serialize)]
pub struct StartContext<'a> {
    pub command: &'a str,
    pub args: Value,
    pub justify: Option<&'a str>,
}

/// Begin an audit entry. Writes the "started" line synchronously and
/// returns a handle that must be `complete`d or `fail`ed.
pub fn start(
    log_path: &Path,
    sandbox: &str,
    ctx: StartContext<'_>,
) -> Result<AuditEntry> {
    let id = Uuid::new_v4().to_string();
    let line = json!({
        "ts": now_iso(),
        "sandbox": sandbox,
        "id": id,
        "phase": "started",
        "command": ctx.command,
        "args": ctx.args,
        "justify": ctx.justify,
    });
    write_line(log_path, &line)?;
    Ok(AuditEntry {
        sandbox: sandbox.to_string(),
        id,
        started: Instant::now(),
        log_path: log_path.to_path_buf(),
        finalized: false,
    })
}

fn write_line(path: &Path, value: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating audit dir {}", parent.display()))?;
    }
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts
        .open(path)
        .with_context(|| format!("opening audit log {}", path.display()))?;
    let mut line = serde_json::to_vec(value).context("serializing audit line")?;
    line.push(b'\n');
    file.write_all(&line).context("writing audit line")?;
    Ok(())
}

fn now_iso() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn writes_started_and_completed_lines() {
        let dir = std::env::temp_dir().join(format!("wise-audit-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("audit.jsonl");
        let entry = start(
            &path,
            "test",
            StartContext {
                command: "agent.fetch",
                args: json!({"--card": "tok_x"}),
                justify: Some("for testing"),
            },
        )
        .unwrap();
        entry.complete(json!({"ok": true})).unwrap();
        let mut s = String::new();
        std::fs::File::open(&path).unwrap().read_to_string(&mut s).unwrap();
        let lines: Vec<&str> = s.trim().split('\n').collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"phase\":\"started\""));
        assert!(lines[0].contains("\"command\":\"agent.fetch\""));
        assert!(lines[0].contains("\"justify\":\"for testing\""));
        assert!(lines[1].contains("\"phase\":\"completed\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn drop_writes_dropped_marker() {
        let dir = std::env::temp_dir().join(format!("wise-audit-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("audit.jsonl");
        {
            let _entry = start(
                &path,
                "test",
                StartContext {
                    command: "x.y",
                    args: json!({}),
                    justify: None,
                },
            )
            .unwrap();
            // entry dropped without complete/fail
        }
        let s = std::fs::read_to_string(&path).unwrap();
        assert!(s.contains("\"phase\":\"dropped\""), "got: {s}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
