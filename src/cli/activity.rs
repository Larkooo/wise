// Activity subcommand. GET /v1/profiles/{p}/activities

use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;

use crate::cli::Ctx;
use crate::output;

#[derive(Debug, Subcommand)]
pub enum ActivityCmd {
    /// List activities for a profile.
    List {
        #[arg(long)]
        profile: Option<i64>,
        #[arg(long, name = "monetary-resource-type")]
        monetary_resource_type: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long, name = "since")]
        since: Option<String>,
        #[arg(long, name = "until")]
        until: Option<String>,
        #[arg(long, default_value = "20")]
        size: u32,
    },
}

pub async fn run(cmd: ActivityCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        ActivityCmd::List {
            profile,
            monetary_resource_type,
            status,
            since,
            until,
            size,
        } => {
            let p = profile.or(ctx.config.default_profile).ok_or_else(|| {
                anyhow::anyhow!("--profile required (or set default-profile)")
            })?;
            let mut q: Vec<(String, String)> = vec![("size".into(), size.to_string())];
            if let Some(m) = monetary_resource_type {
                q.push(("monetaryResourceType".into(), m));
            }
            if let Some(s) = status {
                q.push(("status".into(), s));
            }
            if let Some(s) = since {
                q.push(("since".into(), s));
            }
            if let Some(u) = until {
                q.push(("until".into(), u));
            }
            let v: Value = ctx
                .client
                .get_query(&format!("/v1/profiles/{p}/activities"), &q)
                .await?;
            output::print(&v, ctx.output());
        }
    }
    Ok(())
}
