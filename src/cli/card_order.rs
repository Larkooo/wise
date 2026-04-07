// Card-order subcommands.
//
// Endpoints:
//   POST /v3/spend/profiles/{p}/card-orders
//   GET  /v3/spend/profiles/{p}/card-orders
//   GET  /v3/spend/profiles/{p}/card-orders/{id}
//   GET  /v3/spend/profiles/{p}/card-orders/availability
//   GET  /v3/spend/profiles/{p}/card-orders/{id}/requirements
//   PUT  /v3/spend/profiles/{p}/card-orders/{id}/status

use anyhow::{Context as _, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::cli::Ctx;
use crate::output;

#[derive(Debug, Subcommand)]
pub enum CardOrderCmd {
    /// List available card programs for the profile.
    Programs {
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Create a card order. The body shape is large; pass `--body` JSON for
    /// full control, or use the convenience flags below for the simple case.
    Create {
        /// Full request body JSON. Overrides convenience flags if provided.
        #[arg(long)]
        body: Option<String>,
        /// Card program (from `card-order programs`).
        #[arg(long)]
        program: Option<String>,
        /// VIRTUAL | PHYSICAL.
        #[arg(long)]
        r#type: Option<String>,
        #[arg(long)]
        cardholder_profile_id: Option<i64>,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// List card orders for a profile.
    List {
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Get a card order by id.
    Get {
        card_order_id: String,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Get the requirements for a card order.
    Requirements {
        card_order_id: String,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Cancel a card order (PUT /status with CANCELLED).
    Cancel {
        card_order_id: String,
        #[arg(long)]
        profile: Option<i64>,
    },
}

pub async fn run(cmd: CardOrderCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        CardOrderCmd::Programs { profile } => {
            let p = require_profile(ctx, profile)?;
            let v: Value = ctx
                .client
                .get(&format!("/v3/spend/profiles/{p}/card-orders/availability"))
                .await?;
            output::print(&v, ctx.output());
        }
        CardOrderCmd::Create {
            body,
            program,
            r#type,
            cardholder_profile_id,
            profile,
        } => {
            let p = require_profile(ctx, profile)?;
            ctx.confirm_prod("create a card order")?;
            let body_v: Value = if let Some(b) = body {
                serde_json::from_str(&b).context("--body must be JSON")?
            } else {
                let mut m = json!({});
                if let Some(prog) = program {
                    m["program"] = json!(prog);
                }
                if let Some(t) = r#type {
                    m["type"] = json!(t);
                }
                if let Some(c) = cardholder_profile_id {
                    m["cardHolderProfileId"] = json!(c);
                }
                m
            };
            let v: Value = ctx
                .client
                .post_idempotent(&format!("/v3/spend/profiles/{p}/card-orders"), &body_v)
                .await?;
            output::print(&v, ctx.output());
        }
        CardOrderCmd::List { profile } => {
            let p = require_profile(ctx, profile)?;
            let v: Value = ctx
                .client
                .get(&format!("/v3/spend/profiles/{p}/card-orders"))
                .await?;
            output::print(&v, ctx.output());
        }
        CardOrderCmd::Get {
            card_order_id,
            profile,
        } => {
            let p = require_profile(ctx, profile)?;
            let v: Value = ctx
                .client
                .get(&format!("/v3/spend/profiles/{p}/card-orders/{card_order_id}"))
                .await?;
            output::print(&v, ctx.output());
        }
        CardOrderCmd::Requirements {
            card_order_id,
            profile,
        } => {
            let p = require_profile(ctx, profile)?;
            let v: Value = ctx
                .client
                .get(&format!(
                    "/v3/spend/profiles/{p}/card-orders/{card_order_id}/requirements"
                ))
                .await?;
            output::print(&v, ctx.output());
        }
        CardOrderCmd::Cancel {
            card_order_id,
            profile,
        } => {
            let p = require_profile(ctx, profile)?;
            ctx.confirm_prod("cancel a card order")?;
            let body = json!({ "status": "CANCELLED" });
            let v: Value = ctx
                .client
                .put(
                    &format!("/v3/spend/profiles/{p}/card-orders/{card_order_id}/status"),
                    &body,
                )
                .await?;
            output::print(&v, ctx.output());
        }
    }
    Ok(())
}

fn require_profile(ctx: &Ctx, override_profile: Option<i64>) -> Result<i64> {
    if let Some(p) = override_profile {
        Ok(p)
    } else {
        ctx.require_profile()
    }
}
