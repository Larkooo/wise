// Shared CLI types: GlobalArgs (top-level flags) and Ctx (runtime context
// passed to every subcommand). Each subcommand module is declared here.

use anyhow::{Context as _, Result};
use clap::Args;

use crate::client::WiseClient;
use crate::config::{Config, Env};
use crate::output::OutputFormat;
use crate::sandbox::Sandbox;

pub mod activity;
pub mod agent;
pub mod auth;
pub mod balance;
pub mod card;
pub mod card_order;
pub mod config_cmd;
pub mod currency;
pub mod docs;
pub mod jose;
pub mod profile;
pub mod quote;
pub mod rate;
pub mod recipient;
pub mod sandbox;
pub mod simulate;
pub mod transfer;
pub mod webhook;

#[derive(Debug, Clone, Args)]
pub struct GlobalArgs {
    /// API environment to target.
    #[arg(long, value_enum, env = "WISE_ENV", global = true)]
    pub env: Option<Env>,

    /// API token to use for this invocation (overrides stored credentials).
    #[arg(long, env = "WISE_API_TOKEN", global = true, hide_env_values = true)]
    pub token: Option<String>,

    /// Default profile id to use when a command needs one.
    #[arg(long, env = "WISE_PROFILE", global = true)]
    pub profile: Option<i64>,

    /// Activate a sandbox policy by name (loaded from
    /// ~/.config/wise/sandboxes/<name>.toml).
    #[arg(long, env = "WISE_SANDBOX", global = true)]
    pub sandbox: Option<String>,

    /// Justification string for sandbox-audited commands.
    #[arg(long, global = true)]
    pub justify: Option<String>,

    /// Pretty-print JSON output.
    #[arg(long, global = true)]
    pub pretty: bool,

    /// Render output as a table where supported (humans only).
    #[arg(long, global = true, conflicts_with = "pretty")]
    pub table: bool,

    /// Skip confirmation prompts (required for production money operations).
    #[arg(long, short = 'y', env = "WISE_YES", global = true)]
    pub yes: bool,

    /// Verbose logging to stderr.
    #[arg(long, short = 'v', global = true)]
    pub verbose: bool,
}

impl GlobalArgs {
    pub fn output_format(&self) -> OutputFormat {
        if self.table {
            OutputFormat::Table
        } else if self.pretty {
            OutputFormat::Pretty
        } else {
            OutputFormat::Json
        }
    }
}

/// Runtime context handed to every subcommand.
pub struct Ctx {
    pub args: GlobalArgs,
    pub config: Config,
    pub client: WiseClient,
    pub sandbox: Option<Sandbox>,
}

impl Ctx {
    pub async fn new(args: GlobalArgs) -> Result<Self> {
        let config = Config::load().context("loading config")?;
        let env = args.env.or(config.env).unwrap_or(Env::Sandbox);

        // Token may be unset for `wise auth login` and `wise docs ask` —
        // construct the client anyway and let auth-required calls fail at
        // request time.
        let token = if let Some(t) = args.token.clone() {
            Some(t)
        } else {
            crate::config::load_token(env).ok()
        };

        // Sandbox loading is fail-closed: a missing or invalid file aborts
        // the CLI before any command runs. We deliberately do this here
        // (not in dispatch) so even commands that don't go through the
        // dispatcher (like `--help` resolution) see the same error path.
        //
        // When `require_sandbox = true` (lockdown mode, see AGENT.md), we
        // hand `load_with_lockdown` so the policy file's ownership is
        // verified — anything writable by the calling uid is rejected.
        let sandbox = if let Some(name) = args.sandbox.clone() {
            Some(
                Sandbox::load_with_lockdown(&name, config.require_sandbox)
                    .context("loading active sandbox")?,
            )
        } else {
            None
        };

        // Lockdown gate: when `/etc/wise/config.toml` sets `require_sandbox
        // = true`, every command must run inside an active sandbox. This
        // closes the "agent unsets WISE_SANDBOX and runs wise transfer
        // create" bypass on a properly-isolated VPS deployment, because
        // the agent uid cannot rewrite /etc/wise/config.toml.
        if config.require_sandbox && sandbox.is_none() {
            anyhow::bail!(
                "lockdown_active: this installation requires an active sandbox \
                 (set WISE_SANDBOX=<name> or pass --sandbox <name>). See AGENT.md \
                 \"Deploying on a VPS\" for the deployment recipe."
            );
        }

        let client = WiseClient::new(env, token)?;
        Ok(Self {
            args,
            config,
            client,
            sandbox,
        })
    }

    /// Resolve a profile id from a per-command override (highest priority),
    /// then `--profile` global, then config default. Always runs the sandbox
    /// resource gate on the result, regardless of which source provided the
    /// id — this is critical, otherwise an explicit per-command --profile
    /// would bypass the sandbox's profile allow-list.
    pub fn resolve_profile(&self, override_id: Option<i64>) -> Result<i64> {
        let id = override_id
            .or(self.args.profile)
            .or(self.config.default_profile)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no profile id — pass --profile <id>, set WISE_PROFILE, or run \
                     `wise config set default-profile <id>`"
                )
            })?;
        if let Some(sb) = &self.sandbox {
            sb.check_profile(id)?;
        }
        Ok(id)
    }

    /// Returns the optional profile id (None is fine). Does NOT apply the
    /// sandbox gate — only used by `wise profile current` for introspection.
    pub fn profile_or_default(&self) -> Option<i64> {
        self.args.profile.or(self.config.default_profile)
    }

    /// Resource gate for card tokens. Handlers should call this on every
    /// card token they're about to operate on.
    pub fn check_card(&self, token: &str) -> Result<()> {
        if let Some(sb) = &self.sandbox {
            sb.check_card(token)?;
        }
        Ok(())
    }

    /// Resource gate for balance ids.
    pub fn check_balance(&self, id: i64) -> Result<()> {
        if let Some(sb) = &self.sandbox {
            sb.check_balance(id)?;
        }
        Ok(())
    }

    pub fn output(&self) -> OutputFormat {
        self.args.output_format()
    }

    /// Refuse to proceed in production unless --yes was passed.
    pub fn confirm_prod(&self, action: &str) -> Result<()> {
        if self.client.env() == Env::Production && !self.args.yes {
            anyhow::bail!(
                "refusing to {action} in production without --yes (or WISE_YES=1). \
                 Re-run with --yes to confirm."
            );
        }
        Ok(())
    }
}
