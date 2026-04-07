// Rate subcommands.
//   GET /v1/rates                                       (all)
//   GET /v1/rates?source=A&target=B                     (one pair)
//   GET /v1/rates?source=A&target=B&time=...            (historical point)
//   GET /v1/rates?source=A&target=B&from=&to=&group=    (history)

use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use serde_json::Value;

use crate::cli::Ctx;
use crate::output;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum RateGroup {
    Day,
    Hour,
    Minute,
}

impl RateGroup {
    fn as_str(&self) -> &'static str {
        match self {
            RateGroup::Day => "day",
            RateGroup::Hour => "hour",
            RateGroup::Minute => "minute",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum RateCmd {
    /// Current rate (or all rates if no pair given).
    Get {
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        target: Option<String>,
        /// Historical point ISO8601 timestamp (e.g. 2026-04-07T12:00:00).
        #[arg(long)]
        time: Option<String>,
    },
    /// Historical rates over a window grouped by day/hour/minute.
    History {
        #[arg(long)]
        source: String,
        #[arg(long)]
        target: String,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long, value_enum, default_value = "day")]
        group: RateGroup,
    },
}

pub async fn run(cmd: RateCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        RateCmd::Get { source, target, time } => {
            let mut q: Vec<(String, String)> = Vec::new();
            if let Some(s) = source {
                q.push(("source".into(), s.to_uppercase()));
            }
            if let Some(t) = target {
                q.push(("target".into(), t.to_uppercase()));
            }
            if let Some(ts) = time {
                q.push(("time".into(), ts));
            }
            let v: Value = ctx.client.get_query("/v1/rates", &q).await?;
            output::print(&v, ctx.output());
        }
        RateCmd::History {
            source,
            target,
            from,
            to,
            group,
        } => {
            let q = [
                ("source".to_string(), source.to_uppercase()),
                ("target".to_string(), target.to_uppercase()),
                ("from".to_string(), from),
                ("to".to_string(), to),
                ("group".to_string(), group.as_str().to_string()),
            ];
            let v: Value = ctx.client.get_query("/v1/rates", &q).await?;
            output::print(&v, ctx.output());
        }
    }
    Ok(())
}
