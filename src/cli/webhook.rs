// Webhook subcommands. Wise has two scopes:
//   - Profile-level: /v3/profiles/{p}/subscriptions
//   - Application-level: /v3/applications/{clientKey}/subscriptions
//
// Use --application <clientKey> to target the app scope, otherwise the
// profile scope is used (with --profile or the default profile).

use anyhow::{Context as _, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::cli::Ctx;
use crate::output;

#[derive(Debug, Subcommand)]
pub enum WebhookCmd {
    /// List subscriptions.
    List {
        #[arg(long)]
        application: Option<String>,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Get one subscription.
    Get {
        subscription_id: String,
        #[arg(long)]
        application: Option<String>,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Create a webhook subscription.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        url: String,
        /// Trigger event, e.g. `transfers#state-change` or `balances#credit`.
        #[arg(long)]
        trigger: String,
        #[arg(long, default_value = "2.0.0")]
        version: String,
        #[arg(long)]
        mtls: bool,
        #[arg(long)]
        application: Option<String>,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Delete a subscription.
    Delete {
        subscription_id: String,
        #[arg(long)]
        application: Option<String>,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Send a test notification (application subscriptions only).
    Test {
        subscription_id: String,
        #[arg(long)]
        application: String,
    },
}

pub async fn run(cmd: WebhookCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        WebhookCmd::List { application, profile } => {
            let path = subs_path(ctx, application.as_deref(), profile)?;
            let v: Value = ctx.client.get(&path).await?;
            output::print(&v, ctx.output());
        }
        WebhookCmd::Get {
            subscription_id,
            application,
            profile,
        } => {
            let base = subs_path(ctx, application.as_deref(), profile)?;
            let v: Value = ctx.client.get(&format!("{base}/{subscription_id}")).await?;
            output::print(&v, ctx.output());
        }
        WebhookCmd::Create {
            name,
            url,
            trigger,
            version,
            mtls,
            application,
            profile,
        } => {
            ctx.confirm_prod("create a webhook subscription")?;
            let base = subs_path(ctx, application.as_deref(), profile)?;
            let body = json!({
                "name": name,
                "trigger_on": trigger,
                "delivery": {
                    "version": version,
                    "url": url,
                    "mtls_enabled": mtls,
                },
                "enabled": true,
            });
            let v: Value = ctx.client.post(&base, &body).await?;
            output::print(&v, ctx.output());
        }
        WebhookCmd::Delete {
            subscription_id,
            application,
            profile,
        } => {
            ctx.confirm_prod("delete a webhook subscription")?;
            let base = subs_path(ctx, application.as_deref(), profile)?;
            let v = ctx.client.delete(&format!("{base}/{subscription_id}")).await?;
            output::print(&v, ctx.output());
        }
        WebhookCmd::Test {
            subscription_id,
            application,
        } => {
            let path = format!(
                "/v3/applications/{application}/subscriptions/{subscription_id}/test-notifications"
            );
            let v: Value = ctx.client.post(&path, &json!({})).await?;
            output::print(&v, ctx.output());
        }
    }
    Ok(())
}

fn subs_path(ctx: &Ctx, application: Option<&str>, profile: Option<i64>) -> Result<String> {
    if let Some(app) = application {
        Ok(format!("/v3/applications/{app}/subscriptions"))
    } else {
        let p = profile
            .or(ctx.config.default_profile)
            .context("--profile or --application required")?;
        Ok(format!("/v3/profiles/{p}/subscriptions"))
    }
}
