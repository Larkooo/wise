// Profile subcommands. A profile is the entity (personal or business) that
// owns balances, recipients, transfers, etc. Most other commands need a
// profile id; `wise profile current` is the agent's first stop.

use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;

use crate::cli::Ctx;
use crate::output;

#[derive(Debug, Subcommand)]
pub enum ProfileCmd {
    /// List all profiles for the authenticated user (GET /v2/profiles).
    List,
    /// Get a single profile by id (GET /v2/profiles/{id}).
    Get { profile_id: i64 },
    /// Print the default profile resolved from --profile / WISE_PROFILE / config.
    Current,
}

pub async fn run(cmd: ProfileCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        ProfileCmd::List => {
            let v: Value = ctx.client.get("/v2/profiles").await?;
            output::print(&v, ctx.output());
        }
        ProfileCmd::Get { profile_id } => {
            let v: Value = ctx.client.get(&format!("/v2/profiles/{profile_id}")).await?;
            output::print(&v, ctx.output());
        }
        ProfileCmd::Current => {
            let id = ctx.profile_or_default();
            output::print(
                &serde_json::json!({
                    "default_profile": id,
                    "source": if ctx.args.profile.is_some() {
                        "flag"
                    } else if ctx.config.default_profile.is_some() {
                        "config"
                    } else {
                        "unset"
                    },
                }),
                ctx.output(),
            );
        }
    }
    Ok(())
}
