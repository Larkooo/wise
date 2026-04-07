// wise — agent-friendly CLI for the Wise Platform API.
//
// Entry point: parses the top-level CLI, builds a WiseClient, dispatches to
// subcommands. Each subcommand module owns its own clap enum and run() fn.

use anyhow::Result;
use clap::{Parser, Subcommand};

mod cli;
mod client;
mod config;
mod output;

use cli::{
    activity::ActivityCmd, auth::AuthCmd, balance::BalanceCmd, card::CardCmd,
    card_order::CardOrderCmd, config_cmd::ConfigCmd, currency::CurrencyCmd, docs::DocsCmd,
    jose::JoseCmd, profile::ProfileCmd, quote::QuoteCmd, rate::RateCmd, recipient::RecipientCmd,
    simulate::SimulateCmd, transfer::TransferCmd, webhook::WebhookCmd,
};
use cli::{Ctx, GlobalArgs};

#[derive(Debug, Parser)]
#[command(
    name = "wise",
    about = "Agent-friendly CLI for the Wise Platform API",
    long_about = "Run Wise Platform API operations from the command line.\n\
                  Sandbox by default; use --env production for live operations.\n\
                  Output is JSON by default; use --pretty for indented JSON.\n\
                  Run `wise docs ask \"...\"` to query the live Wise docs.",
    version
)]
struct Cli {
    #[command(flatten)]
    global: GlobalArgs,

    #[command(subcommand)]
    cmd: TopCmd,
}

#[derive(Debug, Subcommand)]
enum TopCmd {
    /// Authenticate with Wise (token, OAuth client credentials).
    Auth {
        #[command(subcommand)]
        cmd: AuthCmd,
    },
    /// Read or write CLI configuration (default profile, env, etc).
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Personal and business profiles.
    Profile {
        #[command(subcommand)]
        cmd: ProfileCmd,
    },
    /// Multi-currency balance accounts (list, create, move money).
    Balance {
        #[command(subcommand)]
        cmd: BalanceCmd,
    },
    /// Quotes (exchange rate + fee calculations).
    Quote {
        #[command(subcommand)]
        cmd: QuoteCmd,
    },
    /// Recipient (beneficiary) accounts.
    Recipient {
        #[command(subcommand)]
        cmd: RecipientCmd,
    },
    /// Transfers — create, fund, track, cancel.
    Transfer {
        #[command(subcommand)]
        cmd: TransferCmd,
    },
    /// Cards: list, freeze, permissions.
    Card {
        #[command(subcommand)]
        cmd: CardCmd,
    },
    /// Card orders: programs, create, status.
    #[command(name = "card-order")]
    CardOrder {
        #[command(subcommand)]
        cmd: CardOrderCmd,
    },
    /// Webhook subscriptions (profile + application level).
    Webhook {
        #[command(subcommand)]
        cmd: WebhookCmd,
    },
    /// Current and historical exchange rates.
    Rate {
        #[command(subcommand)]
        cmd: RateCmd,
    },
    /// Profile activity feed.
    Activity {
        #[command(subcommand)]
        cmd: ActivityCmd,
    },
    /// Supported currencies.
    Currency {
        #[command(subcommand)]
        cmd: CurrencyCmd,
    },
    /// Ask the live Wise docs (wraps docs.wise.com /_ask-ai).
    Docs {
        #[command(subcommand)]
        cmd: DocsCmd,
    },
    /// Sandbox-only simulations (transfer state, top-ups, card auth).
    Simulate {
        #[command(subcommand)]
        cmd: SimulateCmd,
    },
    /// JWE debug helpers (fetch Wise's encryption key, encrypt, decrypt).
    /// Used by the agent-card flow under the hood; see AGENT.md.
    Jose {
        #[command(subcommand)]
        cmd: JoseCmd,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_tracing(cli.global.verbose);

    let ctx = match Ctx::new(cli.global.clone()).await {
        Ok(c) => c,
        Err(e) => {
            output::print_error(&e, &cli.global);
            std::process::exit(2);
        }
    };

    let result = dispatch(cli.cmd, &ctx).await;
    match result {
        Ok(()) => {}
        Err(e) => {
            output::print_error(&e, &cli.global);
            std::process::exit(1);
        }
    }
}

async fn dispatch(cmd: TopCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        TopCmd::Auth { cmd } => cli::auth::run(cmd, ctx).await,
        TopCmd::Config { cmd } => cli::config_cmd::run(cmd, ctx).await,
        TopCmd::Profile { cmd } => cli::profile::run(cmd, ctx).await,
        TopCmd::Balance { cmd } => cli::balance::run(cmd, ctx).await,
        TopCmd::Quote { cmd } => cli::quote::run(cmd, ctx).await,
        TopCmd::Recipient { cmd } => cli::recipient::run(cmd, ctx).await,
        TopCmd::Transfer { cmd } => cli::transfer::run(cmd, ctx).await,
        TopCmd::Card { cmd } => cli::card::run(cmd, ctx).await,
        TopCmd::CardOrder { cmd } => cli::card_order::run(cmd, ctx).await,
        TopCmd::Webhook { cmd } => cli::webhook::run(cmd, ctx).await,
        TopCmd::Rate { cmd } => cli::rate::run(cmd, ctx).await,
        TopCmd::Activity { cmd } => cli::activity::run(cmd, ctx).await,
        TopCmd::Currency { cmd } => cli::currency::run(cmd, ctx).await,
        TopCmd::Docs { cmd } => cli::docs::run(cmd, ctx).await,
        TopCmd::Simulate { cmd } => cli::simulate::run(cmd, ctx).await,
        TopCmd::Jose { cmd } => cli::jose::run(cmd, ctx).await,
    }
}

fn init_tracing(verbose: bool) {
    use tracing_subscriber::{fmt, EnvFilter};
    let default = if verbose { "wise=debug,info" } else { "warn" };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default));
    fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .compact()
        .init();
}
