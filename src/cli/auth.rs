// Authentication subcommands.
//
// `wise auth login` supports two modes:
//   1. Personal/user bearer token (`--token`) — most common, simplest path.
//   2. OAuth client credentials (`--client-id` + `--client-secret`) — exchanges
//      via POST /oauth/token grant_type=client_credentials and stores the
//      resulting access token. The user is responsible for re-running login
//      every 12 hours when it expires.

use anyhow::{Context as _, Result};
use clap::Subcommand;
use serde::Deserialize;
use serde_json::json;

use crate::cli::Ctx;
use crate::client::WiseClient;
use crate::config::{self, Env};
use crate::output;

#[derive(Debug, Subcommand)]
pub enum AuthCmd {
    /// Store an API token (or exchange client credentials for one).
    Login {
        /// Personal/user bearer token. If omitted, --client-id/--client-secret
        /// will be used to fetch a client_credentials token instead.
        #[arg(long, env = "WISE_LOGIN_TOKEN", hide_env_values = true)]
        token: Option<String>,
        #[arg(long, requires = "client_secret")]
        client_id: Option<String>,
        #[arg(long, requires = "client_id", hide_env_values = true)]
        client_secret: Option<String>,
        /// Read the token from stdin (one line) instead of an arg.
        #[arg(long)]
        stdin: bool,
    },
    /// Print the current auth state (env, token presence, /v1/me result).
    Status,
    /// Call /v1/me with the stored token.
    Whoami,
    /// Forget the stored token for the active env.
    Logout,
}

pub async fn run(cmd: AuthCmd, ctx: &Ctx) -> Result<()> {
    let env = ctx.client.env();
    match cmd {
        AuthCmd::Login {
            token,
            client_id,
            client_secret,
            stdin,
        } => login(ctx, env, token, client_id, client_secret, stdin).await,
        AuthCmd::Status => status(ctx, env).await,
        AuthCmd::Whoami => whoami(ctx).await,
        AuthCmd::Logout => logout(ctx, env).await,
    }
}

async fn login(
    ctx: &Ctx,
    env: Env,
    token: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    stdin: bool,
) -> Result<()> {
    let token = if stdin {
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        Some(s.trim().to_string())
    } else {
        token
    };

    let final_token = if let Some(t) = token {
        t
    } else if let (Some(id), Some(secret)) = (client_id, client_secret) {
        // Exchange client credentials for a 12-hour access token.
        // /oauth/token is unauthenticated except for HTTP Basic.
        let unauth = WiseClient::new(env, None)?;
        #[derive(Deserialize)]
        struct TokenResp {
            access_token: String,
            #[serde(default)]
            expires_in: Option<i64>,
            #[serde(default)]
            token_type: Option<String>,
        }
        let resp: TokenResp = unauth
            .post_form_basic(
                "/oauth/token",
                &[("grant_type", "client_credentials")],
                &id,
                &secret,
            )
            .await
            .context("exchanging client credentials")?;
        tracing::info!(
            "obtained {} token (expires_in={:?})",
            resp.token_type.as_deref().unwrap_or("Bearer"),
            resp.expires_in
        );
        resp.access_token
    } else {
        anyhow::bail!(
            "pass --token <T>, or --client-id/--client-secret, or use --stdin to provide a token"
        );
    };

    config::save_token(env, &final_token).context("saving token")?;
    output::print(
        &json!({
            "env": env.as_str(),
            "stored": true,
            "message": format!("token stored for env={}", env.as_str()),
        }),
        ctx.output(),
    );
    Ok(())
}

async fn status(ctx: &Ctx, env: Env) -> Result<()> {
    let has_token = ctx.client.has_token();
    let me = if has_token {
        match ctx.client.get::<serde_json::Value>("/v1/me").await {
            Ok(v) => Some(v),
            Err(e) => {
                tracing::debug!("/v1/me failed: {e}");
                None
            }
        }
    } else {
        None
    };
    output::print(
        &json!({
            "env": env.as_str(),
            "authenticated": has_token,
            "me": me,
        }),
        ctx.output(),
    );
    Ok(())
}

async fn whoami(ctx: &Ctx) -> Result<()> {
    let v: serde_json::Value = ctx.client.get("/v1/me").await?;
    output::print(&v, ctx.output());
    Ok(())
}

async fn logout(ctx: &Ctx, env: Env) -> Result<()> {
    config::delete_token(env)?;
    output::print(
        &json!({
            "env": env.as_str(),
            "logged_out": true,
        }),
        ctx.output(),
    );
    Ok(())
}
