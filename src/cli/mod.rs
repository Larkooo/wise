// Shared CLI types: GlobalArgs (top-level flags) and Ctx (runtime context
// passed to every subcommand). Each subcommand module is declared here.

use anyhow::{Context as _, Result};
use clap::Args;

use crate::client::WiseClient;
use crate::config::{Config, Env};
use crate::output::OutputFormat;

pub mod activity;
pub mod auth;
pub mod balance;
pub mod card;
pub mod card_order;
pub mod config_cmd;
pub mod currency;
pub mod docs;
pub mod profile;
pub mod quote;
pub mod rate;
pub mod recipient;
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
}

impl Ctx {
    pub async fn new(args: GlobalArgs) -> Result<Self> {
        let config = Config::load().context("loading config")?;
        let env = args
            .env
            .or(config.env)
            .unwrap_or(Env::Sandbox);

        // Token may be unset for `wise auth login` and `wise docs ask` —
        // construct the client anyway and let auth-required calls fail at
        // request time.
        let token = if let Some(t) = args.token.clone() {
            Some(t)
        } else {
            crate::config::load_token(env).ok()
        };

        let client = WiseClient::new(env, token)?;
        Ok(Self { args, config, client })
    }

    /// Returns the profile id from --profile, env, config default, or errors.
    pub fn require_profile(&self) -> Result<i64> {
        self.args
            .profile
            .or(self.config.default_profile)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no profile id — pass --profile <id>, set WISE_PROFILE, or run \
                     `wise config set default-profile <id>`"
                )
            })
    }

    /// Returns the optional profile id (None is fine).
    pub fn profile_or_default(&self) -> Option<i64> {
        self.args.profile.or(self.config.default_profile)
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
