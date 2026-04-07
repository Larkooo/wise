// Quote subcommands.
//
// Endpoints:
//   POST  /v3/quotes                              (unauthenticated example)
//   POST  /v3/profiles/{p}/quotes                 (authenticated)
//   GET   /v3/profiles/{p}/quotes/{id}
//   PATCH /v3/profiles/{p}/quotes/{id}            (set targetAccount, etc)

use anyhow::{bail, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::cli::Ctx;
use crate::output;

#[derive(Debug, Subcommand)]
pub enum QuoteCmd {
    /// Create an authenticated quote (POST /v3/profiles/{p}/quotes).
    Create {
        #[arg(long)]
        source: String,
        #[arg(long)]
        target: String,
        /// Source amount (mutually exclusive with --target-amount).
        #[arg(long, conflicts_with = "target_amount")]
        source_amount: Option<f64>,
        #[arg(long)]
        target_amount: Option<f64>,
        #[arg(long, default_value = "BANK_TRANSFER")]
        pay_in: String,
        #[arg(long, default_value = "BANK_TRANSFER")]
        pay_out: String,
        /// Optional pre-selected target recipient account id.
        #[arg(long)]
        target_account: Option<i64>,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Create an unauthenticated example quote (POST /v3/quotes).
    Example {
        #[arg(long)]
        source: String,
        #[arg(long)]
        target: String,
        #[arg(long, conflicts_with = "target_amount")]
        source_amount: Option<f64>,
        #[arg(long)]
        target_amount: Option<f64>,
    },
    /// Get a quote by id.
    Get {
        quote_id: String,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Update a quote — typically to attach a recipient.
    Update {
        quote_id: String,
        #[arg(long)]
        target_account: Option<i64>,
        #[arg(long)]
        pay_in: Option<String>,
        #[arg(long)]
        pay_out: Option<String>,
        #[arg(long)]
        profile: Option<i64>,
    },
}

pub async fn run(cmd: QuoteCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        QuoteCmd::Create {
            source,
            target,
            source_amount,
            target_amount,
            pay_in,
            pay_out,
            target_account,
            profile,
        } => {
            let p = profile
                .or(ctx.config.default_profile)
                .ok_or_else(|| anyhow::anyhow!("--profile required (or set default-profile)"))?;
            ctx.confirm_prod("create a quote")?;
            let body = build_quote_body(
                &source,
                &target,
                source_amount,
                target_amount,
                Some(&pay_in),
                Some(&pay_out),
                target_account,
                Some(p),
            )?;
            let v: Value = ctx
                .client
                .post(&format!("/v3/profiles/{p}/quotes"), &body)
                .await?;
            output::print(&v, ctx.output());
        }
        QuoteCmd::Example {
            source,
            target,
            source_amount,
            target_amount,
        } => {
            let body = build_quote_body(
                &source,
                &target,
                source_amount,
                target_amount,
                None,
                None,
                None,
                None,
            )?;
            let v: Value = ctx.client.post("/v3/quotes", &body).await?;
            output::print(&v, ctx.output());
        }
        QuoteCmd::Get { quote_id, profile } => {
            let p = profile.or(ctx.config.default_profile).ok_or_else(|| {
                anyhow::anyhow!("--profile required (or set default-profile)")
            })?;
            let v: Value = ctx
                .client
                .get(&format!("/v3/profiles/{p}/quotes/{quote_id}"))
                .await?;
            output::print(&v, ctx.output());
        }
        QuoteCmd::Update {
            quote_id,
            target_account,
            pay_in,
            pay_out,
            profile,
        } => {
            let p = profile.or(ctx.config.default_profile).ok_or_else(|| {
                anyhow::anyhow!("--profile required (or set default-profile)")
            })?;
            let mut body = json!({});
            if let Some(t) = target_account {
                body["targetAccount"] = json!(t);
            }
            if let Some(pi) = pay_in {
                body["payIn"] = json!(pi);
            }
            if let Some(po) = pay_out {
                body["payOut"] = json!(po);
            }
            if body.as_object().map(|m| m.is_empty()).unwrap_or(true) {
                bail!("nothing to update — provide --target-account, --pay-in or --pay-out");
            }
            let v: Value = ctx
                .client
                .patch(&format!("/v3/profiles/{p}/quotes/{quote_id}"), &body)
                .await?;
            output::print(&v, ctx.output());
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_quote_body(
    source: &str,
    target: &str,
    source_amount: Option<f64>,
    target_amount: Option<f64>,
    pay_in: Option<&str>,
    pay_out: Option<&str>,
    target_account: Option<i64>,
    profile: Option<i64>,
) -> Result<Value> {
    if source_amount.is_none() && target_amount.is_none() {
        bail!("provide either --source-amount or --target-amount");
    }
    let mut body = json!({
        "sourceCurrency": source.to_uppercase(),
        "targetCurrency": target.to_uppercase(),
    });
    if let Some(a) = source_amount {
        body["sourceAmount"] = json!(a);
    }
    if let Some(a) = target_amount {
        body["targetAmount"] = json!(a);
    }
    if let Some(pi) = pay_in {
        body["payIn"] = json!(pi);
    }
    if let Some(po) = pay_out {
        body["payOut"] = json!(po);
    }
    if let Some(t) = target_account {
        body["targetAccount"] = json!(t);
    }
    if let Some(p) = profile {
        body["profile"] = json!(p);
    }
    Ok(body)
}
