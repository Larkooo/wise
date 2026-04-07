// Command path derivation.
//
// Maps a parsed clap command tree to a dot-separated path string used by
// the sandbox dispatch gate. Every command in the CLI must have exactly
// one canonical path string here — that's how the policy file refers to
// it. Adding a new command means adding a match arm here, which is the
// intentional friction: you cannot ship a command without explicitly
// deciding what its sandbox path is.

use crate::cli::activity::ActivityCmd;
use crate::cli::agent::AgentCmd;
use crate::cli::auth::AuthCmd;
use crate::cli::balance::BalanceCmd;
use crate::cli::card::{CardCmd, PermissionsCmd};
use crate::cli::card_order::CardOrderCmd;
use crate::cli::config_cmd::ConfigCmd;
use crate::cli::currency::CurrencyCmd;
use crate::cli::docs::DocsCmd;
use crate::cli::jose::JoseCmd;
use crate::cli::profile::ProfileCmd;
use crate::cli::quote::QuoteCmd;
use crate::cli::rate::RateCmd;
use crate::cli::recipient::RecipientCmd;
use crate::cli::sandbox::SandboxCmd;
use crate::cli::simulate::SimulateCmd;
use crate::cli::transfer::TransferCmd;
use crate::cli::webhook::WebhookCmd;

/// Top-level alias matching `crate::TopCmd` so this module can be referenced
/// from the crate root without circular use chains.
pub enum Cmd<'a> {
    Auth(&'a AuthCmd),
    Config(&'a ConfigCmd),
    Profile(&'a ProfileCmd),
    Balance(&'a BalanceCmd),
    Quote(&'a QuoteCmd),
    Recipient(&'a RecipientCmd),
    Transfer(&'a TransferCmd),
    Card(&'a CardCmd),
    CardOrder(&'a CardOrderCmd),
    Webhook(&'a WebhookCmd),
    Rate(&'a RateCmd),
    Activity(&'a ActivityCmd),
    Currency(&'a CurrencyCmd),
    Docs(&'a DocsCmd),
    Simulate(&'a SimulateCmd),
    Jose(&'a JoseCmd),
    Sandbox(&'a SandboxCmd),
    Agent(&'a AgentCmd),
}

/// Render a command into its canonical dot path. Stable strings — they
/// appear verbatim in user-edited TOML files, so renaming is a breaking
/// change.
pub fn command_path(cmd: Cmd<'_>) -> String {
    match cmd {
        Cmd::Auth(c) => format!("auth.{}", auth_leaf(c)),
        Cmd::Config(c) => format!("config.{}", config_leaf(c)),
        Cmd::Profile(c) => format!("profile.{}", profile_leaf(c)),
        Cmd::Balance(c) => format!("balance.{}", balance_leaf(c)),
        Cmd::Quote(c) => format!("quote.{}", quote_leaf(c)),
        Cmd::Recipient(c) => format!("recipient.{}", recipient_leaf(c)),
        Cmd::Transfer(c) => format!("transfer.{}", transfer_leaf(c)),
        Cmd::Card(c) => format!("card.{}", card_leaf(c)),
        Cmd::CardOrder(c) => format!("card-order.{}", card_order_leaf(c)),
        Cmd::Webhook(c) => format!("webhook.{}", webhook_leaf(c)),
        Cmd::Rate(c) => format!("rate.{}", rate_leaf(c)),
        Cmd::Activity(c) => format!("activity.{}", activity_leaf(c)),
        Cmd::Currency(c) => format!("currency.{}", currency_leaf(c)),
        Cmd::Docs(c) => format!("docs.{}", docs_leaf(c)),
        Cmd::Simulate(c) => format!("simulate.{}", simulate_leaf(c)),
        Cmd::Jose(c) => format!("jose.{}", jose_leaf(c)),
        Cmd::Sandbox(c) => format!("sandbox.{}", sandbox_leaf(c)),
        Cmd::Agent(c) => format!("agent.{}", agent_leaf(c)),
    }
}

/// Argument-aware constraints. Each command exposes the (key, value) pairs
/// that the sandbox `path:key=value` deny syntax can match against. We only
/// surface the few arguments where deny constraints make sense — currently
/// `card.status --status` for the freeze-only-not-unfreeze use case.
pub fn command_args(cmd: Cmd<'_>) -> Vec<(String, String)> {
    match cmd {
        Cmd::Card(CardCmd::Status { status, .. }) => {
            vec![("status".into(), format!("{:?}", status).to_uppercase())]
        }
        _ => Vec::new(),
    }
}

// ---------- per-group leaves ----------

fn auth_leaf(c: &AuthCmd) -> &'static str {
    match c {
        AuthCmd::Login { .. } => "login",
        AuthCmd::Status => "status",
        AuthCmd::Whoami => "whoami",
        AuthCmd::Logout => "logout",
    }
}

fn config_leaf(c: &ConfigCmd) -> &'static str {
    match c {
        ConfigCmd::Get { .. } => "get",
        ConfigCmd::Set { .. } => "set",
        ConfigCmd::List => "list",
        ConfigCmd::Path => "path",
    }
}

fn profile_leaf(c: &ProfileCmd) -> &'static str {
    match c {
        ProfileCmd::List => "list",
        ProfileCmd::Get { .. } => "get",
        ProfileCmd::Current => "current",
    }
}

fn balance_leaf(c: &BalanceCmd) -> &'static str {
    match c {
        BalanceCmd::List { .. } => "list",
        BalanceCmd::Get { .. } => "get",
        BalanceCmd::Create { .. } => "create",
        BalanceCmd::Delete { .. } => "delete",
        BalanceCmd::Move { .. } => "move",
        BalanceCmd::Topup { .. } => "topup",
        BalanceCmd::Total { .. } => "total",
    }
}

fn quote_leaf(c: &QuoteCmd) -> &'static str {
    match c {
        QuoteCmd::Create { .. } => "create",
        QuoteCmd::Example { .. } => "example",
        QuoteCmd::Get { .. } => "get",
        QuoteCmd::Update { .. } => "update",
    }
}

fn recipient_leaf(c: &RecipientCmd) -> &'static str {
    match c {
        RecipientCmd::List { .. } => "list",
        RecipientCmd::Create { .. } => "create",
        RecipientCmd::Get { .. } => "get",
        RecipientCmd::Delete { .. } => "delete",
        RecipientCmd::Requirements { .. } => "requirements",
    }
}

fn transfer_leaf(c: &TransferCmd) -> &'static str {
    match c {
        TransferCmd::Create { .. } => "create",
        TransferCmd::List { .. } => "list",
        TransferCmd::Get { .. } => "get",
        TransferCmd::Cancel { .. } => "cancel",
        TransferCmd::Fund { .. } => "fund",
        TransferCmd::Requirements { .. } => "requirements",
        TransferCmd::Payments { .. } => "payments",
        TransferCmd::Receipt { .. } => "receipt",
    }
}

fn card_leaf(c: &CardCmd) -> String {
    match c {
        CardCmd::List { .. } => "list".into(),
        CardCmd::Get { .. } => "get".into(),
        CardCmd::Status { .. } => "status".into(),
        CardCmd::ResetPinCount { .. } => "reset-pin-count".into(),
        CardCmd::Permissions { cmd } => format!("permissions.{}", permissions_leaf(cmd)),
    }
}

fn permissions_leaf(c: &PermissionsCmd) -> &'static str {
    match c {
        PermissionsCmd::Get { .. } => "get",
        PermissionsCmd::Set { .. } => "set",
    }
}

fn card_order_leaf(c: &CardOrderCmd) -> &'static str {
    match c {
        CardOrderCmd::Programs { .. } => "programs",
        CardOrderCmd::Create { .. } => "create",
        CardOrderCmd::List { .. } => "list",
        CardOrderCmd::Get { .. } => "get",
        CardOrderCmd::Requirements { .. } => "requirements",
        CardOrderCmd::Cancel { .. } => "cancel",
    }
}

fn webhook_leaf(c: &WebhookCmd) -> &'static str {
    match c {
        WebhookCmd::List { .. } => "list",
        WebhookCmd::Get { .. } => "get",
        WebhookCmd::Create { .. } => "create",
        WebhookCmd::Delete { .. } => "delete",
        WebhookCmd::Test { .. } => "test",
    }
}

fn rate_leaf(c: &RateCmd) -> &'static str {
    match c {
        RateCmd::Get { .. } => "get",
        RateCmd::History { .. } => "history",
    }
}

fn activity_leaf(c: &ActivityCmd) -> &'static str {
    match c {
        ActivityCmd::List { .. } => "list",
    }
}

fn currency_leaf(c: &CurrencyCmd) -> &'static str {
    match c {
        CurrencyCmd::List => "list",
    }
}

fn docs_leaf(c: &DocsCmd) -> &'static str {
    match c {
        DocsCmd::Ask { .. } => "ask",
    }
}

fn simulate_leaf(c: &SimulateCmd) -> &'static str {
    match c {
        SimulateCmd::TransferState { .. } => "transfer-state",
        SimulateCmd::BalanceTopup { .. } => "balance-topup",
        SimulateCmd::VerifyProfile { .. } => "verify-profile",
        SimulateCmd::VerifyAll => "verify-all",
        SimulateCmd::BankTx { .. } => "bank-tx",
        SimulateCmd::SwiftIn { .. } => "swift-in",
        SimulateCmd::CardAuth { .. } => "card-auth",
        SimulateCmd::CardClearing { .. } => "card-clearing",
        SimulateCmd::CardReversal { .. } => "card-reversal",
    }
}

fn jose_leaf(c: &JoseCmd) -> &'static str {
    match c {
        JoseCmd::FetchKey { .. } => "fetch-key",
        JoseCmd::Encrypt { .. } => "encrypt",
        JoseCmd::Decrypt { .. } => "decrypt",
    }
}

fn sandbox_leaf(c: &SandboxCmd) -> &'static str {
    match c {
        SandboxCmd::New { .. } => "new",
        SandboxCmd::List => "list",
        SandboxCmd::Show { .. } => "show",
        SandboxCmd::Edit { .. } => "edit",
        SandboxCmd::Delete { .. } => "delete",
        SandboxCmd::Check { .. } => "check",
        SandboxCmd::Shell { .. } => "shell",
        SandboxCmd::Audit { .. } => "audit",
    }
}

fn agent_leaf(c: &AgentCmd) -> &'static str {
    match c {
        AgentCmd::Init { .. } => "init",
        AgentCmd::Paste { .. } => "paste",
        AgentCmd::Status { .. } => "status",
        AgentCmd::Fetch { .. } => "fetch",
        AgentCmd::Rotate { .. } => "rotate",
        AgentCmd::Panic { .. } => "panic",
    }
}
