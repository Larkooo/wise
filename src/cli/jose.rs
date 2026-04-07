// `wise jose` — JWE debug commands.
//
// These exist for two reasons:
//   1. Verify our JWE module against real Wise infrastructure (you can fetch
//      the live Wise public key and encrypt to it).
//   2. Give human operators a way to manually round-trip a payload while
//      debugging the agent-card flow without touching real cards.
//
// Real card-touching code in `wise agent fetch` will use the same module
// directly via crate::client::jose, not via this CLI subcommand.

use anyhow::{Context as _, Result};
use clap::Subcommand;
use serde_json::{json, Value};

use crate::cli::Ctx;
use crate::client::jose;
use crate::output;

#[derive(Debug, Subcommand)]
pub enum JoseCmd {
    /// Fetch Wise's RSA public key for client-side encryption.
    /// Hits GET /twcard-data/v1/clientSideEncryption/fetchEncryptingKey.
    /// Optionally write the PEM to a file.
    FetchKey {
        /// Write the PEM to this file (otherwise printed to stdout).
        #[arg(long, short = 'o')]
        output: Option<String>,
    },
    /// Encrypt a JSON payload to a recipient public key, producing a compact
    /// JWE string. Algorithms: RSA-OAEP-256 + A256GCM (the only pair Wise uses).
    Encrypt {
        /// Path to the recipient PEM public key.
        #[arg(long)]
        key: String,
        /// JSON plaintext to encrypt. Use `--stdin` to read from stdin.
        #[arg(long, conflicts_with = "stdin")]
        plaintext: Option<String>,
        #[arg(long)]
        stdin: bool,
    },
    /// Decrypt a compact JWE with a private key. Useful for verifying that a
    /// JWE produced by `encrypt` round-trips correctly when you have both
    /// halves of the keypair.
    Decrypt {
        #[arg(long)]
        key: String,
        /// The compact JWE string. Use `--stdin` to read from stdin.
        #[arg(long, conflicts_with = "stdin")]
        jwe: Option<String>,
        #[arg(long)]
        stdin: bool,
    },
}

pub async fn run(cmd: JoseCmd, ctx: &Ctx) -> Result<()> {
    match cmd {
        JoseCmd::FetchKey { output: out_path } => {
            // The endpoint returns a JSON envelope around the PEM. Different
            // Wise tenants have returned slightly different shapes over time;
            // we accept either a top-level `pem`/`publicKey` field or fall
            // back to printing whatever we got so the operator can inspect.
            let raw: Value = ctx
                .client
                .get("/twcard-data/v1/clientSideEncryption/fetchEncryptingKey")
                .await
                .context("fetching Wise encryption key")?;

            let pem = raw
                .get("publicKey")
                .and_then(|v| v.as_str())
                .or_else(|| raw.get("pem").and_then(|v| v.as_str()))
                .or_else(|| raw.get("key").and_then(|v| v.as_str()));

            if let Some(pem_str) = pem {
                if let Some(path) = out_path {
                    std::fs::write(&path, pem_str)
                        .with_context(|| format!("writing PEM to {path}"))?;
                    output::print(
                        &json!({ "saved": true, "path": path, "bytes": pem_str.len() }),
                        ctx.output(),
                    );
                } else {
                    output::print(
                        &json!({
                            "pem": pem_str,
                            "raw": raw,
                        }),
                        ctx.output(),
                    );
                }
            } else {
                // Unknown shape — emit it untouched and let the operator
                // figure out which field holds the key.
                output::print(&raw, ctx.output());
            }
        }

        JoseCmd::Encrypt { key, plaintext, stdin } => {
            let pem = std::fs::read_to_string(&key)
                .with_context(|| format!("reading public key {key}"))?;
            let pub_key = jose::parse_public_pem(&pem)?;
            let plaintext_bytes = read_plaintext(plaintext, stdin)?;
            let jwe = jose::encrypt_compact(&plaintext_bytes, &pub_key)?;
            // Print the JWE on its own line. We deliberately do *not* wrap it
            // in JSON when in the default Json output mode — JWE strings are
            // already a single token, and a bare line is easier to pipe.
            println!("{jwe}");
        }

        JoseCmd::Decrypt { key, jwe, stdin } => {
            let pem = std::fs::read_to_string(&key)
                .with_context(|| format!("reading private key {key}"))?;
            let priv_key = jose::parse_private_pem(&pem)?;
            let jwe_str = if stdin {
                let mut s = String::new();
                std::io::stdin().read_line(&mut s)?;
                s.trim().to_string()
            } else {
                jwe.context("provide --jwe or --stdin")?
            };
            let plaintext = jose::decrypt_compact(&jwe_str, &priv_key)?;
            // Try to interpret as JSON; if so, print it through the usual
            // output formatter so --pretty works. Otherwise emit as a string.
            match serde_json::from_slice::<Value>(&plaintext) {
                Ok(v) => output::print(&v, ctx.output()),
                Err(_) => {
                    let s = String::from_utf8_lossy(&plaintext);
                    println!("{s}");
                }
            }
        }
    }
    Ok(())
}

fn read_plaintext(plaintext: Option<String>, stdin: bool) -> Result<Vec<u8>> {
    if stdin {
        use std::io::Read;
        let mut buf = Vec::new();
        std::io::stdin().read_to_end(&mut buf)?;
        Ok(buf)
    } else {
        let s = plaintext.context("provide --plaintext or --stdin")?;
        Ok(s.into_bytes())
    }
}
