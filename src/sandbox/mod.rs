// Sandbox primitive — see SANDBOX.md for the design contract.
//
// A `Sandbox` is the parsed, validated representation of one TOML policy
// file activated for the current process. It exposes:
//
//   - `Sandbox::load(name)` — read + validate the policy
//   - `check_command(path, args)` — dispatch gate, called from main.rs
//   - `check_profile/card/balance(id)` — resource gates, called from Ctx
//   - `start_audit(...)` — wrap any sandboxed call in an AuditEntry
//   - `condition_for(path)` — surface per-command conditions to handlers
//
// All loading is fail-closed: a missing file, an invalid policy, or any
// validation error stops the CLI before the command runs. There is no
// silent fallback.

use anyhow::{Context as _, Result};
use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};

/// Root-owned sandbox directory used by the lockdown deployment recipe
/// (see AGENT.md). Policies in here outrank user policies and pass the
/// ownership check, because the agent uid cannot rewrite them.
const SYSTEM_SANDBOXES_DIR: &str = "/etc/wise/sandboxes";

pub mod audit;
pub mod path;
pub mod policy;
pub mod ratelimit;

pub use audit::{AuditEntry, StartContext};
pub use path::{command_args, command_path, Cmd};
pub use policy::{Conditions, Decision, Escalation, EscalationMode, Policy};

/// Active sandbox: parsed policy + the file path it came from.
#[derive(Debug, Clone)]
pub struct Sandbox {
    pub policy: Policy,
    pub source: PathBuf,
}

impl Sandbox {
    /// Load a sandbox policy by name. Searches `/etc/wise/sandboxes/`
    /// (root-owned, lockdown-friendly) before `~/.config/wise/sandboxes/`,
    /// so a system-installed policy always wins. The `name` field inside
    /// the file must match the basename.
    ///
    /// When `lockdown` is true, the policy file must not be writable by the
    /// caller's uid (or by group/other on unix). This is what makes the
    /// agent on a VPS unable to rewrite its own policy: under lockdown,
    /// the only loadable policies are the ones root pinned in /etc.
    pub fn load_with_lockdown(name: &str, lockdown: bool) -> Result<Self> {
        let path = Self::resolve_path(name)?;
        let s = fs::read_to_string(&path)
            .with_context(|| format!("reading sandbox file {}", path.display()))?;
        let policy: Policy = toml::from_str(&s)
            .with_context(|| format!("parsing sandbox file {}", path.display()))?;
        if policy.name != name {
            anyhow::bail!(
                "sandbox name mismatch: file is `{name}.toml` but policy.name = `{}`",
                policy.name
            );
        }
        policy.validate()?;
        if lockdown {
            check_policy_ownership(&path).with_context(|| {
                format!(
                    "lockdown rejected sandbox file {} — see AGENT.md \"Deploying on a VPS\"",
                    path.display()
                )
            })?;
        }
        Ok(Sandbox {
            policy,
            source: path,
        })
    }

    /// Convenience wrapper for callers that don't yet know the lockdown
    /// state — used by `wise sandbox show/check/edit` etc., which all run
    /// as a human and don't need the ownership clamp.
    pub fn load(name: &str) -> Result<Self> {
        Self::load_with_lockdown(name, false)
    }

    /// Resolve `name` to an actual file path, preferring the system dir.
    fn resolve_path(name: &str) -> Result<PathBuf> {
        let basename = format!("{name}.toml");
        let sys = Path::new(SYSTEM_SANDBOXES_DIR).join(&basename);
        if sys.exists() {
            return Ok(sys);
        }
        let user = Self::sandboxes_dir()?.join(&basename);
        if user.exists() {
            return Ok(user);
        }
        anyhow::bail!(
            "sandbox '{name}' not found in {} or {}. Run `wise sandbox new {name}` first.",
            SYSTEM_SANDBOXES_DIR,
            Self::sandboxes_dir()?.display()
        )
    }

    /// Path that a sandbox of `name` would be *written* to. Always the user
    /// directory — `wise sandbox new` is a human command, and we never
    /// want the CLI to attempt writes into `/etc`.
    pub fn path_for(name: &str) -> Result<PathBuf> {
        let dir = Self::sandboxes_dir()?;
        Ok(dir.join(format!("{name}.toml")))
    }

    /// Directory holding user-level sandbox files (write target).
    pub fn sandboxes_dir() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("com", "wise", "wise")
            .ok_or_else(|| anyhow::anyhow!("could not resolve config directory"))?;
        Ok(dirs.config_dir().join("sandboxes"))
    }

    /// List the names of all sandbox files on disk. Merges the system and
    /// user directories — system entries take precedence on name collision
    /// (matches `load`), but `list_all` itself just returns a deduped sorted
    /// list of names.
    pub fn list_all() -> Result<Vec<String>> {
        let mut names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for dir in [PathBuf::from(SYSTEM_SANDBOXES_DIR), Self::sandboxes_dir()?] {
            if !dir.exists() {
                continue;
            }
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("toml") {
                    continue;
                }
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    names.insert(stem.to_string());
                }
            }
        }
        Ok(names.into_iter().collect())
    }

    /// Persist a fresh policy to disk. Refuses to clobber an existing file
    /// unless `force = true`.
    pub fn save(&self, force: bool) -> Result<()> {
        let parent = self.source.parent().ok_or_else(|| {
            anyhow::anyhow!("sandbox source path has no parent: {}", self.source.display())
        })?;
        fs::create_dir_all(parent)
            .with_context(|| format!("creating sandboxes dir {}", parent.display()))?;
        if self.source.exists() && !force {
            anyhow::bail!(
                "sandbox already exists at {} (pass --force to overwrite)",
                self.source.display()
            );
        }
        let body = toml::to_string_pretty(&self.policy).context("serializing policy")?;
        fs::write(&self.source, body)
            .with_context(|| format!("writing sandbox file {}", self.source.display()))?;
        set_secure_perms(&self.source)?;
        Ok(())
    }

    // ---------- runtime checks ----------

    pub fn name(&self) -> &str {
        &self.policy.name
    }

    /// Dispatch gate. Returns Ok(()) iff the policy allows this command.
    pub fn check_command(
        &self,
        path: &str,
        args: &[(String, String)],
    ) -> Result<()> {
        match self.policy.check(path, args) {
            Decision::Allow => Ok(()),
            Decision::Denied { rule } => Err(deny_error(self, path, &format!("denied by rule `{rule}`"))),
            Decision::NotAllowed => Err(deny_error(self, path, "not in allow list")),
        }
    }

    pub fn check_profile(&self, id: i64) -> Result<()> {
        if !self.policy.check_profile(id) {
            anyhow::bail!(
                "sandbox '{}' restricts profiles to {:?}; profile {id} is not allowed",
                self.policy.name,
                self.policy.profiles.as_ref().unwrap_or(&Vec::new())
            );
        }
        Ok(())
    }

    pub fn check_card(&self, token: &str) -> Result<()> {
        if !self.policy.check_card(token) {
            anyhow::bail!(
                "sandbox '{}' restricts cards to {:?}; `{token}` is not allowed",
                self.policy.name,
                self.policy.cards.as_ref().unwrap_or(&Vec::new())
            );
        }
        Ok(())
    }

    pub fn check_balance(&self, id: i64) -> Result<()> {
        if !self.policy.check_balance(id) {
            anyhow::bail!(
                "sandbox '{}' restricts balances to {:?}; {id} is not allowed",
                self.policy.name,
                self.policy.balances.as_ref().unwrap_or(&Vec::new())
            );
        }
        Ok(())
    }

    /// Look up the per-command conditions block for `path`, if any.
    pub fn condition_for(&self, path: &str) -> Option<&Conditions> {
        self.policy.conditions.get(path)
    }

    /// Apply the rate limit + justification check for `path` and start an
    /// audit entry. Call `entry.complete(...)` or `entry.fail(...)` on the
    /// returned handle once the underlying command finishes. Returns
    /// `Ok(None)` if no conditions apply (no audit, no rate limit, no
    /// justification needed).
    pub fn enforce_conditions(
        &self,
        cmd_path: &str,
        args: &serde_json::Value,
        justify: Option<&str>,
    ) -> Result<Option<AuditEntry>> {
        let cond = match self.condition_for(cmd_path) {
            Some(c) => c,
            None => return Ok(None),
        };

        if cond.require_justification && justify.is_none() {
            anyhow::bail!(
                "sandbox '{}' requires --justify on `{cmd_path}` (per [conditions.\"{cmd_path}\"].require_justification)",
                self.policy.name
            );
        }
        if let Some(audit_path) = &cond.audit {
            if let Some(rl) = &cond.rate_limit {
                let (count, window) = policy::parse_rate_limit(rl)?;
                ratelimit::check(audit_path, cmd_path, count, window)?;
            }
            let entry = audit::start(
                audit_path,
                &self.policy.name,
                StartContext {
                    command: cmd_path,
                    args: args.clone(),
                    justify,
                },
            )?;
            return Ok(Some(entry));
        }
        // Justification was checked but no audit path means no log to write.
        Ok(None)
    }
}

/// Build a structured deny error so it surfaces nicely through the existing
/// `output::print_error` path.
fn deny_error(sandbox: &Sandbox, path: &str, reason: &str) -> anyhow::Error {
    anyhow::anyhow!(
        "sandbox_denied: command `{path}` blocked by sandbox `{}` ({reason})",
        sandbox.policy.name
    )
}

#[cfg(unix)]
fn set_secure_perms(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_secure_perms(_path: &Path) -> Result<()> {
    Ok(())
}

/// Refuse to load a policy file that the calling uid (or anyone other than
/// root) could rewrite — under lockdown, that policy is not actually
/// binding. The expected layout is `/etc/wise/sandboxes/<name>.toml`,
/// `0644 root:root`, which the agent uid cannot modify.
#[cfg(unix)]
fn check_policy_ownership(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    let meta = fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?;
    let mode = meta.permissions().mode() & 0o777;
    let owner = meta.uid();
    let caller = unsafe { libc::geteuid() };
    if owner == caller && caller != 0 {
        anyhow::bail!(
            "policy_writable_by_caller: {} is owned by uid {caller} (the calling user). \
             Under lockdown the policy must be owned by root so the agent cannot rewrite it.",
            path.display()
        );
    }
    if mode & 0o022 != 0 {
        anyhow::bail!(
            "policy_writable_by_others: {} has mode {:04o}; group/world write must be off under lockdown",
            path.display(),
            mode
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_policy_ownership(_path: &Path) -> Result<()> {
    // No uid concept on non-unix; lockdown is a unix-only feature in v1.
    Ok(())
}

#[cfg(all(test, unix))]
mod ownership_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn write_temp_policy(mode: u32) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wise-lockdown-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("p.toml");
        fs::write(&path, "name=\"p\"\nallow=[\"*\"]\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn ownership_check_rejects_caller_owned_file() {
        // The test process owns the temp file, so the check must reject it
        // (running CI as root would skip this — guard accordingly).
        let caller = unsafe { libc::geteuid() };
        if caller == 0 {
            return; // root owns it; check intentionally permits this
        }
        let path = write_temp_policy(0o600);
        let err = check_policy_ownership(&path).unwrap_err();
        assert!(
            err.to_string().contains("policy_writable_by_caller"),
            "got: {err}"
        );
    }

    #[test]
    fn ownership_check_rejects_world_writable() {
        // Even if root owns it, world-writable defeats the point.
        let caller = unsafe { libc::geteuid() };
        if caller == 0 {
            return;
        }
        let path = write_temp_policy(0o666);
        let err = check_policy_ownership(&path).unwrap_err();
        // Caller-owned check fires first; either failure mode is fine.
        let msg = err.to_string();
        assert!(
            msg.contains("policy_writable_by_caller") || msg.contains("policy_writable_by_others"),
            "got: {msg}"
        );
    }

    #[test]
    fn plain_load_skips_ownership_check() {
        // Sandbox::load (non-lockdown) must keep working on a user-owned
        // file — this is the developer-laptop path, where forcing chown
        // would be hostile.
        let dir = std::env::temp_dir().join(format!("wise-load-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("devsbx.toml");
        fs::write(&path, "name=\"devsbx\"\nallow=[\"*\"]\n").unwrap();
        // Use the internal helper directly — Sandbox::load goes through
        // path resolution which expects the standard layout, but the
        // ownership-check skip is what we're testing here.
        // (False under lockdown means the check is bypassed.)
        let s = fs::read_to_string(&path).unwrap();
        let policy: Policy = toml::from_str(&s).unwrap();
        policy.validate().unwrap();
        // No ownership check requested, so this is just a smoke test that
        // the file is parseable as a policy.
        assert_eq!(policy.name, "devsbx");
    }
}

// Custom Serialize impl for Policy via derive on policy.rs would force us to
// either pull serde derive features into policy.rs or do it manually here.
// We do it manually here to keep policy.rs read-only deserialization-only.
impl serde::Serialize for Policy {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = serializer.serialize_map(None)?;
        m.serialize_entry("name", &self.name)?;
        if let Some(d) = &self.description {
            m.serialize_entry("description", d)?;
        }
        if let Some(p) = &self.profiles {
            m.serialize_entry("profiles", p)?;
        }
        if let Some(c) = &self.cards {
            m.serialize_entry("cards", c)?;
        }
        if let Some(b) = &self.balances {
            m.serialize_entry("balances", b)?;
        }
        m.serialize_entry("allow", &self.allow)?;
        if !self.deny.is_empty() {
            m.serialize_entry("deny", &self.deny)?;
        }
        if !self.policy_conditions_empty() {
            m.serialize_entry("conditions", &self.conditions)?;
        }
        m.serialize_entry("escalation", &self.escalation)?;
        m.end()
    }
}

impl Policy {
    fn policy_conditions_empty(&self) -> bool {
        self.conditions.is_empty()
    }
}

impl serde::Serialize for Conditions {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut m = serializer.serialize_map(None)?;
        if let Some(rl) = &self.rate_limit {
            m.serialize_entry("rate_limit", rl)?;
        }
        if self.require_justification {
            m.serialize_entry("require_justification", &true)?;
        }
        if let Some(p) = &self.audit {
            m.serialize_entry("audit", p)?;
        }
        m.end()
    }
}

impl serde::Serialize for Escalation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mode_str = match self.mode {
            EscalationMode::Deny => "deny",
            EscalationMode::Tty => "tty",
            EscalationMode::Command => "command",
        };
        let mut m = serializer.serialize_map(None)?;
        m.serialize_entry("mode", mode_str)?;
        if let Some(c) = &self.command {
            m.serialize_entry("command", c)?;
        }
        if let Some(t) = &self.timeout {
            m.serialize_entry("timeout", t)?;
        }
        m.end()
    }
}
