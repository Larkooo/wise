// Card subcommands.
//
// Endpoints:
//   GET   /v3/spend/profiles/{p}/cards
//   GET   /v3/spend/profiles/{p}/cards/{token}
//   PUT   /v3/spend/profiles/{p}/cards/{token}/status
//   POST  /v3/spend/profiles/{p}/cards/{token}/reset-pin-count
//   GET   /v3/spend/profiles/{p}/cards/{token}/spending-permissions
//   PATCH /v4/spend/profiles/{p}/cards/{token}/spending-permissions  (bulk)

use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use serde_json::{json, Value};

use crate::cli::Ctx;
use crate::output;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CardStatus {
    Active,
    Frozen,
    Blocked,
}

impl CardStatus {
    fn as_str(&self) -> &'static str {
        match self {
            CardStatus::Active => "ACTIVE",
            CardStatus::Frozen => "FROZEN",
            CardStatus::Blocked => "BLOCKED",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum CardCmd {
    /// List cards on a profile.
    List {
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Get one card by token.
    Get {
        card_token: String,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Update card status (ACTIVE | FROZEN | BLOCKED).
    Status {
        card_token: String,
        #[arg(long, value_enum)]
        status: CardStatus,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Reset the wrong-PIN counter.
    ResetPinCount {
        card_token: String,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Spending permissions.
    Permissions {
        #[command(subcommand)]
        cmd: PermissionsCmd,
    },
}

#[derive(Debug, Subcommand)]
pub enum PermissionsCmd {
    /// Get current permissions for a card.
    Get {
        card_token: String,
        #[arg(long)]
        profile: Option<i64>,
    },
    /// Bulk-update permissions. Pass a JSON object with permission flags
    /// (e.g. `{"ECOM": true, "ATM": false}`) via --permissions.
    Set {
        card_token: String,
        /// JSON object of {permission: bool}.
        #[arg(long)]
        permissions: String,
        #[arg(long)]
        profile: Option<i64>,
    },
}

pub async fn run(cmd: CardCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        CardCmd::List { profile } => {
            let p = ctx.resolve_profile(profile)?;
            let v: Value = ctx
                .client
                .get(&format!("/v3/spend/profiles/{p}/cards"))
                .await?;
            output::print(&v, ctx.output());
        }
        CardCmd::Get { card_token, profile } => {
            let p = ctx.resolve_profile(profile)?;
            ctx.check_card(&card_token)?;
            let v: Value = ctx
                .client
                .get(&format!("/v3/spend/profiles/{p}/cards/{card_token}"))
                .await?;
            output::print(&v, ctx.output());
        }
        CardCmd::Status {
            card_token,
            status,
            profile,
        } => {
            let p = ctx.resolve_profile(profile)?;
            ctx.check_card(&card_token)?;
            ctx.confirm_prod("change card status")?;
            let body = json!({ "status": status.as_str() });
            let v: Value = ctx
                .client
                .put(
                    &format!("/v3/spend/profiles/{p}/cards/{card_token}/status"),
                    &body,
                )
                .await?;
            output::print(&v, ctx.output());
        }
        CardCmd::ResetPinCount { card_token, profile } => {
            let p = ctx.resolve_profile(profile)?;
            ctx.check_card(&card_token)?;
            let v: Value = ctx
                .client
                .post(
                    &format!("/v3/spend/profiles/{p}/cards/{card_token}/reset-pin-count"),
                    &json!({}),
                )
                .await?;
            output::print(&v, ctx.output());
        }
        CardCmd::Permissions { cmd } => match cmd {
            PermissionsCmd::Get { card_token, profile } => {
                let p = ctx.resolve_profile(profile)?;
                ctx.check_card(&card_token)?;
                let v: Value = ctx
                    .client
                    .get(&format!(
                        "/v3/spend/profiles/{p}/cards/{card_token}/spending-permissions"
                    ))
                    .await?;
                output::print(&v, ctx.output());
            }
            PermissionsCmd::Set {
                card_token,
                permissions,
                profile,
            } => {
                let p = ctx.resolve_profile(profile)?;
                ctx.check_card(&card_token)?;
                ctx.confirm_prod("update card permissions")?;
                let body: Value = serde_json::from_str(&permissions)
                    .map_err(|e| anyhow::anyhow!("--permissions must be a JSON object: {e}"))?;
                let v: Value = ctx
                    .client
                    .patch(
                        &format!(
                            "/v4/spend/profiles/{p}/cards/{card_token}/spending-permissions"
                        ),
                        &body,
                    )
                    .await?;
                output::print(&v, ctx.output());
            }
        },
    }
    Ok(())
}

