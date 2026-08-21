//! TOTP two-factor authentication for admin accounts (RFC 6238), Part 9 item 12.
//!
//! Admin accounts can see every organisation's metrics and reset circuit breakers, so a
//! stolen admin password must not be sufficient on its own.
//!
//! # The two details that are easy to get wrong
//!
//! * **A window of ±1 step.** Phone clocks drift. Accepting only the current step
//!   produces intermittent, unreproducible login failures; accepting a wide window
//!   materially extends the life of a stolen code. One step either side (±30s) is the
//!   standard trade.
//! * **Constant-time comparison.** A `==` on the code leaks, through timing, how many
//!   leading digits were right — enough to reduce a six-digit space to six sequential
//!   searches of ten.

use crate::error::{AegisError, Result};
use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// Seconds per TOTP step.
pub const STEP_SECONDS: u64 = 30;
/// Digits in a generated code.
pub const DIGITS: u32 = 6;
/// Steps accepted either side of the current one.
pub const WINDOW_STEPS: i64 = 1;
/// Length of a generated secret, in bytes.
pub const SECRET_BYTES: usize = 20;

/// Base32 alphabet (RFC 4648), which is what authenticator apps expect.
const BASE32_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Generate a new TOTP secret, base32-encoded.
pub fn generate_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; SECRET_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base32_encode(&bytes)
}

/// The `otpauth://` URI an authenticator app scans.
pub fn provisioning_uri(secret: &str, account: &str, issuer: &str) -> String {
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={}&period={}",
        urlencode(issuer),
        urlencode(account),
        secret,
        urlencode(issuer),
        DIGITS,
        STEP_SECONDS,
    )
}

/// Generate the code for a given Unix timestamp.
pub fn generate_code(secret: &str, timestamp: u64) -> Result<String> {
    let key = base32_decode(secret).ok_or(AegisError::Crypto)?;
    Ok(generate_for_counter(&key, timestamp / STEP_SECONDS))
}

fn generate_for_counter(key: &[u8], counter: u64) -> String {
    let Ok(mut mac) = HmacSha1::new_from_slice(key) else {
        return String::new();
    };
    mac.update(&counter.to_be_bytes());
    let digest = mac.finalize().into_bytes();

    // Dynamic truncation, RFC 4226 section 5.4.
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let binary = ((digest[offset] as u32 & 0x7f) << 24)
        | ((digest[offset + 1] as u32) << 16)
        | ((digest[offset + 2] as u32) << 8)
        | (digest[offset + 3] as u32);

    let modulus = 10u32.pow(DIGITS);
    format!("{:0width$}", binary % modulus, width = DIGITS as usize)
}

/// Verify a code against the current time, allowing for clock drift.
pub fn verify_code(secret: &str, code: &str, timestamp: u64) -> bool {
    let Some(key) = base32_decode(secret) else {
        return false;
    };

    // Strip spaces: authenticator apps display "123 456" and people paste it that way.
    let candidate: String = code.chars().filter(|c| c.is_ascii_digit()).collect();
    if candidate.len() != DIGITS as usize {
        return false;
    }

    let current = (timestamp / STEP_SECONDS) as i64;
    for offset in -WINDOW_STEPS..=WINDOW_STEPS {
        let counter = (current + offset).max(0) as u64;
        // Constant-time: a plain `==` leaks how many leading digits matched.
        if crate::crypto::secure_compare(&generate_for_counter(&key, counter), &candidate) {
            return true;
        }
    }
    false
}

/// Encode bytes as base32 without padding.
fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;

    for &byte in data {
        buffer = (buffer << 8) | byte as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(BASE32_ALPHABET[((buffer >> bits) & 0x1f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(BASE32_ALPHABET[((buffer << (5 - bits)) & 0x1f) as usize] as char);
    }
    out
}

/// Decode a base32 string, tolerating lowercase, spaces, and padding.
fn base32_decode(encoded: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;

    for ch in encoded.chars() {
        if ch == '=' || ch == ' ' || ch == '-' {
            continue;
        }
        let upper = ch.to_ascii_uppercase();
        let index = BASE32_ALPHABET.iter().position(|&c| c as char == upper)? as u32;

        buffer = (buffer << 5) | index;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }

    (!out.is_empty()).then_some(out)
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_secrets_are_base32_and_long_enough() {
        let secret = generate_secret();
        assert_eq!(secret.len(), 32, "20 bytes encodes to 32 base32 characters");
        assert!(secret
            .chars()
            .all(|c| BASE32_ALPHABET.contains(&(c as u8))));
    }

    #[test]
    fn secrets_are_unique() {
        use std::collections::HashSet;
        let secrets: HashSet<String> = (0..500).map(|_| generate_secret()).collect();
        assert_eq!(secrets.len(), 500);
    }

    #[test]
    fn base32_round_trips() {
        for data in [
            b"hello world!".to_vec(),
            vec![0u8; 20],
            vec![255u8; 20],
            (0..20u8).collect::<Vec<u8>>(),
        ] {
            let encoded = base32_encode(&data);
            let decoded = base32_decode(&encoded).unwrap();
            assert_eq!(&decoded[..data.len()], &data[..]);
        }
    }

    #[test]
    fn base32_decoding_tolerates_how_people_actually_paste_secrets() {
        let secret = generate_secret();
        let decoded = base32_decode(&secret).unwrap();

        assert_eq!(base32_decode(&secret.to_lowercase()).unwrap(), decoded);
        assert_eq!(base32_decode(&format!("{secret}==")).unwrap(), decoded);

        let spaced = secret
            .chars()
            .collect::<Vec<_>>()
            .chunks(4)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(base32_decode(&spaced).unwrap(), decoded);
    }

    #[test]
    fn base32_rejects_invalid_input() {
        assert!(base32_decode("!!!!").is_none());
        assert!(base32_decode("").is_none());
        // 0, 1, and 8 are deliberately absent from the alphabet.
        assert!(base32_decode("ABC1").is_none());
    }

    #[test]
    fn codes_are_six_digits() {
        let secret = generate_secret();
        let code = generate_code(&secret, 1_800_000_000).unwrap();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn codes_are_stable_within_a_step_and_change_between_them() {
        let secret = generate_secret();
        let base = 1_800_000_000u64 / STEP_SECONDS * STEP_SECONDS;

        let start = generate_code(&secret, base).unwrap();
        let end = generate_code(&secret, base + STEP_SECONDS - 1).unwrap();
        assert_eq!(start, end, "the code must not change mid-step");

        let next = generate_code(&secret, base + STEP_SECONDS).unwrap();
        assert_ne!(start, next, "the code must change between steps");
    }

    #[test]
    fn a_current_code_verifies() {
        let secret = generate_secret();
        let now = 1_800_000_000;
        let code = generate_code(&secret, now).unwrap();
        assert!(verify_code(&secret, &code, now));
    }

    #[test]
    fn clock_drift_of_one_step_is_tolerated() {
        // Phone clocks drift. Rejecting on that produces login failures nobody can
        // reproduce.
        let secret = generate_secret();
        let now = 1_800_000_000;
        let code = generate_code(&secret, now).unwrap();

        assert!(verify_code(&secret, &code, now + STEP_SECONDS));
        assert!(verify_code(&secret, &code, now - STEP_SECONDS));
    }

    #[test]
    fn drift_beyond_the_window_is_rejected() {
        // A wider window materially extends the life of a stolen code.
        let secret = generate_secret();
        let now = 1_800_000_000;
        let code = generate_code(&secret, now).unwrap();

        assert!(!verify_code(&secret, &code, now + 5 * STEP_SECONDS));
        assert!(!verify_code(&secret, &code, now - 5 * STEP_SECONDS));
    }

    #[test]
    fn a_code_from_another_secret_is_rejected() {
        let now = 1_800_000_000;
        let code = generate_code(&generate_secret(), now).unwrap();
        assert!(!verify_code(&generate_secret(), &code, now));
    }

    #[test]
    fn codes_are_accepted_as_users_actually_type_them() {
        // Authenticator apps display "123 456" and people paste it verbatim.
        let secret = generate_secret();
        let now = 1_800_000_000;
        let code = generate_code(&secret, now).unwrap();

        let spaced = format!("{} {}", &code[..3], &code[3..]);
        assert!(verify_code(&secret, &spaced, now));
        assert!(verify_code(&secret, &format!("  {code}  "), now));
    }

    #[test]
    fn malformed_codes_are_rejected_without_panicking() {
        let secret = generate_secret();
        let now = 1_800_000_000;
        for code in ["", "12345", "1234567", "abcdef", "!!!!!!"] {
            assert!(!verify_code(&secret, code, now), "accepted {code:?}");
        }
    }

    #[test]
    fn an_invalid_secret_never_verifies() {
        assert!(!verify_code("not-base32!!", "123456", 1_800_000_000));
        assert!(!verify_code("", "123456", 1_800_000_000));
    }

    #[test]
    fn the_provisioning_uri_is_scannable() {
        let secret = generate_secret();
        let uri = provisioning_uri(&secret, "admin@aegis.dev", "Aegis");

        assert!(uri.starts_with("otpauth://totp/"));
        assert!(uri.contains(&format!("secret={secret}")));
        assert!(uri.contains("issuer=Aegis"));
        assert!(uri.contains("digits=6"));
        assert!(uri.contains("period=30"));
        // The account contains an @, which must be encoded in the label.
        assert!(uri.contains("admin%40aegis.dev"), "{uri}");
    }

    #[test]
    fn rfc_6238_reference_vector_matches() {
        // RFC 6238 Appendix B: the ASCII secret "12345678901234567890" at T=59
        // produces 94287082 for SHA-1. Truncated to six digits, that is 287082.
        let secret = base32_encode(b"12345678901234567890");
        assert_eq!(generate_code(&secret, 59).unwrap(), "287082");
    }
}
