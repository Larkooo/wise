// JWE (JSON Web Encryption) for the two algorithms Wise actually uses on the
// sensitive-card-details and SCA endpoints:
//
//     alg = RSA-OAEP-256   (key wrapping with SHA-256)
//     enc = A256GCM        (AES-256-GCM content encryption)
//
// We deliberately do *not* implement a general-purpose JOSE crate. There are
// dozens of footguns in the broader spec (algorithm confusion, "none" alg,
// JKU/X5C public-key fetching, etc.) that we don't need and that introduce
// risk if exposed by accident. This module supports exactly one cipher pair
// in compact serialization, and refuses anything else loudly.
//
// Compact JWE serialization (RFC 7516 §7.1):
//
//     BASE64URL(UTF8(JOSE Header))     . header
//     BASE64URL(JWE Encrypted Key)     . encrypted CEK (RSA-OAEP-256 wrapped)
//     BASE64URL(JWE Initialization Vector) . 12-byte GCM nonce
//     BASE64URL(JWE Ciphertext)        . AES-256-GCM ciphertext (no tag)
//     BASE64URL(JWE Authentication Tag) . 16-byte GCM auth tag
//
// The protected header bytes (BASE64URL form) are used as Additional
// Authenticated Data (AAD) for the GCM operation, exactly as in RFC 7516 §5.1.

use anyhow::{anyhow, bail, Context as _, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rsa::pkcs1::DecodeRsaPublicKey as _;
use rsa::pkcs8::{DecodePrivateKey as _, DecodePublicKey as _};
use rsa::{Oaep, RsaPrivateKey, RsaPublicKey};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};

/// The protected JOSE header for the only algorithm pair we support.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct ProtectedHeader {
    alg: String,
    enc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cty: Option<String>,
}

impl ProtectedHeader {
    fn rsa_oaep_256_a256gcm() -> Self {
        Self {
            alg: "RSA-OAEP-256".to_string(),
            enc: "A256GCM".to_string(),
            cty: Some("application/json".to_string()),
        }
    }
}

/// Parse a PEM-encoded RSA public key. Tries the modern SubjectPublicKeyInfo
/// format first (`-----BEGIN PUBLIC KEY-----`), falls back to PKCS#1
/// (`-----BEGIN RSA PUBLIC KEY-----`).
pub fn parse_public_pem(pem: &str) -> Result<RsaPublicKey> {
    if let Ok(k) = RsaPublicKey::from_public_key_pem(pem) {
        return Ok(k);
    }
    RsaPublicKey::from_pkcs1_pem(pem).context("parsing RSA public key (tried SPKI then PKCS#1)")
}

/// Parse a PEM-encoded RSA private key. Tries PKCS#8 first
/// (`-----BEGIN PRIVATE KEY-----`), falls back to PKCS#1
/// (`-----BEGIN RSA PRIVATE KEY-----`).
pub fn parse_private_pem(pem: &str) -> Result<RsaPrivateKey> {
    if let Ok(k) = RsaPrivateKey::from_pkcs8_pem(pem) {
        return Ok(k);
    }
    use rsa::pkcs1::DecodeRsaPrivateKey as _;
    RsaPrivateKey::from_pkcs1_pem(pem).context("parsing RSA private key (tried PKCS#8 then PKCS#1)")
}

/// Encrypt `plaintext` to `recipient_pub` as a compact JWE string using
/// RSA-OAEP-256 + A256GCM.
pub fn encrypt_compact(plaintext: &[u8], recipient_pub: &RsaPublicKey) -> Result<String> {
    let mut rng = rand::thread_rng();

    // 1. Build and base64url-encode the protected header.
    let header = ProtectedHeader::rsa_oaep_256_a256gcm();
    let header_json = serde_json::to_vec(&header).context("encoding JWE header")?;
    let header_b64 = URL_SAFE_NO_PAD.encode(&header_json);

    // 2. Generate the Content Encryption Key (32 bytes for AES-256).
    let mut cek = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rng, &mut cek);

    // 3. Wrap the CEK with RSA-OAEP-256.
    let padding = Oaep::new::<Sha256>();
    let encrypted_cek = recipient_pub
        .encrypt(&mut rng, padding, &cek)
        .context("RSA-OAEP-256 wrapping CEK")?;
    let encrypted_cek_b64 = URL_SAFE_NO_PAD.encode(&encrypted_cek);

    // 4. Generate the 12-byte GCM nonce (IV).
    let mut iv = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rng, &mut iv);
    let iv_b64 = URL_SAFE_NO_PAD.encode(iv);

    // 5. Encrypt the plaintext with AES-256-GCM. Per RFC 7516 §5.1 the AAD
    //    is the ASCII bytes of the BASE64URL-encoded protected header.
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&cek));
    let nonce = Nonce::from_slice(&iv);
    let ciphertext_with_tag = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad: header_b64.as_bytes(),
            },
        )
        .map_err(|e| anyhow!("AES-256-GCM encrypt failed: {e}"))?;

    if ciphertext_with_tag.len() < 16 {
        bail!("AES-GCM output too short to contain a 16-byte tag");
    }
    let tag_start = ciphertext_with_tag.len() - 16;
    let ciphertext = &ciphertext_with_tag[..tag_start];
    let tag = &ciphertext_with_tag[tag_start..];

    let ciphertext_b64 = URL_SAFE_NO_PAD.encode(ciphertext);
    let tag_b64 = URL_SAFE_NO_PAD.encode(tag);

    Ok(format!(
        "{header_b64}.{encrypted_cek_b64}.{iv_b64}.{ciphertext_b64}.{tag_b64}"
    ))
}

/// Decrypt a compact JWE string with `recipient_priv`. Verifies the header
/// matches RSA-OAEP-256 + A256GCM and refuses anything else.
pub fn decrypt_compact(jwe: &str, recipient_priv: &RsaPrivateKey) -> Result<Vec<u8>> {
    let parts: Vec<&str> = jwe.split('.').collect();
    if parts.len() != 5 {
        bail!(
            "JWE must have 5 dot-separated parts (header.cek.iv.ciphertext.tag); got {}",
            parts.len()
        );
    }
    let (header_b64, cek_b64, iv_b64, ct_b64, tag_b64) =
        (parts[0], parts[1], parts[2], parts[3], parts[4]);

    // Decode and verify the protected header.
    let header_bytes = URL_SAFE_NO_PAD
        .decode(header_b64)
        .context("decoding JWE protected header (base64url)")?;
    let header: ProtectedHeader =
        serde_json::from_slice(&header_bytes).context("parsing JWE protected header JSON")?;
    if header.alg != "RSA-OAEP-256" {
        bail!(
            "unsupported JWE alg `{}`; this module only handles RSA-OAEP-256",
            header.alg
        );
    }
    if header.enc != "A256GCM" {
        bail!(
            "unsupported JWE enc `{}`; this module only handles A256GCM",
            header.enc
        );
    }

    // Decode the rest of the parts.
    let encrypted_cek = URL_SAFE_NO_PAD
        .decode(cek_b64)
        .context("decoding JWE encrypted key")?;
    let iv = URL_SAFE_NO_PAD.decode(iv_b64).context("decoding JWE IV")?;
    if iv.len() != 12 {
        bail!("JWE IV must be 12 bytes for A256GCM, got {}", iv.len());
    }
    let ciphertext = URL_SAFE_NO_PAD
        .decode(ct_b64)
        .context("decoding JWE ciphertext")?;
    let tag = URL_SAFE_NO_PAD
        .decode(tag_b64)
        .context("decoding JWE auth tag")?;
    if tag.len() != 16 {
        bail!("JWE auth tag must be 16 bytes for A256GCM, got {}", tag.len());
    }

    // Unwrap the CEK with RSA-OAEP-256.
    let padding = Oaep::new::<Sha256>();
    let cek = recipient_priv
        .decrypt(padding, &encrypted_cek)
        .context("RSA-OAEP-256 unwrapping CEK")?;
    if cek.len() != 32 {
        bail!("unwrapped CEK must be 32 bytes for AES-256, got {}", cek.len());
    }

    // AES-GCM expects ciphertext || tag concatenated.
    let mut combined = Vec::with_capacity(ciphertext.len() + tag.len());
    combined.extend_from_slice(&ciphertext);
    combined.extend_from_slice(&tag);

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&cek));
    let nonce = Nonce::from_slice(&iv);
    let plaintext = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &combined,
                aad: header_b64.as_bytes(),
            },
        )
        .map_err(|e| anyhow!("AES-256-GCM decrypt/auth failed: {e}"))?;

    Ok(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::pkcs1::EncodeRsaPublicKey as _;
    use rsa::pkcs8::{EncodePrivateKey as _, EncodePublicKey as _};
    use rsa::pkcs8::LineEnding;

    fn fresh_keypair() -> (RsaPrivateKey, RsaPublicKey) {
        // 2048 is fine for tests; we don't need 4096-bit performance hits here.
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate key");
        let pub_key = RsaPublicKey::from(&priv_key);
        (priv_key, pub_key)
    }

    #[test]
    fn round_trip_small_payload() {
        let (priv_key, pub_key) = fresh_keypair();
        let plaintext = br#"{"hello":"world"}"#;

        let jwe = encrypt_compact(plaintext, &pub_key).expect("encrypt");
        assert_eq!(jwe.matches('.').count(), 4, "compact JWE must have 4 dots");

        let decrypted = decrypt_compact(&jwe, &priv_key).expect("decrypt");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn round_trip_larger_payload() {
        let (priv_key, pub_key) = fresh_keypair();
        let plaintext = vec![0xab; 4096];
        let jwe = encrypt_compact(&plaintext, &pub_key).expect("encrypt");
        let decrypted = decrypt_compact(&jwe, &priv_key).expect("decrypt");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn header_is_rejected_if_wrong_alg() {
        let (priv_key, pub_key) = fresh_keypair();
        let jwe = encrypt_compact(b"x", &pub_key).expect("encrypt");
        // Replace the protected header with one declaring a different alg.
        let bad_header = URL_SAFE_NO_PAD.encode(br#"{"alg":"dir","enc":"A256GCM"}"#);
        let mut parts: Vec<&str> = jwe.split('.').collect();
        parts[0] = &bad_header;
        let tampered = parts.join(".");
        let err = decrypt_compact(&tampered, &priv_key).unwrap_err();
        assert!(
            err.to_string().contains("unsupported JWE alg"),
            "expected alg rejection, got: {err}"
        );
    }

    #[test]
    fn tampered_ciphertext_fails_auth() {
        let (priv_key, pub_key) = fresh_keypair();
        let jwe = encrypt_compact(br#"{"x":1}"#, &pub_key).expect("encrypt");
        let mut parts: Vec<String> = jwe.split('.').map(String::from).collect();
        // Flip a byte inside the ciphertext (decode, mutate, re-encode).
        let mut ct = URL_SAFE_NO_PAD.decode(&parts[3]).unwrap();
        ct[0] ^= 0xff;
        parts[3] = URL_SAFE_NO_PAD.encode(&ct);
        let tampered = parts.join(".");
        let err = decrypt_compact(&tampered, &priv_key).unwrap_err();
        assert!(
            err.to_string().contains("AES-256-GCM decrypt/auth failed"),
            "expected GCM auth failure, got: {err}"
        );
    }

    #[test]
    fn pem_round_trip_pkcs8_and_spki() {
        let (priv_key, pub_key) = fresh_keypair();
        let priv_pem = priv_key.to_pkcs8_pem(LineEnding::LF).unwrap();
        let pub_pem = pub_key.to_public_key_pem(LineEnding::LF).unwrap();
        let pub_parsed = parse_public_pem(&pub_pem).unwrap();
        let priv_parsed = parse_private_pem(&priv_pem).unwrap();
        let jwe = encrypt_compact(b"hello", &pub_parsed).unwrap();
        let pt = decrypt_compact(&jwe, &priv_parsed).unwrap();
        assert_eq!(pt, b"hello");
    }

    #[test]
    fn pem_round_trip_pkcs1_public() {
        let (_priv, pub_key) = fresh_keypair();
        let pkcs1_pem = pub_key.to_pkcs1_pem(LineEnding::LF).unwrap();
        let parsed = parse_public_pem(&pkcs1_pem).unwrap();
        // Just verify the parsed key is functional by encrypting a tiny payload.
        let _ = encrypt_compact(b"x", &parsed).unwrap();
    }
}
