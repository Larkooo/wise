// Balance subcommands — multi-currency account operations.
//
// Endpoints used:
//   GET    /v4/profiles/{p}/balances?types=...
//   POST   /v4/profiles/{p}/balances
//   GET    /v4/profiles/{p}/balances/{id}
//   DELETE /v4/profiles/{p}/balances/{id}
//   POST   /v2/profiles/{p}/balance-movements
//   GET    /v1/profiles/{p}/total-funds/{currency}
//   POST   /v1/simulation/balance/topup    (sandbox only — see simulate.rs too)

use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use serde_json::{json, Value};

use crate::cli::Ctx;
use crate::output;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum BalanceType {
    Standard,
    Savings,
}

impl BalanceType {
    fn as_str(&self) -> &'static str {
        match self {
            BalanceType::Standard => "STANDARD",
            BalanceType::Savings => "SAVINGS",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum BalanceCmd {
    /// List balances on a profile.
    List {
        /// Comma-separated list of types (default: STANDARD,SAVINGS).
        #[arg(long, default_value = "STANDARD,SAVINGS")]
        types: String,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Get one balance by id.
    Get {
        balance_id: i64,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Create a balance account in the given currency.
    Create {
        #[arg(long)]
        currency: String,
        #[arg(long, value_enum, default_value = "standard")]
        r#type: BalanceType,
        /// Required for SAVINGS jars; ignored for STANDARD.
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Close a balance account (must be zero-balance).
    Delete {
        balance_id: i64,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Move money between balances (same-currency move or quote-driven convert).
    Move {
        /// Source balance id.
        #[arg(long)]
        from: i64,
        /// Target balance id.
        #[arg(long)]
        to: i64,
        /// Amount to move (omit for quote-driven movement).
        #[arg(long)]
        amount: Option<f64>,
        /// Currency for `--amount` (omit for quote).
        #[arg(long)]
        currency: Option<String>,
        /// Quote id (required for cross-currency conversion).
        #[arg(long)]
        quote: Option<String>,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Sandbox-only top up.
    Topup {
        balance_id: i64,
        #[arg(long)]
        amount: f64,
        #[arg(long)]
        currency: String,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Total worth + available across balances in one currency.
    Total {
        #[arg(long)]
        currency: String,
        #[arg(long)]
        profile: Option<i64>,
    },
}

pub async fn run(cmd: BalanceCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        BalanceCmd::List { types, profile } => {
            let p = profile_or_required(ctx, profile)?;
            let v: Value = ctx
                .client
                .get_query(&format!("/v4/profiles/{p}/balances"), &[("types", types)])
                .await?;
            output::print(&v, ctx.output());
        }
        BalanceCmd::Get { balance_id, profile } => {
            let p = profile_or_required(ctx, profile)?;
            let v: Value = ctx
                .client
                .get(&format!("/v4/profiles/{p}/balances/{balance_id}"))
                .await?;
            output::print(&v, ctx.output());
        }
        BalanceCmd::Create {
            currency,
            r#type,
            name,
            profile,
        } => {
            let p = profile_or_required(ctx, profile)?;
            ctx.confirm_prod("create a balance")?;
            let body = json!({
                "currency": currency.to_uppercase(),
                "type": r#type.as_str(),
                "name": name,
            });
            let v: Value = ctx
                .client
                .post_idempotent(&format!("/v4/profiles/{p}/balances"), &body)
                .await?;
            output::print(&v, ctx.output());
        }
        BalanceCmd::Delete { balance_id, profile } => {
            let p = profile_or_required(ctx, profile)?;
            ctx.confirm_prod("delete a balance")?;
            let v = ctx
                .client
                .delete(&format!("/v4/profiles/{p}/balances/{balance_id}"))
                .await?;
            output::print(&v, ctx.output());
        }
        BalanceCmd::Move {
            from,
            to,
            amount,
            currency,
            quote,
            profile,
        } => {
            let p = profile_or_required(ctx, profile)?;
            ctx.confirm_prod("move money between balances")?;
            let mut body = json!({
                "sourceBalanceId": from,
                "targetBalanceId": to,
            });
            if let Some(q) = quote {
                body["quoteId"] = json!(q);
            }
            if let (Some(amt), Some(cur)) = (amount, currency) {
                body["amount"] = json!({
                    "value": amt,
                    "currency": cur.to_uppercase(),
                });
            }
            let v: Value = ctx
                .client
                .post_idempotent(&format!("/v2/profiles/{p}/balance-movements"), &body)
                .await?;
            output::print(&v, ctx.output());
        }
        BalanceCmd::Topup {
            balance_id,
            amount,
            currency,
            profile,
        } => {
            let p = profile_or_required(ctx, profile)?;
            // Sandbox top-up endpoint shape from docs.
            let body = json!({
                "profileId": p,
                "balanceId": balance_id,
                "amount": amount,
                "currency": currency.to_uppercase(),
            });
            let v: Value = ctx.client.post("/v1/simulation/balance/topup", &body).await?;
            output::print(&v, ctx.output());
        }
        BalanceCmd::Total { currency, profile } => {
            let p = profile_or_required(ctx, profile)?;
            let v: Value = ctx
                .client
                .get(&format!(
                    "/v1/profiles/{p}/total-funds/{}",
                    currency.to_uppercase()
                ))
                .await?;
            output::print(&v, ctx.output());
        }
    }
    Ok(())
}

fn profile_or_required(ctx: &Ctx, override_profile: Option<i64>) -> Result<i64> {
    if let Some(p) = override_profile {
        Ok(p)
    } else {
        ctx.require_profile()
    }
}
