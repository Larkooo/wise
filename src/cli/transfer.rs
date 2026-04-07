// Transfer subcommands.
//
// Endpoints:
//   POST /v1/transfers
//   GET  /v1/transfers
//   GET  /v1/transfers/{id}
//   PUT  /v1/transfers/{id}/cancel
//   POST /v3/profiles/{p}/transfers/{id}/payments  (fund)
//   POST /v1/transfer-requirements
//   GET  /v1/transfers/{id}/payments
//   GET  /v1/transfers/{id}/receipt.pdf

use anyhow::{Context as _, Result};
use clap::Subcommand;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::cli::Ctx;
use crate::output;

#[derive(Debug, Subcommand)]
pub enum TransferCmd {
    /// Create a transfer (POST /v1/transfers).
    Create {
        #[arg(long)]
        quote: String,
        #[arg(long)]
        target_account: i64,
        #[arg(long)]
        reference: Option<String>,
        #[arg(long)]
        purpose: Option<String>,
        #[arg(long)]
        source_of_funds: Option<String>,
        /// Custom transaction id for idempotency. Auto-generated if absent.
        #[arg(long)]
        customer_tx_id: Option<String>,
        /// Free-form details JSON to merge into the request body.
        #[arg(long)]
        details_json: Option<String>,
    },
    /// List transfers.
    List {
        #[arg(long)]
        profile: Option<i64>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long, name = "source-currency")]
        source_currency: Option<String>,
        #[arg(long, name = "target-currency")]
        target_currency: Option<String>,
        #[arg(long, name = "created-since")]
        created_since: Option<String>,
        #[arg(long, name = "created-before")]
        created_before: Option<String>,
        #[arg(long, default_value = "20")]
        limit: u32,
        #[arg(long, default_value = "0")]
        offset: u32,
    },
    /// Get a transfer by id.
    Get { transfer_id: i64 },
    /// Cancel a transfer (must be in a cancellable state).
    Cancel { transfer_id: i64 },
    /// Fund a transfer from a balance.
    Fund {
        transfer_id: i64,
        /// BALANCE | TRUSTED_PRE_FUND_BULK | BANK_TRANSFER (batch only).
        #[arg(long, default_value = "BALANCE")]
        r#type: String,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Discover required fields for a transfer creation given a quote+account.
    Requirements {
        #[arg(long)]
        quote: String,
        #[arg(long)]
        target_account: i64,
        #[arg(long)]
        details_json: Option<String>,
    },
    /// List completed payments backing a transfer.
    Payments { transfer_id: i64 },
    /// Download the PDF receipt to a file (or stdout if --output -).
    Receipt {
        transfer_id: i64,
        /// Output path. Defaults to ./transfer-{id}.pdf.
        #[arg(long, short = 'o')]
        output: Option<String>,
    },
}

pub async fn run(cmd: TransferCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        TransferCmd::Create {
            quote,
            target_account,
            reference,
            purpose,
            source_of_funds,
            customer_tx_id,
            details_json,
        } => {
            ctx.confirm_prod("create a transfer")?;
            let mut details = json!({});
            if let Some(r) = reference {
                details["reference"] = json!(r);
            }
            if let Some(p) = purpose {
                details["transferPurpose"] = json!(p);
            }
            if let Some(s) = source_of_funds {
                details["sourceOfFunds"] = json!(s);
            }
            if let Some(extra) = details_json {
                let extra_v: Value =
                    serde_json::from_str(&extra).context("--details-json must be JSON")?;
                if let Some(map) = extra_v.as_object() {
                    for (k, v) in map {
                        details[k] = v.clone();
                    }
                }
            }
            let body = json!({
                "targetAccount": target_account,
                "quoteUuid": quote,
                "customerTransactionId": customer_tx_id.unwrap_or_else(|| Uuid::new_v4().to_string()),
                "details": details,
            });
            let v: Value = ctx.client.post("/v1/transfers", &body).await?;
            output::print(&v, ctx.output());
        }
        TransferCmd::List {
            profile,
            status,
            source_currency,
            target_currency,
            created_since,
            created_before,
            limit,
            offset,
        } => {
            let mut q: Vec<(String, String)> = Vec::new();
            let p = profile.or(ctx.config.default_profile);
            if let Some(p) = p {
                q.push(("profile".into(), p.to_string()));
            }
            if let Some(s) = status {
                q.push(("status".into(), s));
            }
            if let Some(s) = source_currency {
                q.push(("sourceCurrency".into(), s.to_uppercase()));
            }
            if let Some(t) = target_currency {
                q.push(("targetCurrency".into(), t.to_uppercase()));
            }
            if let Some(d) = created_since {
                q.push(("createdDateStart".into(), d));
            }
            if let Some(d) = created_before {
                q.push(("createdDateEnd".into(), d));
            }
            q.push(("limit".into(), limit.to_string()));
            q.push(("offset".into(), offset.to_string()));
            let v: Value = ctx.client.get_query("/v1/transfers", &q).await?;
            output::print(&v, ctx.output());
        }
        TransferCmd::Get { transfer_id } => {
            let v: Value = ctx.client.get(&format!("/v1/transfers/{transfer_id}")).await?;
            output::print(&v, ctx.output());
        }
        TransferCmd::Cancel { transfer_id } => {
            ctx.confirm_prod("cancel a transfer")?;
            let v: Value = ctx
                .client
                .put_empty(&format!("/v1/transfers/{transfer_id}/cancel"))
                .await?;
            output::print(&v, ctx.output());
        }
        TransferCmd::Fund {
            transfer_id,
            r#type,
            profile,
        } => {
            ctx.confirm_prod("fund a transfer")?;
            let p = profile.or(ctx.config.default_profile).ok_or_else(|| {
                anyhow::anyhow!("--profile required (or set default-profile)")
            })?;
            let body = json!({ "type": r#type });
            let v: Value = ctx
                .client
                .post(
                    &format!("/v3/profiles/{p}/transfers/{transfer_id}/payments"),
                    &body,
                )
                .await?;
            output::print(&v, ctx.output());
        }
        TransferCmd::Requirements {
            quote,
            target_account,
            details_json,
        } => {
            let mut details = json!({});
            if let Some(extra) = details_json {
                details = serde_json::from_str(&extra).context("--details-json must be JSON")?;
            }
            let body = json!({
                "targetAccount": target_account,
                "quoteUuid": quote,
                "details": details,
            });
            let v: Value = ctx.client.post("/v1/transfer-requirements", &body).await?;
            output::print(&v, ctx.output());
        }
        TransferCmd::Payments { transfer_id } => {
            let v: Value = ctx
                .client
                .get(&format!("/v1/transfers/{transfer_id}/payments"))
                .await?;
            output::print(&v, ctx.output());
        }
        TransferCmd::Receipt { transfer_id, output: out_path } => {
            let path = out_path.unwrap_or_else(|| format!("transfer-{transfer_id}.pdf"));
            let bytes = ctx
                .client
                .get_bytes(&format!("/v1/transfers/{transfer_id}/receipt.pdf"))
                .await?;
            if path == "-" {
                use std::io::Write;
                let mut out = std::io::stdout().lock();
                out.write_all(&bytes)?;
            } else {
                std::fs::write(&path, &bytes)
                    .with_context(|| format!("writing receipt to {path}"))?;
                output::print(
                    &json!({ "saved": true, "path": path, "bytes": bytes.len() }),
                    ctx.output(),
                );
            }
        }
    }
    Ok(())
}
