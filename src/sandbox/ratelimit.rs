// Sliding-window rate limiter backed by the audit log itself.
//
// We don't keep a separate rate-limit DB. The audit log already records
// every started call with a timestamp; the rate limiter tails the file
// (last ~1k lines is plenty for any sane window) and counts started lines
// for the same command path within the configured window.
//
// This means rate limits track *attempts* (started lines), not successes.
// That's the right semantics for an agent: a failed call still consumed a
// budget slot, otherwise an agent could grind through limits by retrying
// against transient failures.

use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::Duration;

/// Returns the number of started entries for `command` in the trailing
/// `window`, looking only at the audit file at `log_path`. If the file does
/// not exist yet the count is 0.
pub fn count_recent(log_path: &Path, command: &str, window: Duration) -> Result<u32> {
    if !log_path.exists() {
        return Ok(0);
    }
    let file = File::open(log_path)
        .with_context(|| format!("opening audit log {}", log_path.display()))?;
    let cutoff = Utc::now()
        - chrono::Duration::from_std(window).unwrap_or_else(|_| chrono::Duration::seconds(0));
    let reader = BufReader::new(file);
    let mut count = 0u32;
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        // Cheap field probe before parsing — most started lines are short.
        if !line.contains("\"phase\":\"started\"") {
            continue;
        }
        if !line.contains(&format!("\"command\":\"{command}\"")) {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts_str = match value.get("ts").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => continue,
        };
        let ts = match DateTime::parse_from_rfc3339(ts_str) {
            Ok(t) => t.with_timezone(&Utc),
            Err(_) => continue,
        };
        if ts >= cutoff {
            count += 1;
        }
    }
    Ok(count)
}

/// Returns Ok(()) if the call would still fit in the budget, otherwise
/// an error explaining the limit and how long until the oldest in-window
/// call expires.
pub fn check(
    log_path: &Path,
    command: &str,
    limit: u32,
    window: Duration,
) -> Result<()> {
    let n = count_recent(log_path, command, window)?;
    if n >= limit {
        anyhow::bail!(
            "sandbox rate limit exceeded for `{command}`: {n}/{limit} in the last {}s. \
             Wait for older calls to age out before retrying.",
            window.as_secs()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::audit;
    use serde_json::json;
    use std::time::Duration;

    fn temp_log(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wise-rl-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("audit.jsonl")
    }

    #[test]
    fn empty_log_zero_count() {
        let p = temp_log("empty");
        assert_eq!(count_recent(&p, "x.y", Duration::from_secs(60)).unwrap(), 0);
    }

    #[test]
    fn counts_only_matching_command() {
        let p = temp_log("matching");
        for cmd in ["a.b", "a.b", "x.y", "a.b"] {
            let entry = audit::start(
                &p,
                "test",
                audit::StartContext {
                    command: cmd,
                    args: json!({}),
                    justify: None,
                },
            )
            .unwrap();
            entry.complete(json!({})).unwrap();
        }
        assert_eq!(count_recent(&p, "a.b", Duration::from_secs(60)).unwrap(), 3);
        assert_eq!(count_recent(&p, "x.y", Duration::from_secs(60)).unwrap(), 1);
        assert_eq!(count_recent(&p, "z.z", Duration::from_secs(60)).unwrap(), 0);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn check_blocks_when_over_limit() {
        let p = temp_log("over");
        for _ in 0..3 {
            let entry = audit::start(
                &p,
                "test",
                audit::StartContext {
                    command: "agent.fetch",
                    args: json!({}),
                    justify: None,
                },
            )
            .unwrap();
            entry.complete(json!({})).unwrap();
        }
        assert!(check(&p, "agent.fetch", 5, Duration::from_secs(3600)).is_ok());
        let err = check(&p, "agent.fetch", 3, Duration::from_secs(3600)).unwrap_err();
        assert!(err.to_string().contains("rate limit exceeded"));
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
