// Sandbox-only simulation subcommands.
//
// Endpoints:
//   GET  /v1/simulation/transfers/{id}/{state}
//   POST /v1/simulation/balance/topup
//   POST /v1/simulation/profiles/{p}/verifications
//   POST /v1/simulation/verify-profile
//   POST /v1/simulation/profiles/{p}/bank-transactions/import
//   POST /v1/simulation/profiles/{p}/swift-in
//   POST /v2/simulation/spend/profiles/{p}/cards/{token}/transactions/authorisation
//   POST /v1/simulation/spend/profiles/{p}/cards/{token}/transactions/clearing
//   POST /v1/simulation/spend/profiles/{p}/cards/{token}/transactions/reversal

use anyhow::{Context as _, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::cli::Ctx;
use crate::config::Env;
use crate::output;

#[derive(Debug, Subcommand)]
pub enum SimulateCmd {
    /// Walk a transfer through its lifecycle (sandbox only).
    TransferState {
        transfer_id: i64,
        /// processing | funds_converted | outgoing_payment_sent | bounced_back | funds_refunded
        state: String,
    },
    /// Top up a balance with virtual funds.
    BalanceTopup {
        #[arg(long)]
        profile: i64,
        #[arg(long)]
        balance: i64,
        #[arg(long)]
        amount: f64,
        #[arg(long)]
        currency: String,
    },
    /// Force-verify a single profile.
    VerifyProfile { profile_id: i64 },
    /// Verify all profiles for the authenticated user.
    VerifyAll,
    /// Simulate an incoming bank transfer (USD/EUR/GBP).
    BankTx {
        #[arg(long)]
        profile: i64,
        #[arg(long)]
        amount: f64,
        #[arg(long)]
        currency: String,
        /// Optional free-form details JSON to merge into the body.
        #[arg(long)]
        details_json: Option<String>,
    },
    /// Simulate an incoming SWIFT payment.
    SwiftIn {
        #[arg(long)]
        profile: i64,
        #[arg(long)]
        amount: f64,
        #[arg(long)]
        currency: String,
        #[arg(long)]
        details_json: Option<String>,
    },
    /// Simulate a card transaction authorisation.
    CardAuth {
        #[arg(long)]
        profile: i64,
        #[arg(long)]
        card_token: String,
        /// Full request body JSON. The Wise API needs PAN + amount etc.
        #[arg(long)]
        body: String,
    },
    /// Clear a previously authorised card transaction.
    CardClearing {
        #[arg(long)]
        profile: i64,
        #[arg(long)]
        card_token: String,
        #[arg(long)]
        body: String,
    },
    /// Reverse a card transaction.
    CardReversal {
        #[arg(long)]
        profile: i64,
        #[arg(long)]
        card_token: String,
        #[arg(long)]
        body: String,
    },
}

pub async fn run(cmd: SimulateCmd, ctx: &Ctx) -> Result<()> {
    if ctx.client.env() == Env::Production {
        anyhow::bail!("simulation endpoints are sandbox-only — switch with --env sandbox");
    }
    match cmd {
        SimulateCmd::TransferState { transfer_id, state } => {
            // The state is a path segment per the docs (GET).
            let v: Value = ctx
                .client
                .get(&format!("/v1/simulation/transfers/{transfer_id}/{state}"))
                .await?;
            output::print(&v, ctx.output());
        }
        SimulateCmd::BalanceTopup {
            profile,
            balance,
            amount,
            currency,
        } => {
            let body = json!({
                "profileId": profile,
                "balanceId": balance,
                "amount": amount,
                "currency": currency.to_uppercase(),
            });
            let v: Value = ctx.client.post("/v1/simulation/balance/topup", &body).await?;
            output::print(&v, ctx.output());
        }
        SimulateCmd::VerifyProfile { profile_id } => {
            let v: Value = ctx
                .client
                .post(
                    &format!("/v1/simulation/profiles/{profile_id}/verifications"),
                    &json!({}),
                )
                .await?;
            output::print(&v, ctx.output());
        }
        SimulateCmd::VerifyAll => {
            let v: Value = ctx
                .client
                .post("/v1/simulation/verify-profile", &json!({}))
                .await?;
            output::print(&v, ctx.output());
        }
        SimulateCmd::BankTx {
            profile,
            amount,
            currency,
            details_json,
        } => {
            let mut body = json!({
                "amount": amount,
                "currency": currency.to_uppercase(),
            });
            if let Some(extra) = details_json {
                let extra: Value =
                    serde_json::from_str(&extra).context("--details-json must be JSON")?;
                if let Some(map) = extra.as_object() {
                    for (k, v) in map {
                        body[k] = v.clone();
                    }
                }
            }
            let v: Value = ctx
                .client
                .post(
                    &format!("/v1/simulation/profiles/{profile}/bank-transactions/import"),
                    &body,
                )
                .await?;
            output::print(&v, ctx.output());
        }
        SimulateCmd::SwiftIn {
            profile,
            amount,
            currency,
            details_json,
        } => {
            let mut body = json!({
                "amount": amount,
                "currency": currency.to_uppercase(),
            });
            if let Some(extra) = details_json {
                let extra: Value =
                    serde_json::from_str(&extra).context("--details-json must be JSON")?;
                if let Some(map) = extra.as_object() {
                    for (k, v) in map {
                        body[k] = v.clone();
                    }
                }
            }
            let v: Value = ctx
                .client
                .post(&format!("/v1/simulation/profiles/{profile}/swift-in"), &body)
                .await?;
            output::print(&v, ctx.output());
        }
        SimulateCmd::CardAuth {
            profile,
            card_token,
            body,
        } => {
            let body_v: Value = serde_json::from_str(&body).context("--body must be JSON")?;
            let v: Value = ctx
                .client
                .post(
                    &format!(
                        "/v2/simulation/spend/profiles/{profile}/cards/{card_token}/transactions/authorisation"
                    ),
                    &body_v,
                )
                .await?;
            output::print(&v, ctx.output());
        }
        SimulateCmd::CardClearing {
            profile,
            card_token,
            body,
        } => {
            let body_v: Value = serde_json::from_str(&body).context("--body must be JSON")?;
            let v: Value = ctx
                .client
                .post(
                    &format!(
                        "/v1/simulation/spend/profiles/{profile}/cards/{card_token}/transactions/clearing"
                    ),
                    &body_v,
                )
                .await?;
            output::print(&v, ctx.output());
        }
        SimulateCmd::CardReversal {
            profile,
            card_token,
            body,
        } => {
            let body_v: Value = serde_json::from_str(&body).context("--body must be JSON")?;
            let v: Value = ctx
                .client
                .post(
                    &format!(
                        "/v1/simulation/spend/profiles/{profile}/cards/{card_token}/transactions/reversal"
                    ),
                    &body_v,
                )
                .await?;
            output::print(&v, ctx.output());
        }
    }
    Ok(())
}
