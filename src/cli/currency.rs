// Currency subcommand. GET /v1/currencies

use anyhow::Result;
use clap::Subcommand;
use serde_json::Value;

use crate::cli::Ctx;
use crate::output;

#[derive(Debug, Subcommand)]
pub enum CurrencyCmd {
    /// List all currencies supported for transfers.
    List,
}

pub async fn run(cmd: CurrencyCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        CurrencyCmd::List => {
            let v: Value = ctx.client.get("/v1/currencies").await?;
            output::print(&v, ctx.output());
        }
    }
    Ok(())
}
