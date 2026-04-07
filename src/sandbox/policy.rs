// Sandbox policy data + matching logic.
//
// A Policy is the parsed representation of one TOML sandbox file. The
// matching logic implements the dot-path glob language documented in
// SANDBOX.md: literal segments, single-segment wildcards (`*`),
// `prefix.*` and `*.suffix` shortcuts, and the special `*` global wildcard.
//
// We deliberately do *not* pull in `globset` or any other crate — the
// language is small enough that a focused implementation is shorter than
// the dependency edge.

use anyhow::{bail, Context as _, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Top-level policy mirroring the TOML schema.
#[derive(Debug, Deserialize, Clone)]
pub struct Policy {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub profiles: Option<Vec<i64>>,
    #[serde(default)]
    pub cards: Option<Vec<String>>,
    #[serde(default)]
    pub balances: Option<Vec<i64>>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub conditions: HashMap<String, Conditions>,
    #[serde(default)]
    pub escalation: Escalation,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct Conditions {
    #[serde(default)]
    pub rate_limit: Option<String>,
    #[serde(default)]
    pub require_justification: bool,
    #[serde(default)]
    pub audit: Option<PathBuf>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct Escalation {
    #[serde(default = "default_escalation_mode")]
    pub mode: EscalationMode,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub timeout: Option<String>,
}

fn default_escalation_mode() -> EscalationMode {
    EscalationMode::Deny
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EscalationMode {
    Deny,
    Tty,
    Command,
}

impl Default for EscalationMode {
    fn default() -> Self {
        EscalationMode::Deny
    }
}

/// Result of evaluating one command path against one policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The command is allowed by an explicit allow rule, no deny override.
    Allow,
    /// The command is not in the allow list (or no allow list at all).
    NotAllowed,
    /// The command was matched by a deny rule, overriding any allow.
    Denied { rule: String },
}

impl Policy {
    /// Validate the loaded policy. Catches obvious mistakes early so
    /// `wise sandbox check` and the dispatch gate fail loudly rather than
    /// silently denying everything.
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            bail!("policy.name is required");
        }
        if self.allow.is_empty() {
            bail!(
                "policy.allow is empty — sandbox '{}' would deny everything. \
                 Add at least one rule (use `*` to allow all).",
                self.name
            );
        }
        for pat in self.allow.iter().chain(self.deny.iter()) {
            validate_pattern(pat)?;
        }
        if matches!(self.escalation.mode, EscalationMode::Command) && self.escalation.command.is_none() {
            bail!(
                "escalation.mode = 'command' requires escalation.command to be set"
            );
        }
        Ok(())
    }

    /// Decide whether `path` is allowed under this policy. Argument-aware
    /// denies (`path:key=value`) are matched against `args`.
    pub fn check(&self, path: &str, args: &[(String, String)]) -> Decision {
        // Deny rules are evaluated first; any match wins regardless of
        // whether allow would have permitted the command.
        for rule in &self.deny {
            if rule_matches(rule, path, args) {
                return Decision::Denied { rule: rule.clone() };
            }
        }
        for rule in &self.allow {
            if rule_matches(rule, path, args) {
                return Decision::Allow;
            }
        }
        Decision::NotAllowed
    }

    /// Returns true if `id` is allowed by the profile scoping list.
    /// `None` (no list configured) means unrestricted.
    pub fn check_profile(&self, id: i64) -> bool {
        match &self.profiles {
            None => true,
            Some(list) => list.contains(&id),
        }
    }

    pub fn check_card(&self, token: &str) -> bool {
        match &self.cards {
            None => true,
            Some(list) => list.iter().any(|t| t == token),
        }
    }

    pub fn check_balance(&self, id: i64) -> bool {
        match &self.balances {
            None => true,
            Some(list) => list.contains(&id),
        }
    }
}

/// Parse a `"3/hour"` style rate limit into (count, window).
pub fn parse_rate_limit(s: &str) -> Result<(u32, Duration)> {
    let (count, unit) = s
        .split_once('/')
        .with_context(|| format!("rate limit must look like `3/hour`, got `{s}`"))?;
    let count: u32 = count.trim().parse().context("rate limit count")?;
    let window = match unit.trim() {
        "second" | "sec" | "s" => Duration::from_secs(1),
        "minute" | "min" | "m" => Duration::from_secs(60),
        "hour" | "hr" | "h" => Duration::from_secs(3600),
        "day" | "d" => Duration::from_secs(86_400),
        other => bail!("unknown rate limit unit `{other}`; use second/minute/hour/day"),
    };
    Ok((count, window))
}

// ---------- internal: pattern matching ----------

fn validate_pattern(pat: &str) -> Result<()> {
    if pat.is_empty() {
        bail!("empty pattern in allow/deny list");
    }
    // Strip the optional `:key=value` argument constraint.
    let (path_part, _arg_part) = split_arg_constraint(pat);
    for seg in path_part.split('.') {
        if seg.is_empty() {
            bail!("empty segment in pattern `{pat}`");
        }
        // Only `*` and bare identifiers are allowed in segments.
        if seg != "*" && !seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            bail!("invalid segment `{seg}` in pattern `{pat}`");
        }
    }
    Ok(())
}

fn split_arg_constraint(rule: &str) -> (&str, Option<(&str, &str)>) {
    if let Some((path, arg)) = rule.split_once(':') {
        if let Some((k, v)) = arg.split_once('=') {
            return (path, Some((k, v)));
        }
    }
    (rule, None)
}

fn rule_matches(rule: &str, path: &str, args: &[(String, String)]) -> bool {
    let (path_pat, arg_constraint) = split_arg_constraint(rule);
    if !path_glob_matches(path_pat, path) {
        return false;
    }
    match arg_constraint {
        None => true,
        Some((key, val)) => args
            .iter()
            .any(|(k, v)| (k == key || k == &format!("--{key}")) && v == val),
    }
}

/// Match a glob pattern against a dot-path. Supports:
///   - exact:        `balance.move`           matches `balance.move`
///   - any segment:  `balance.*`              matches `balance.move` (one seg)
///   - leading any:  `*.list`                 matches `transfer.list`
///   - global any:   `*`                      matches everything
///
/// We deliberately do not support `**` (multi-segment) in v1 — the surface
/// is small enough that explicit prefixes are clearer.
fn path_glob_matches(pat: &str, path: &str) -> bool {
    if pat == "*" {
        return true;
    }
    let pat_segs: Vec<&str> = pat.split('.').collect();
    let path_segs: Vec<&str> = path.split('.').collect();
    if pat_segs.len() != path_segs.len() {
        return false;
    }
    pat_segs
        .iter()
        .zip(path_segs.iter())
        .all(|(p, s)| *p == "*" || p == s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pol(allow: &[&str], deny: &[&str]) -> Policy {
        Policy {
            name: "t".into(),
            description: None,
            profiles: None,
            cards: None,
            balances: None,
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: deny.iter().map(|s| s.to_string()).collect(),
            conditions: HashMap::new(),
            escalation: Escalation::default(),
        }
    }

    #[test]
    fn exact_match_allow() {
        let p = pol(&["balance.list"], &[]);
        assert_eq!(p.check("balance.list", &[]), Decision::Allow);
        assert_eq!(p.check("balance.move", &[]), Decision::NotAllowed);
    }

    #[test]
    fn segment_wildcard() {
        let p = pol(&["balance.*"], &[]);
        assert_eq!(p.check("balance.list", &[]), Decision::Allow);
        assert_eq!(p.check("balance.move", &[]), Decision::Allow);
        // Wildcard does not match across dots.
        assert_eq!(p.check("balance.permissions.set", &[]), Decision::NotAllowed);
    }

    #[test]
    fn leading_wildcard() {
        let p = pol(&["*.list"], &[]);
        assert_eq!(p.check("balance.list", &[]), Decision::Allow);
        assert_eq!(p.check("transfer.list", &[]), Decision::Allow);
        assert_eq!(p.check("transfer.create", &[]), Decision::NotAllowed);
    }

    #[test]
    fn global_wildcard() {
        let p = pol(&["*"], &[]);
        assert_eq!(p.check("anything", &[]), Decision::Allow);
        assert_eq!(p.check("a.b.c.d", &[]), Decision::Allow);
    }

    #[test]
    fn deny_overrides_allow() {
        let p = pol(&["balance.*"], &["balance.move"]);
        match p.check("balance.move", &[]) {
            Decision::Denied { rule } => assert_eq!(rule, "balance.move"),
            other => panic!("expected Denied, got {other:?}"),
        }
        assert_eq!(p.check("balance.list", &[]), Decision::Allow);
    }

    #[test]
    fn arg_aware_deny() {
        let p = pol(
            &["card.status"],
            &["card.status:status=ACTIVE"],
        );
        let active_args = vec![("status".into(), "ACTIVE".into())];
        let frozen_args = vec![("status".into(), "FROZEN".into())];
        match p.check("card.status", &active_args) {
            Decision::Denied { .. } => {}
            other => panic!("expected Denied, got {other:?}"),
        }
        assert_eq!(p.check("card.status", &frozen_args), Decision::Allow);
    }

    #[test]
    fn validate_rejects_empty_allow() {
        let p = pol(&[], &[]);
        let err = p.validate().unwrap_err();
        assert!(err.to_string().contains("would deny everything"));
    }

    #[test]
    fn validate_rejects_bad_segment() {
        let mut p = pol(&["balance.move!"], &[]);
        p.name = "x".into();
        let err = p.validate().unwrap_err();
        assert!(err.to_string().contains("invalid segment"));
    }

    #[test]
    fn validate_command_escalation_requires_command() {
        let mut p = pol(&["*"], &[]);
        p.escalation.mode = EscalationMode::Command;
        let err = p.validate().unwrap_err();
        assert!(err.to_string().contains("escalation.command"));
    }

    #[test]
    fn rate_limit_parser() {
        assert_eq!(parse_rate_limit("3/hour").unwrap(), (3, Duration::from_secs(3600)));
        assert_eq!(parse_rate_limit("10/min").unwrap(), (10, Duration::from_secs(60)));
        assert!(parse_rate_limit("3/century").is_err());
        assert!(parse_rate_limit("nope").is_err());
    }

    #[test]
    fn profile_scoping() {
        let mut p = pol(&["*"], &[]);
        p.profiles = Some(vec![1, 2, 3]);
        assert!(p.check_profile(1));
        assert!(!p.check_profile(99));
        p.profiles = None;
        assert!(p.check_profile(99));
    }
}
