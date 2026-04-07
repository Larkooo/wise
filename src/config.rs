// Persistent configuration + credential storage.
//
// Config is stored as TOML at $XDG_CONFIG_HOME/wise/config.toml. Tokens are
// stored in the OS keychain via the `keyring` crate (entry per env), with a
// plaintext fallback at credentials.toml (mode 0600) for systems where the
// keychain isn't available (e.g. headless CI).

use anyhow::{Context as _, Result};
use clap::ValueEnum;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const KEYRING_SERVICE: &str = "wise-cli";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    pub env: Option<Env>,
    pub default_profile: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Env {
    Sandbox,
    Production,
}

impl Env {
    pub fn as_str(&self) -> &'static str {
        match self {
            Env::Sandbox => "sandbox",
            Env::Production => "production",
        }
    }
}

impl std::fmt::Display for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Self::path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let s = fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg = toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let s = toml::to_string_pretty(self).context("serializing config")?;
        fs::write(&path, s).with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    pub fn path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("com", "wise", "wise")
            .ok_or_else(|| anyhow::anyhow!("could not resolve config directory"))?;
        Ok(dirs.config_dir().join("config.toml"))
    }
}

/// Store the API token for an environment.
pub fn save_token(env: Env, token: &str) -> Result<()> {
    match keyring::Entry::new(KEYRING_SERVICE, env.as_str()) {
        Ok(entry) => match entry.set_password(token) {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::debug!("keyring set failed, falling back to file: {e}");
            }
        },
        Err(e) => {
            tracing::debug!("keyring entry construction failed, falling back to file: {e}");
        }
    }
    save_token_file(env, token)
}

/// Load the API token for an environment.
pub fn load_token(env: Env) -> Result<String> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, env.as_str()) {
        if let Ok(pw) = entry.get_password() {
            if !pw.is_empty() {
                return Ok(pw);
            }
        }
    }
    load_token_file(env)
}

/// Delete the API token for an environment.
pub fn delete_token(env: Env) -> Result<()> {
    let mut keyring_err: Option<String> = None;
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, env.as_str()) {
        if let Err(e) = entry.delete_credential() {
            keyring_err = Some(e.to_string());
        }
    }
    let _ = delete_token_file(env);
    // Treat "no such entry" as success.
    if let Some(e) = keyring_err {
        if !e.contains("No matching") && !e.contains("not found") && !e.contains("NoEntry") {
            tracing::debug!("keyring delete error: {e}");
        }
    }
    Ok(())
}

fn credentials_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "wise", "wise")
        .ok_or_else(|| anyhow::anyhow!("could not resolve config directory"))?;
    Ok(dirs.config_dir().join("credentials.toml"))
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CredentialsFile {
    sandbox: Option<String>,
    production: Option<String>,
}

fn read_creds() -> Result<CredentialsFile> {
    let path = credentials_path()?;
    if !path.exists() {
        return Ok(CredentialsFile::default());
    }
    let s = fs::read_to_string(&path)?;
    Ok(toml::from_str(&s).unwrap_or_default())
}

fn write_creds(creds: &CredentialsFile) -> Result<()> {
    let path = credentials_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let s = toml::to_string_pretty(creds)?;
    fs::write(&path, s)?;
    set_secure_perms(&path)?;
    Ok(())
}

fn save_token_file(env: Env, token: &str) -> Result<()> {
    let mut creds = read_creds()?;
    match env {
        Env::Sandbox => creds.sandbox = Some(token.to_string()),
        Env::Production => creds.production = Some(token.to_string()),
    }
    write_creds(&creds)
}

fn load_token_file(env: Env) -> Result<String> {
    let creds = read_creds()?;
    let tok = match env {
        Env::Sandbox => creds.sandbox,
        Env::Production => creds.production,
    };
    tok.ok_or_else(|| anyhow::anyhow!("no token stored for env={env}"))
}

fn delete_token_file(env: Env) -> Result<()> {
    let mut creds = read_creds()?;
    match env {
        Env::Sandbox => creds.sandbox = None,
        Env::Production => creds.production = None,
    }
    if creds.sandbox.is_none() && creds.production.is_none() {
        let path = credentials_path()?;
        let _ = fs::remove_file(path);
        Ok(())
    } else {
        write_creds(&creds)
    }
}

#[cfg(unix)]
fn set_secure_perms(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, perms)?;
    Ok(())
}

#[cfg(not(unix))]
fn set_secure_perms(_path: &Path) -> Result<()> {
    Ok(())
}
