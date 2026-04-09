// wise — CLI for the Wise Platform API.
//
// Entry point: parses the top-level CLI, builds a WiseClient, dispatches to
// subcommands. Each subcommand module owns its own clap enum and run() fn.

use anyhow::Result;
use clap::{Parser, Subcommand};
use serde_json::json;

mod agent;
mod cli;
mod client;
mod config;
mod output;
mod sandbox;

use cli::agent::AgentCmd;
use cli::sandbox::SandboxCmd;
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
    about = "CLI for the Wise Platform API",
    long_about = "Run Wise Platform API operations from the command line.\n\
                  Select the target API environment with --env or `wise config set env ...`.\n\
                  Use --sandbox <name> to activate optional policy controls for automation.\n\
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
    /// Sandbox policies for scoped agent CLI access (see SANDBOX.md).
    Sandbox {
        #[command(subcommand)]
        cmd: SandboxCmd,
    },
    /// Manual-paste agent card flow (see AGENT.md).
    Agent {
        #[command(subcommand)]
        cmd: AgentCmd,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_tracing(cli.global.verbose);
    let require_env = top_cmd_requires_env(&cli.cmd);

    let ctx = match Ctx::new(cli.global.clone(), require_env).await {
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
    // Sandbox dispatch + condition gates run *before* the command handler
    // so a denied command never establishes a network connection.
    let cmd_path = top_cmd_path(&cmd);
    let cmd_args = top_cmd_args(&cmd);
    let mut audit_handle: Option<sandbox::AuditEntry> = None;
    if let Some(sb) = &ctx.sandbox {
        // 1. Sandbox-management commands cannot run from inside an active
        //    sandbox — that would let an agent rewrite its own policy.
        if cmd_path.starts_with("sandbox.") {
            return Err(anyhow::anyhow!(
                "sandbox_denied: `{cmd_path}` cannot be invoked from inside an active sandbox \
                 (sandbox = `{}`)",
                sb.name()
            ));
        }
        // 2. Dispatch gate.
        sb.check_command(&cmd_path, &cmd_args)?;
        // 3. Per-command conditions (rate limit, --justify, audit).
        let args_json = sandbox_args_json(&cmd_args);
        audit_handle = sb.enforce_conditions(
            &cmd_path,
            &args_json,
            ctx.args.justify.as_deref(),
        )?;
    }

    let result = match cmd {
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
        TopCmd::Sandbox { cmd } => cli::sandbox::run(cmd, ctx).await,
        TopCmd::Agent { cmd } => cli::agent::run(cmd, ctx).await,
    };

    if let Some(handle) = audit_handle {
        match &result {
            Ok(()) => {
                let _ = handle.complete(json!({"ok": true}));
            }
            Err(e) => {
                let _ = handle.fail(&e.to_string());
            }
        }
    }

    result
}

fn top_cmd_requires_env(cmd: &TopCmd) -> bool {
    match cmd {
        TopCmd::Config { .. } => false,
        TopCmd::Sandbox { .. } => false,
        TopCmd::Docs { .. } => false,
        TopCmd::Agent { cmd } => !matches!(
            cmd,
            AgentCmd::Init { .. }
                | AgentCmd::Paste { .. }
                | AgentCmd::Status { .. }
                | AgentCmd::Fetch { .. }
                | AgentCmd::Rotate { .. }
                | AgentCmd::Panic { .. }
        ),
        TopCmd::Jose { cmd } => !matches!(
            cmd,
            JoseCmd::Encrypt { .. } | JoseCmd::Decrypt { .. }
        ),
        _ => true,
    }
}

/// Render the top-level command's sandbox path. The thin wrappers in
/// sandbox::path own the per-leaf strings; this function just dispatches.
fn top_cmd_path(cmd: &TopCmd) -> String {
    use sandbox::Cmd;
    sandbox::command_path(match cmd {
        TopCmd::Auth { cmd } => Cmd::Auth(cmd),
        TopCmd::Config { cmd } => Cmd::Config(cmd),
        TopCmd::Profile { cmd } => Cmd::Profile(cmd),
        TopCmd::Balance { cmd } => Cmd::Balance(cmd),
        TopCmd::Quote { cmd } => Cmd::Quote(cmd),
        TopCmd::Recipient { cmd } => Cmd::Recipient(cmd),
        TopCmd::Transfer { cmd } => Cmd::Transfer(cmd),
        TopCmd::Card { cmd } => Cmd::Card(cmd),
        TopCmd::CardOrder { cmd } => Cmd::CardOrder(cmd),
        TopCmd::Webhook { cmd } => Cmd::Webhook(cmd),
        TopCmd::Rate { cmd } => Cmd::Rate(cmd),
        TopCmd::Activity { cmd } => Cmd::Activity(cmd),
        TopCmd::Currency { cmd } => Cmd::Currency(cmd),
        TopCmd::Docs { cmd } => Cmd::Docs(cmd),
        TopCmd::Simulate { cmd } => Cmd::Simulate(cmd),
        TopCmd::Jose { cmd } => Cmd::Jose(cmd),
        TopCmd::Sandbox { cmd } => Cmd::Sandbox(cmd),
        TopCmd::Agent { cmd } => Cmd::Agent(cmd),
    })
}

/// Surface argument-aware deny constraints. Mostly empty — see
/// sandbox::path::command_args.
fn top_cmd_args(cmd: &TopCmd) -> Vec<(String, String)> {
    use sandbox::Cmd;
    sandbox::command_args(match cmd {
        TopCmd::Auth { cmd } => Cmd::Auth(cmd),
        TopCmd::Config { cmd } => Cmd::Config(cmd),
        TopCmd::Profile { cmd } => Cmd::Profile(cmd),
        TopCmd::Balance { cmd } => Cmd::Balance(cmd),
        TopCmd::Quote { cmd } => Cmd::Quote(cmd),
        TopCmd::Recipient { cmd } => Cmd::Recipient(cmd),
        TopCmd::Transfer { cmd } => Cmd::Transfer(cmd),
        TopCmd::Card { cmd } => Cmd::Card(cmd),
        TopCmd::CardOrder { cmd } => Cmd::CardOrder(cmd),
        TopCmd::Webhook { cmd } => Cmd::Webhook(cmd),
        TopCmd::Rate { cmd } => Cmd::Rate(cmd),
        TopCmd::Activity { cmd } => Cmd::Activity(cmd),
        TopCmd::Currency { cmd } => Cmd::Currency(cmd),
        TopCmd::Docs { cmd } => Cmd::Docs(cmd),
        TopCmd::Simulate { cmd } => Cmd::Simulate(cmd),
        TopCmd::Jose { cmd } => Cmd::Jose(cmd),
        TopCmd::Sandbox { cmd } => Cmd::Sandbox(cmd),
        TopCmd::Agent { cmd } => Cmd::Agent(cmd),
    })
}

fn sandbox_args_json(args: &[(String, String)]) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    for (k, v) in args {
        m.insert(k.clone(), serde_json::Value::String(v.clone()));
    }
    serde_json::Value::Object(m)
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
