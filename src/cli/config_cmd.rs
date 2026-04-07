// CLI subcommand for reading/writing the persistent config TOML.

use anyhow::{Context as _, Result};
use clap::Subcommand;
use serde_json::json;

use crate::cli::Ctx;
use crate::config::{Config, Env};
use crate::output;

#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    /// Print one config value.
    Get { key: String },
    /// Set one config value.
    Set { key: String, value: String },
    /// Print the full config + path.
    List,
    /// Print the config file path.
    Path,
}

pub async fn run(cmd: ConfigCmd, ctx: &Ctx) -> Result<()> {
    let mut cfg = Config::load()?;
    match cmd {
        ConfigCmd::Get { key } => {
            let value = match key.as_str() {
                "env" => json!(cfg.env.map(|e| e.as_str())),
                "default-profile" | "default_profile" => json!(cfg.default_profile),
                other => anyhow::bail!("unknown key '{other}'. valid: env, default-profile"),
            };
            output::print(&value, ctx.output());
        }
        ConfigCmd::Set { key, value } => {
            match key.as_str() {
                "env" => {
                    let env = match value.as_str() {
                        "sandbox" => Env::Sandbox,
                        "production" | "prod" => Env::Production,
                        other => anyhow::bail!("invalid env '{other}', expected sandbox|production"),
                    };
                    cfg.env = Some(env);
                }
                "default-profile" | "default_profile" => {
                    let id: i64 = value.parse().context("default-profile must be an integer")?;
                    cfg.default_profile = Some(id);
                }
                other => anyhow::bail!("unknown key '{other}'"),
            }
            cfg.save()?;
            output::print(&json!({ "saved": true, "key": key, "value": value }), ctx.output());
        }
        ConfigCmd::List => {
            let path = Config::path()?;
            output::print(
                &json!({
                    "path": path,
                    "env": cfg.env.map(|e| e.as_str()),
                    "default_profile": cfg.default_profile,
                }),
                ctx.output(),
            );
        }
        ConfigCmd::Path => {
            let path = Config::path()?;
            output::print(&json!({ "path": path }), ctx.output());
        }
    }
    Ok(())
}
