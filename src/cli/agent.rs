// `wise agent …` — manual-paste agent card flow.
//
// This is the practical "agent has its own card" feature that ships today,
// without requiring partner-tier OAuth credentials. The user issues a Wise
// card on wise.com (because personal API tokens cannot create cards), then:
//
//     wise agent init <name>            scaffolds the sandbox + spend caps
//     wise agent paste --sandbox <name>  one-time PAN/CVV/expiry entry
//     wise agent status --sandbox <name> shows masked PAN, last fetch
//     wise agent fetch --sandbox <name> --justify "..."
//                                       returns full card details (audited)
//     wise agent rotate --sandbox <name> wipe + re-paste flow
//     wise agent panic --sandbox <name>  wipe immediately, no questions

use anyhow::{bail, Context as _, Result};
use chrono::Utc;
use clap::Subcommand;
use serde_json::json;
use std::collections::HashMap;
use std::io::Write;

use crate::agent::{self, StoredCard};
use crate::cli::Ctx;
use crate::output;
use crate::sandbox::{
    policy::{Conditions, Escalation, EscalationMode, Policy},
    Sandbox,
};

#[derive(Debug, Subcommand)]
pub enum AgentCmd {
    /// Scaffold a new agent sandbox + spend caps. Does NOT create a card —
    /// the user must issue one on wise.com first because personal API
    /// tokens cannot reach the card-creation endpoints (see AGENT.md).
    Init {
        /// Sandbox name (also the keychain entry name for the card).
        name: String,
        /// Wise profile id this agent will run under.
        #[arg(long)]
        profile: i64,
        /// Optional Wise card token (metadata only — used in the sandbox
        /// `cards = [...]` allow-list to scope card endpoints).
        #[arg(long)]
        card_token: Option<String>,
        /// Per-fetch rate limit. Default 5/hour.
        #[arg(long, default_value = "5/hour")]
        rate_limit: String,
        /// Allow `wise agent fetch` calls in this sandbox? Default true.
        #[arg(long, default_value_t = true)]
        allow_fetch: bool,
        /// Overwrite an existing sandbox file with the same name.
        #[arg(long)]
        force: bool,
    },

    /// One-time PAN/CVV/expiry/cardholder entry. Reads PAN and CVV from
    /// stdin without echoing. Validates Luhn + expiry + length before
    /// storing in the keychain.
    Paste {
        /// Sandbox to associate with the stored card.
        #[arg(long)]
        sandbox: String,
        /// Optional Wise card token to record alongside the PAN.
        #[arg(long)]
        card_token: Option<String>,
        /// Replace an existing stored card.
        #[arg(long)]
        replace: bool,
    },

    /// Show whether a card is stored, the masked PAN, and recent audit
    /// activity. Does NOT reveal the full PAN.
    Status {
        #[arg(long)]
        sandbox: String,
    },

    /// Retrieve the full stored card. This is the only command an agent
    /// process actually calls — must run inside an active sandbox (the
    /// sandbox name is derived from `--sandbox` / `WISE_SANDBOX` on the
    /// global args, not a per-command flag, so an agent in sandbox A
    /// cannot pass a different name to read sandbox B's card). Sandbox
    /// enforcement (rate limit, justify, audit) happens upstream in
    /// `main.rs::dispatch`.
    Fetch {
        /// Mask the CVV in the output (returns `***`). Useful for testing
        /// without exposing the real value.
        #[arg(long)]
        mask_cvv: bool,
    },

    /// Wipe the stored card. Use this when rotating to a new physical card
    /// or after testing. Does not touch the sandbox file.
    Rotate {
        #[arg(long)]
        sandbox: String,
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Emergency wipe + audit incident line. No confirmation, exits 0
    /// even if nothing was stored.
    Panic {
        #[arg(long)]
        sandbox: String,
    },
}

pub async fn run(cmd: AgentCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        AgentCmd::Init {
            name,
            profile,
            card_token,
            rate_limit,
            allow_fetch,
            force,
        } => init(ctx, name, profile, card_token, rate_limit, allow_fetch, force),

        AgentCmd::Paste {
            sandbox,
            card_token,
            replace,
        } => paste(ctx, sandbox, card_token, replace),

        AgentCmd::Status { sandbox } => status(ctx, sandbox),

        AgentCmd::Fetch { mask_cvv } => fetch(ctx, mask_cvv),

        AgentCmd::Rotate { sandbox, yes } => rotate(ctx, sandbox, yes),

        AgentCmd::Panic { sandbox } => panic_wipe(ctx, sandbox),
    }
}

// ---------- init ----------

fn init(
    ctx: &Ctx,
    name: String,
    profile: i64,
    card_token: Option<String>,
    rate_limit: String,
    allow_fetch: bool,
    force: bool,
) -> Result<()> {
    let mut allow = vec![
        "balance.list".to_string(),
        "balance.get".to_string(),
        "card.get".to_string(),
        "card.freeze".to_string(),
        "rate.get".to_string(),
        "currency.list".to_string(),
        "docs.ask".to_string(),
        "agent.status".to_string(),
    ];
    if allow_fetch {
        allow.push("agent.fetch".to_string());
    }

    // Always deny the management surface so an agent can't rewrite its
    // own credentials by escaping into agent.paste/init/rotate/panic.
    let deny = vec![
        "agent.init".to_string(),
        "agent.paste".to_string(),
        "agent.rotate".to_string(),
        "agent.panic".to_string(),
        "card.unfreeze".to_string(),
        "card.permissions.set".to_string(),
        "transfer.*".to_string(),
        "balance.move".to_string(),
        "balance.delete".to_string(),
        "balance.create".to_string(),
        "balance.topup".to_string(),
        "recipient.create".to_string(),
        "recipient.delete".to_string(),
    ];

    let audit_path = Sandbox::sandboxes_dir()?.join(format!("{name}.audit.jsonl"));

    let mut conditions: HashMap<String, Conditions> = HashMap::new();
    if allow_fetch {
        conditions.insert(
            "agent.fetch".to_string(),
            Conditions {
                rate_limit: Some(rate_limit.clone()),
                require_justification: true,
                audit: Some(audit_path.clone()),
            },
        );
    }

    let policy = Policy {
        name: name.clone(),
        description: Some(format!("Agent card scope for `{name}`")),
        profiles: Some(vec![profile]),
        cards: card_token.clone().map(|t| vec![t]),
        balances: None,
        allow,
        deny,
        conditions,
        escalation: Escalation {
            mode: EscalationMode::Deny,
            command: None,
            timeout: None,
        },
    };
    policy.validate()?;
    let sb = Sandbox {
        policy,
        source: Sandbox::path_for(&name)?,
    };
    sb.save(force)?;

    let next_steps = format!(
        "Next steps:\n  \
         1. Issue a Wise virtual card on wise.com (the API path requires partner OAuth).\n  \
         2. Run `wise agent paste --sandbox {name}` to enter PAN/CVV/expiry.\n  \
         3. Set `WISE_SANDBOX={name}` in the agent's environment.\n  \
         4. The agent calls `wise agent fetch --sandbox {name} --justify \"...\"`."
    );
    output::print(
        &json!({
            "created": true,
            "sandbox": name,
            "policy_path": sb.source,
            "audit_path": audit_path,
            "card_token": card_token,
            "next_steps": next_steps,
        }),
        ctx.output(),
    );
    Ok(())
}

// ---------- paste ----------

fn paste(
    ctx: &Ctx,
    sandbox_name: String,
    card_token: Option<String>,
    replace: bool,
) -> Result<()> {
    // Sanity-check the sandbox exists before asking for sensitive input —
    // we don't want the user to paste a PAN and only then learn the
    // sandbox name was wrong.
    let _ = Sandbox::load(&sandbox_name)
        .with_context(|| format!("sandbox '{sandbox_name}' must exist before paste"))?;

    println!("Paste card details for sandbox '{sandbox_name}'.");
    println!("PAN and CVV are read from stdin without echo.");
    println!();

    let pan = read_secret("PAN")?;
    agent::validate_pan(&pan).context("PAN validation")?;

    let cvv = read_secret("CVV")?;
    agent::validate_cvv(&cvv).context("CVV validation")?;

    let expiry = prompt("Expiry (MM/YYYY)")?;
    let (month, year) = parse_expiry(&expiry)?;
    agent::validate_expiry(month, year).context("expiry validation")?;

    let cardholder = prompt("Cardholder name (as on card)")?;
    if cardholder.trim().is_empty() {
        bail!("cardholder name is required");
    }

    let card = StoredCard {
        pan: pan.chars().filter(|c| c.is_ascii_digit()).collect(),
        cvv,
        expiry_month: month,
        expiry_year: year,
        cardholder_name: cardholder.trim().to_string(),
        card_token,
        stored_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    };
    agent::store(&sandbox_name, &card, replace)?;

    output::print(
        &json!({
            "stored": true,
            "sandbox": sandbox_name,
            "pan_masked": card.pan_masked(),
            "expiry": format!("{:02}/{}", card.expiry_month, card.expiry_year),
        }),
        ctx.output(),
    );
    Ok(())
}

// ---------- status ----------

fn status(ctx: &Ctx, sandbox_name: String) -> Result<()> {
    let sb = Sandbox::load(&sandbox_name)?;
    let stored = agent::exists(&sandbox_name);
    let card_summary = if stored {
        let card = agent::load(&sandbox_name)?;
        Some(json!({
            "pan_masked": card.pan_masked(),
            "expiry": format!("{:02}/{}", card.expiry_month, card.expiry_year),
            "cardholder_name": card.cardholder_name,
            "card_token": card.card_token,
            "stored_at": card.stored_at,
        }))
    } else {
        None
    };
    output::print(
        &json!({
            "sandbox": sandbox_name,
            "policy_path": sb.source,
            "stored": stored,
            "card": card_summary,
            "profiles": sb.policy.profiles,
            "cards": sb.policy.cards,
            "rate_limit": sb.policy
                .conditions
                .get("agent.fetch")
                .and_then(|c| c.rate_limit.clone()),
        }),
        ctx.output(),
    );
    Ok(())
}

// ---------- fetch ----------

fn fetch(ctx: &Ctx, mask_cvv: bool) -> Result<()> {
    // The dispatch gate has already enforced sandbox conditions
    // (rate_limit + require_justification + audit) by the time we get
    // here. We derive the sandbox name from the active context — there
    // is deliberately no per-command --sandbox flag so an agent in
    // sandbox A cannot use it to read sandbox B's card.
    let active = ctx
        .sandbox
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!(
            "agent fetch must be invoked inside an active sandbox \
             (set WISE_SANDBOX=<name> or pass --sandbox <name>)"
        ))?;
    let sandbox_name = active.name().to_string();

    let card = agent::load(&sandbox_name)
        .with_context(|| format!("loading card for sandbox '{sandbox_name}'"))?;

    let cvv_out = if mask_cvv { "***".to_string() } else { card.cvv.clone() };
    output::print(
        &json!({
            "sandbox": sandbox_name,
            "pan": card.pan,
            "cvv": cvv_out,
            "expiry_month": card.expiry_month,
            "expiry_year": card.expiry_year,
            "cardholder_name": card.cardholder_name,
            "card_token": card.card_token,
        }),
        ctx.output(),
    );
    Ok(())
}

// ---------- rotate / panic ----------

fn rotate(ctx: &Ctx, sandbox_name: String, yes: bool) -> Result<()> {
    if !yes {
        bail!(
            "refusing to wipe stored card for '{sandbox_name}' without --yes (or -y)"
        );
    }
    agent::delete(&sandbox_name)?;
    output::print(
        &json!({
            "rotated": true,
            "sandbox": sandbox_name,
            "next": format!("re-paste with `wise agent paste --sandbox {sandbox_name}`"),
        }),
        ctx.output(),
    );
    Ok(())
}

fn panic_wipe(ctx: &Ctx, sandbox_name: String) -> Result<()> {
    let _ = agent::delete(&sandbox_name);
    eprintln!("[panic] wiped agent card for sandbox '{sandbox_name}'");
    output::print(
        &json!({
            "panicked": true,
            "sandbox": sandbox_name,
            "wiped": true,
            "note": "card storage cleared. Card is NOT frozen on Wise's side — \
                    do that manually via the Wise app or partner OAuth.",
        }),
        ctx.output(),
    );
    Ok(())
}

// ---------- helpers ----------

fn read_secret(prompt_text: &str) -> Result<String> {
    let s = rpassword::prompt_password(format!("{prompt_text}: "))
        .with_context(|| format!("reading {prompt_text} from stdin"))?;
    if s.trim().is_empty() {
        bail!("{prompt_text} cannot be empty");
    }
    Ok(s.trim().to_string())
}

fn prompt(prompt_text: &str) -> Result<String> {
    print!("{prompt_text}: ");
    std::io::stdout().flush().ok();
    let mut s = String::new();
    std::io::stdin().read_line(&mut s)?;
    Ok(s.trim().to_string())
}

fn parse_expiry(s: &str) -> Result<(u8, u16)> {
    let (m, y) = s
        .split_once('/')
        .with_context(|| format!("expiry must look like MM/YYYY, got `{s}`"))?;
    let month: u8 = m.trim().parse().context("expiry month must be a number")?;
    let year_str = y.trim();
    let year: u16 = if year_str.len() == 2 {
        // Be helpful: 2-digit YY → 2000+YY rather than rejecting outright.
        let yy: u16 = year_str.parse().context("expiry year must be numeric")?;
        2000 + yy
    } else {
        year_str.parse().context("expiry year must be numeric")?
    };
    Ok((month, year))
}
