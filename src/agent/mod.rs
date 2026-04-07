// Local-only agent card storage.
//
// This module backs the manual-paste agent flow described in AGENT.md as
// "Option C". The user issues a Wise card on wise.com (because personal
// API tokens cannot create cards), then runs `wise agent paste` once. The
// PAN/CVV/expiry/cardholder are validated, packed as JSON, and stored in
// the OS keychain under a per-sandbox entry. `wise agent fetch` retrieves
// them on demand under sandbox audit + rate limit + justification gates.
//
// The keychain is the only encryption layer. We deliberately do *not*
// add a separate JWE wrapper on top — that would put both halves of any
// asymmetric scheme in the same keystore, which is illusory defense.
// macOS Keychain / Linux Secret Service / Windows Credential Manager are
// already encrypted with the user's login credentials, which is the
// strongest local boundary available without requiring a separate
// passphrase from the user every time.

use anyhow::{bail, Context as _, Result};
use chrono::{Datelike, Utc};
use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "wise-agent-card";

/// Stored card details, written to and read from the OS keychain as JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCard {
    pub pan: String,
    pub cvv: String,
    pub expiry_month: u8,
    pub expiry_year: u16,
    pub cardholder_name: String,
    pub card_token: Option<String>,
    pub stored_at: String,
}

impl StoredCard {
    /// Validate every field. Run before storage and again on retrieval so
    /// a corrupted keychain entry is caught at fetch time, not by the
    /// downstream consumer.
    pub fn validate(&self) -> Result<()> {
        validate_pan(&self.pan)?;
        validate_cvv(&self.cvv)?;
        validate_expiry(self.expiry_month, self.expiry_year)?;
        if self.cardholder_name.trim().is_empty() {
            bail!("cardholder_name is empty");
        }
        Ok(())
    }

    /// PAN with the middle digits replaced by `*`. Used in `wise agent
    /// status` so the human can sanity-check which card is stored without
    /// exposing the full number.
    pub fn pan_masked(&self) -> String {
        let n = self.pan.len();
        if n <= 8 {
            return "*".repeat(n);
        }
        let head = &self.pan[..4];
        let tail = &self.pan[n - 4..];
        let middle = "*".repeat(n - 8);
        format!("{head}{middle}{tail}")
    }
}

/// Save a card to the keychain under the per-sandbox entry. Refuses to
/// overwrite an existing entry unless `replace = true`.
pub fn store(sandbox: &str, card: &StoredCard, replace: bool) -> Result<()> {
    card.validate().context("validating card before storage")?;
    let entry = keyring::Entry::new(KEYRING_SERVICE, sandbox)
        .with_context(|| format!("opening keyring entry {KEYRING_SERVICE}/{sandbox}"))?;
    if !replace && entry.get_password().is_ok() {
        bail!(
            "agent card already stored for sandbox '{sandbox}' — \
             pass --replace to overwrite, or `wise agent rotate` to clear it first"
        );
    }
    let body = serde_json::to_string(card).context("serializing card for storage")?;
    entry
        .set_password(&body)
        .with_context(|| format!("writing keyring entry {KEYRING_SERVICE}/{sandbox}"))?;
    Ok(())
}

/// Load a card from the keychain. Re-validates the JSON before returning
/// so a tampered entry is caught here.
pub fn load(sandbox: &str) -> Result<StoredCard> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, sandbox)
        .with_context(|| format!("opening keyring entry {KEYRING_SERVICE}/{sandbox}"))?;
    let body = entry
        .get_password()
        .with_context(|| format!("no card stored for sandbox '{sandbox}'"))?;
    let card: StoredCard = serde_json::from_str(&body).context("parsing stored card JSON")?;
    card.validate().context("validating stored card")?;
    Ok(card)
}

/// Delete the keychain entry for `sandbox`. Returns Ok even if the entry
/// did not exist (idempotent — `wise agent panic` should never error).
pub fn delete(sandbox: &str) -> Result<()> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, sandbox) {
        let _ = entry.delete_credential();
    }
    Ok(())
}

/// True iff a card is stored for the given sandbox.
pub fn exists(sandbox: &str) -> bool {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, sandbox) {
        return entry.get_password().is_ok();
    }
    false
}

// ---------- input validation ----------

/// PAN must be 13–19 digits and pass the Luhn check.
pub fn validate_pan(pan: &str) -> Result<()> {
    let digits: Vec<u32> = pan
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .filter_map(|c| c.to_digit(10))
        .collect();
    if digits.len() < 13 || digits.len() > 19 {
        bail!("PAN must be 13-19 digits, got {}", digits.len());
    }
    if pan.chars().any(|c| !c.is_ascii_digit() && !c.is_whitespace() && c != '-') {
        bail!("PAN contains non-digit characters");
    }
    if !luhn_valid(&digits) {
        bail!("PAN failed Luhn checksum — re-check your input");
    }
    Ok(())
}

fn luhn_valid(digits: &[u32]) -> bool {
    let mut sum = 0u32;
    let mut alt = false;
    for d in digits.iter().rev() {
        let mut n = *d;
        if alt {
            n *= 2;
            if n > 9 {
                n -= 9;
            }
        }
        sum += n;
        alt = !alt;
    }
    sum % 10 == 0
}

pub fn validate_cvv(cvv: &str) -> Result<()> {
    if cvv.len() < 3 || cvv.len() > 4 {
        bail!("CVV must be 3 or 4 digits, got {}", cvv.len());
    }
    if !cvv.chars().all(|c| c.is_ascii_digit()) {
        bail!("CVV must be all digits");
    }
    Ok(())
}

pub fn validate_expiry(month: u8, year: u16) -> Result<()> {
    if !(1..=12).contains(&month) {
        bail!("expiry month must be 1-12, got {month}");
    }
    if year < 100 {
        bail!(
            "expiry year must be a full 4-digit year (e.g. 2028), got {year} — \
             pass YY as 20YY"
        );
    }
    let now = Utc::now();
    let cur_year = now.year() as u16;
    let cur_month = now.month() as u8;
    if year < cur_year || (year == cur_year && month < cur_month) {
        bail!("expiry {month:02}/{year} is in the past");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luhn_accepts_known_test_pan() {
        // Visa test PAN — passes Luhn.
        validate_pan("4111111111111111").unwrap();
    }

    #[test]
    fn luhn_rejects_bad_check_digit() {
        let err = validate_pan("4111111111111112").unwrap_err();
        assert!(err.to_string().contains("Luhn"));
    }

    #[test]
    fn pan_strips_whitespace_and_dashes() {
        validate_pan("4111-1111-1111 1111").unwrap();
    }

    #[test]
    fn pan_rejects_letters() {
        let err = validate_pan("4111111111111ABC").unwrap_err();
        assert!(err.to_string().contains("non-digit"));
    }

    #[test]
    fn pan_rejects_too_short() {
        let err = validate_pan("411111").unwrap_err();
        assert!(err.to_string().contains("13-19"));
    }

    #[test]
    fn cvv_rejects_two_digits() {
        assert!(validate_cvv("12").is_err());
    }

    #[test]
    fn cvv_accepts_three_and_four() {
        assert!(validate_cvv("123").is_ok());
        assert!(validate_cvv("1234").is_ok());
    }

    #[test]
    fn expiry_rejects_past() {
        // 1999 is unambiguously in the past.
        let err = validate_expiry(1, 1999).unwrap_err();
        assert!(err.to_string().contains("past"));
    }

    #[test]
    fn expiry_rejects_2_digit_year() {
        let err = validate_expiry(12, 28).unwrap_err();
        assert!(err.to_string().contains("4-digit"));
    }

    #[test]
    fn pan_masked_keeps_first_and_last_4() {
        let card = StoredCard {
            pan: "4111111111111111".into(),
            cvv: "123".into(),
            expiry_month: 12,
            expiry_year: 2030,
            cardholder_name: "Test".into(),
            card_token: None,
            stored_at: "now".into(),
        };
        assert_eq!(card.pan_masked(), "4111********1111");
    }
}
