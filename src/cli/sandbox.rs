// `wise sandbox …` — manage sandbox policy files.
//
// These commands are explicitly not callable from inside an active sandbox
// (the dispatch gate in main.rs::dispatch refuses `sandbox.*` paths when a
// sandbox is loaded). That guarantees an agent cannot rewrite its own
// policy or list other policies.

use anyhow::{Context as _, Result};
use clap::Subcommand;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::cli::Ctx;
use crate::output;
use crate::sandbox::{
    policy::{Conditions, Decision, Escalation, EscalationMode, Policy},
    Sandbox,
};

#[derive(Debug, Subcommand)]
pub enum SandboxCmd {
    /// Scaffold a new sandbox file. Defaults are deny-by-default with a
    /// minimal allow list — you'll likely want to edit it afterwards.
    New {
        name: String,
        /// Restrict commands to these profile IDs.
        #[arg(long)]
        profile: Vec<i64>,
        /// Restrict to these card tokens.
        #[arg(long)]
        card: Vec<String>,
        /// Restrict to these balance IDs.
        #[arg(long)]
        balance: Vec<i64>,
        /// Glob patterns to add to the allow list. Repeat for multiple.
        #[arg(long, default_value = "*.list,*.get,rate.*,docs.ask")]
        allow: String,
        /// Free-form description, surfaced in `wise sandbox show`.
        #[arg(long)]
        description: Option<String>,
        /// Overwrite an existing file with the same name.
        #[arg(long)]
        force: bool,
    },
    /// List all sandbox files on disk.
    List,
    /// Print the parsed policy for a sandbox (JSON).
    Show { name: String },
    /// Open the sandbox file in $EDITOR.
    Edit { name: String },
    /// Delete a sandbox file.
    Delete {
        name: String,
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Dry-run: would `<cmd-path>` be allowed under `<sandbox>`?
    Check {
        name: String,
        cmd_path: String,
    },
    /// Spawn $SHELL with WISE_SANDBOX=<name> set in the environment.
    Shell {
        name: String,
        /// Shell binary to exec. Defaults to $SHELL or /bin/sh.
        #[arg(long)]
        shell: Option<String>,
    },
    /// Tail the audit log for a sandbox (if one is configured).
    Audit {
        name: String,
        /// How many lines to print from the tail.
        #[arg(long, short = 'n', default_value_t = 20)]
        lines: usize,
    },
}

pub async fn run(cmd: SandboxCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        SandboxCmd::New {
            name,
            profile,
            card,
            balance,
            allow,
            description,
            force,
        } => {
            let allow_list: Vec<String> =
                allow.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            let policy = Policy {
                name: name.clone(),
                description,
                profiles: if profile.is_empty() { None } else { Some(profile) },
                cards: if card.is_empty() { None } else { Some(card) },
                balances: if balance.is_empty() { None } else { Some(balance) },
                allow: allow_list,
                deny: Vec::new(),
                conditions: HashMap::new(),
                escalation: Escalation {
                    mode: EscalationMode::Deny,
                    command: None,
                    timeout: None,
                },
            };
            policy.validate()?;
            let sb = Sandbox {
                policy,
                source: Sandbox::path_for(&name)?,
            };
            sb.save(force)?;
            output::print(
                &json!({
                    "created": true,
                    "name": name,
                    "path": sb.source,
                }),
                ctx.output(),
            );
        }

        SandboxCmd::List => {
            let names = Sandbox::list_all()?;
            let dir = Sandbox::sandboxes_dir()?;
            output::print(
                &json!({
                    "dir": dir,
                    "sandboxes": names,
                }),
                ctx.output(),
            );
        }

        SandboxCmd::Show { name } => {
            let sb = Sandbox::load(&name)?;
            // Round-trip through serde so the output matches the on-disk
            // representation rather than Rust's Debug shape.
            let value: Value = serde_json::to_value(&sb.policy)?;
            output::print(&value, ctx.output());
        }

        SandboxCmd::Edit { name } => {
            let path = Sandbox::path_for(&name)?;
            if !path.exists() {
                anyhow::bail!(
                    "sandbox '{name}' not found at {}; create it with `wise sandbox new`",
                    path.display()
                );
            }
            let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            let status = std::process::Command::new(&editor).arg(&path).status()?;
            if !status.success() {
                anyhow::bail!("editor `{editor}` exited with status {status}");
            }
            // Re-validate after edit so we catch typos immediately.
            let _ = Sandbox::load(&name).context("validating edited sandbox")?;
            output::print(&json!({ "edited": true, "path": path }), ctx.output());
        }

        SandboxCmd::Delete { name, yes } => {
            let path = Sandbox::path_for(&name)?;
            if !path.exists() {
                anyhow::bail!("sandbox '{name}' not found at {}", path.display());
            }
            if !yes {
                anyhow::bail!(
                    "refusing to delete sandbox '{name}' without --yes (or -y)"
                );
            }
            fs::remove_file(&path)
                .with_context(|| format!("deleting {}", path.display()))?;
            output::print(&json!({ "deleted": true, "path": path }), ctx.output());
        }

        SandboxCmd::Check { name, cmd_path } => {
            let sb = Sandbox::load(&name)?;
            let decision = sb.policy.check(&cmd_path, &[]);
            let allowed = matches!(decision, Decision::Allow);
            let detail = match &decision {
                Decision::Allow => json!("allow"),
                Decision::NotAllowed => json!("not in allow list"),
                Decision::Denied { rule } => json!({ "denied_by_rule": rule }),
            };
            output::print(
                &json!({
                    "sandbox": name,
                    "command": cmd_path,
                    "allowed": allowed,
                    "decision": detail,
                }),
                ctx.output(),
            );
        }

        SandboxCmd::Shell { name, shell } => {
            // Validate the sandbox loads cleanly before exec'ing the shell —
            // otherwise the user enters a "broken" environment they have to
            // unset to escape.
            let _ = Sandbox::load(&name)?;
            let shell_bin = shell
                .or_else(|| std::env::var("SHELL").ok())
                .unwrap_or_else(|| "/bin/sh".to_string());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                let err = std::process::Command::new(&shell_bin)
                    .env("WISE_SANDBOX", &name)
                    .env("WISE_SANDBOX_ACTIVE", "1")
                    .exec();
                // exec only returns on failure.
                anyhow::bail!("failed to exec {shell_bin}: {err}");
            }
            #[cfg(not(unix))]
            {
                let status = std::process::Command::new(&shell_bin)
                    .env("WISE_SANDBOX", &name)
                    .env("WISE_SANDBOX_ACTIVE", "1")
                    .status()?;
                std::process::exit(status.code().unwrap_or(1));
            }
        }

        SandboxCmd::Audit { name, lines } => {
            let sb = Sandbox::load(&name)?;
            // Find an audit path: first per-command condition that has one,
            // or fall back to a default in the sandboxes dir.
            let mut audit_path: Option<PathBuf> = None;
            for c in sb.policy.conditions.values() {
                if let Some(p) = &c.audit {
                    audit_path = Some(p.clone());
                    break;
                }
            }
            let audit_path = audit_path.unwrap_or_else(|| {
                Sandbox::sandboxes_dir()
                    .map(|d| d.join(format!("{name}.audit.jsonl")))
                    .unwrap_or_else(|_| PathBuf::from(format!("{name}.audit.jsonl")))
            });
            if !audit_path.exists() {
                output::print(
                    &json!({
                        "path": audit_path,
                        "lines": [],
                        "message": "no audit log yet",
                    }),
                    ctx.output(),
                );
                return Ok(());
            }
            let body = fs::read_to_string(&audit_path)?;
            let all: Vec<&str> = body.lines().collect();
            let take = all.len().saturating_sub(lines);
            let tail: Vec<Value> = all[take..]
                .iter()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            output::print(
                &json!({
                    "path": audit_path,
                    "lines": tail,
                }),
                ctx.output(),
            );
        }
    }
    // `Conditions` reference is unused outside of policy/audit; silence the
    // import linter without exposing the type publicly here.
    let _ = std::marker::PhantomData::<Conditions>;
    Ok(())
}
