// Recipient (beneficiary) account subcommands.
//
// The shape of recipient `details` varies wildly per currency, so we accept
// a free-form `--details` JSON object passed by the agent. Use
// `wise recipient requirements --quote <id>` to discover what's required.
//
// Endpoints:
//   POST /v1/accounts
//   GET  /v2/accounts
//   GET  /v2/accounts/{id}
//   DELETE /v2/accounts/{id}
//   GET  /v1/quotes/{quoteId}/account-requirements

use anyhow::{Context as _, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::cli::Ctx;
use crate::output;

#[derive(Debug, Subcommand)]
pub enum RecipientCmd {
    /// List recipients.
    List {
        #[arg(long)]
        profile: Option<i64>,
        #[arg(long)]
        currency: Option<String>,
        #[arg(long)]
        size: Option<u32>,
    },
    /// Create a recipient. `details` is a free-form JSON object whose shape
    /// depends on the currency — use `recipient requirements` to discover.
    Create {
        #[arg(long)]
        currency: String,
        /// Recipient type (e.g. `sort_code`, `iban`, `aba`, `email`, ...).
        #[arg(long, name = "type")]
        r#type: String,
        #[arg(long)]
        account_holder_name: String,
        /// JSON object passed verbatim as the `details` field.
        #[arg(long)]
        details: String,
        #[arg(long)]
        profile: Option<i64>,
        /// Mark this recipient as owned by the customer (self-transfers).
        #[arg(long)]
        owned_by_customer: bool,
        /// Create as a refund recipient (?refund=true).
        #[arg(long)]
        refund: bool,
    },
    /// Get a recipient by id.
    Get { account_id: i64 },
    /// Delete (deactivate) a recipient.
    Delete { account_id: i64 },
    /// Discover the required fields for a recipient given a quote.
    Requirements {
        #[arg(long)]
        quote: String,
        /// Force address fields to be returned.
        #[arg(long)]
        address_required: bool,
    },
}

pub async fn run(cmd: RecipientCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        RecipientCmd::List {
            profile,
            currency,
            size,
        } => {
            let mut params: Vec<(String, String)> = Vec::new();
            // resolve_profile is fail-fast: if a sandbox is active and the
            // profile is restricted, this errors before any network call.
            // If no profile is set anywhere, list everything (no filter).
            if profile.is_some() || ctx.config.default_profile.is_some() {
                let p = ctx.resolve_profile(profile)?;
                params.push(("profile".into(), p.to_string()));
            }
            if let Some(c) = currency {
                params.push(("currency".into(), c.to_uppercase()));
            }
            if let Some(s) = size {
                params.push(("size".into(), s.to_string()));
            }
            let v: Value = ctx.client.get_query("/v2/accounts", &params).await?;
            output::print(&v, ctx.output());
        }
        RecipientCmd::Create {
            currency,
            r#type,
            account_holder_name,
            details,
            profile,
            owned_by_customer,
            refund,
        } => {
            ctx.confirm_prod("create a recipient")?;
            let details_v: Value =
                serde_json::from_str(&details).context("--details must be a JSON object")?;
            let mut body = json!({
                "currency": currency.to_uppercase(),
                "type": r#type,
                "accountHolderName": account_holder_name,
                "ownedByCustomer": owned_by_customer,
                "details": details_v,
            });
            if profile.is_some() || ctx.config.default_profile.is_some() {
                let p = ctx.resolve_profile(profile)?;
                body["profile"] = json!(p);
            }
            let path = if refund {
                "/v1/accounts?refund=true"
            } else {
                "/v1/accounts"
            };
            let v: Value = ctx.client.post(path, &body).await?;
            output::print(&v, ctx.output());
        }
        RecipientCmd::Get { account_id } => {
            let v: Value = ctx.client.get(&format!("/v2/accounts/{account_id}")).await?;
            output::print(&v, ctx.output());
        }
        RecipientCmd::Delete { account_id } => {
            ctx.confirm_prod("delete a recipient")?;
            let v = ctx.client.delete(&format!("/v2/accounts/{account_id}")).await?;
            output::print(&v, ctx.output());
        }
        RecipientCmd::Requirements {
            quote,
            address_required,
        } => {
            let path = if address_required {
                format!("/v1/quotes/{quote}/account-requirements?addressRequired=true")
            } else {
                format!("/v1/quotes/{quote}/account-requirements")
            };
            let v: Value = ctx.client.get(&path).await?;
            output::print(&v, ctx.output());
        }
    }
    Ok(())
}
