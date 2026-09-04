//! Cryptographic primitives: credential encryption, key derivation, token generation,
//! and password hashing.
//!
//! `MASTER_BUILD.md` Part 9 items 1 and 2 live here. Two rules drive every choice below:
//!
//! * A customer's provider key is the most dangerous thing we hold. It is encrypted with
//!   AES-256-GCM under a key that never touches the database, and it is never returned to
//!   a browser in any form.
//! * Our own API keys are never stored. We keep a SHA-256 hash and a display prefix, so a
//!   database disclosure does not hand an attacker working credentials.

use crate::error::{AegisError, Result};
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// Prefix identifying an Aegis secret API key.
pub const API_KEY_PREFIX: &str = "aegis_sk_";
/// Prefix identifying an Aegis session token.
pub const SESSION_TOKEN_PREFIX: &str = "aegis_st_";
/// Prefix identifying an Aegis SCIM provisioning token.
///
/// Deliberately distinct from [`API_KEY_PREFIX`] even though the two are generated
/// identically — a SCIM token can deprovision every member of an organisation, so it
/// should be visually distinguishable at a glance (in a log line, a secrets manager, a
/// screen-share) from an ordinary key that only calls a chat endpoint.
pub const SCIM_TOKEN_PREFIX: &str = "aegis_scim_";
/// Number of random characters after the prefix. 43 base62 characters carry ~256 bits.
pub const API_KEY_RANDOM_LEN: usize = 43;
/// Characters shown in the dashboard so a user can tell two keys apart.
pub const KEY_PREFIX_DISPLAY_LEN: usize = 16;

/// AES-256-GCM nonce length in bytes.
const NONCE_LEN: usize = 12;

const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// A freshly minted API key. The plaintext exists only in this struct, only once.
#[derive(Debug, Clone)]
pub struct GeneratedKey {
    /// Full key, shown to the user exactly once at creation and never persisted.
    pub plaintext: String,
    /// First [`KEY_PREFIX_DISPLAY_LEN`] characters, safe to store and display.
    pub prefix: String,
    /// SHA-256 hex digest — the only form we keep.
    pub hash: String,
}

/// Generate a new API key using the OS CSPRNG.
///
/// Rejection sampling keeps the base62 alphabet uniform; the naive `% 62` would make the
/// first two characters of the alphabet measurably more likely.
pub fn generate_api_key() -> GeneratedKey {
    let random = random_base62(API_KEY_RANDOM_LEN);
    let plaintext = format!("{API_KEY_PREFIX}{random}");
    let prefix = plaintext.chars().take(KEY_PREFIX_DISPLAY_LEN).collect();
    let hash = hash_token(&plaintext);
    GeneratedKey {
        plaintext,
        prefix,
        hash,
    }
}

/// Generate a session token. Same construction as an API key, different prefix so the two
/// are never confused in a log or a lookup.
pub fn generate_session_token() -> GeneratedKey {
    let random = random_base62(API_KEY_RANDOM_LEN);
    let plaintext = format!("{SESSION_TOKEN_PREFIX}{random}");
    let prefix = plaintext.chars().take(KEY_PREFIX_DISPLAY_LEN).collect();
    let hash = hash_token(&plaintext);
    GeneratedKey {
        plaintext,
        prefix,
        hash,
    }
}

/// Generate a new SCIM provisioning token.
///
/// The token, hashed and stored in `scim_tokens`, was always mintable this way in the
/// repository layer — `repo::create_scim_token` existed and was tested. Nothing in the
/// management API ever called it, so a customer wanting SCIM had no way to get a token
/// without a direct database write on our side. Found in the enterprise readiness audit.
pub fn generate_scim_token() -> GeneratedKey {
    let random = random_base62(API_KEY_RANDOM_LEN);
    let plaintext = format!("{SCIM_TOKEN_PREFIX}{random}");
    let prefix = plaintext.chars().take(KEY_PREFIX_DISPLAY_LEN).collect();
    let hash = hash_token(&plaintext);
    GeneratedKey {
        plaintext,
        prefix,
        hash,
    }
}

fn random_base62(len: usize) -> String {
    let mut rng = OsRng;
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    while out.len() < len {
        rng.fill_bytes(&mut buf);
        for &byte in buf.iter() {
            // 248 = 62 * 4: the largest multiple of 62 under 256. Discarding the rest
            // keeps the distribution uniform.
            if byte < 248 {
                out.push(BASE62[(byte % 62) as usize] as char);
                if out.len() == len {
                    break;
                }
            }
        }
    }
    out
}

/// SHA-256 hex digest of a token. The stored form of every credential we issue.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Constant-time comparison of two hex digests.
///
/// Used when a candidate hash is compared against a stored one. A byte-by-byte `==`
/// leaks, through timing, how many leading characters matched.
pub fn secure_compare(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

/// True when `candidate` has the shape of an Aegis API key.
/// A cheap syntactic gate before any lookup, so malformed input never reaches the store.
pub fn looks_like_api_key(candidate: &str) -> bool {
    candidate.len() == API_KEY_PREFIX.len() + API_KEY_RANDOM_LEN
        && candidate.starts_with(API_KEY_PREFIX)
        && candidate[API_KEY_PREFIX.len()..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric())
}

// ---------------------------------------------------------------------------
// Symmetric encryption for BYOK provider credentials
// ---------------------------------------------------------------------------

/// Encrypt `plaintext` with AES-256-GCM under `key`.
///
/// Output layout is `nonce || ciphertext || tag`. The nonce is random per call — reusing
/// a nonce under the same key breaks GCM catastrophically, so it is generated here and
/// never supplied by a caller.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));

    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| AegisError::Crypto)?;

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Decrypt a blob produced by [`encrypt`].
///
/// Returns [`AegisError::Crypto`] for every failure mode — wrong key, truncated input,
/// tampered ciphertext — without distinguishing them. Which one it was is not something
/// a caller needs to know, and telling them turns the error into an oracle.
pub fn decrypt(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() <= NONCE_LEN {
        return Err(AegisError::Crypto);
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let (nonce_bytes, ciphertext) = blob.split_at(NONCE_LEN);
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|_| AegisError::Crypto)
}

/// Encrypt a UTF-8 string, returning base64 for JSON transport.
pub fn encrypt_string(key: &[u8; 32], plaintext: &str) -> Result<String> {
    Ok(base64::engine::general_purpose::STANDARD.encode(encrypt(key, plaintext.as_bytes())?))
}

/// Decrypt a base64 blob back into a string.
pub fn decrypt_string(key: &[u8; 32], encoded: &str) -> Result<String> {
    let blob = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| AegisError::Crypto)?;
    String::from_utf8(decrypt(key, &blob)?).map_err(|_| AegisError::Crypto)
}

/// Derive a per-tenant encryption key from the master key and an organisation id
/// (HKDF-SHA256, Part 9 item 2 / Phase 6).
///
/// Deterministic, so no derived key is ever stored, and org-scoped, so compromising one
/// tenant's derived key reveals nothing about another's.
pub fn derive_tenant_key(master_key: &[u8; 32], org_id: &str) -> [u8; 32] {
    let hk = hkdf::Hkdf::<Sha256>::new(Some(b"aegis-tenant-key-v1"), master_key);
    let mut derived = [0u8; 32];
    // `expand` only fails for absurd output lengths; 32 bytes is always valid.
    let _ = hk.expand(org_id.as_bytes(), &mut derived);
    derived
}

/// Derive a per-*user* encryption key, for secrets that belong to an account rather than
/// an organisation.
///
/// A TOTP secret is exactly this shape: a user can belong to several organisations, so
/// encrypting it under [`derive_tenant_key`] would either tie it to one organisation
/// arbitrarily or require re-encrypting it per membership. A distinct HKDF `info` string
/// (`"aegis-user-key-v1"` vs. `"aegis-tenant-key-v1"`) keeps the two derivation spaces
/// disjoint under the same master key, so a leaked user key reveals nothing about any
/// tenant key and vice versa.
pub fn derive_user_key(master_key: &[u8; 32], user_id: &str) -> [u8; 32] {
    let hk = hkdf::Hkdf::<Sha256>::new(Some(b"aegis-user-key-v1"), master_key);
    let mut derived = [0u8; 32];
    let _ = hk.expand(user_id.as_bytes(), &mut derived);
    derived
}

// ---------------------------------------------------------------------------
// Password hashing
// ---------------------------------------------------------------------------

/// Hash a password with argon2id and a random salt.
pub fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| AegisError::Crypto)
}

/// Verify a password against a stored argon2id hash.
///
/// Returns `false` rather than an error for a malformed stored hash: a corrupt row must
/// not become a login bypass, and it must not distinguish itself from a wrong password.
pub fn verify_password(password: &str, stored_hash: &str) -> bool {
    match PasswordHash::new(stored_hash) {
        Ok(parsed) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn api_keys_have_the_specified_shape() {
        let key = generate_api_key();
        assert!(key.plaintext.starts_with(API_KEY_PREFIX));
        assert_eq!(
            key.plaintext.len(),
            API_KEY_PREFIX.len() + API_KEY_RANDOM_LEN
        );
        assert_eq!(key.prefix.len(), KEY_PREFIX_DISPLAY_LEN);
        assert!(key.plaintext.starts_with(&key.prefix));
        assert_eq!(key.hash.len(), 64, "sha256 hex is 64 characters");
        assert!(looks_like_api_key(&key.plaintext));
    }

    #[test]
    fn api_keys_are_unique_across_many_generations() {
        let keys: HashSet<String> = (0..2_000).map(|_| generate_api_key().plaintext).collect();
        assert_eq!(keys.len(), 2_000, "CSPRNG produced a collision");
    }

    #[test]
    fn base62_alphabet_is_used_uniformly() {
        // Rejection sampling should keep every character roughly equally likely. With
        // 62_000 samples the expected count per character is 1000; a modulo-bias bug
        // skews the first four characters by ~6%, far outside this tolerance.
        let mut counts = [0usize; 62];
        let sample = random_base62(62_000);
        for ch in sample.chars() {
            let idx = BASE62
                .iter()
                .position(|&b| b as char == ch)
                .expect("in alphabet");
            counts[idx] += 1;
        }
        let min = *counts.iter().min().unwrap();
        let max = *counts.iter().max().unwrap();
        assert!(max - min < 250, "distribution skewed: min={min} max={max}");
    }

    #[test]
    fn session_tokens_are_distinguishable_from_api_keys() {
        let session = generate_session_token();
        assert!(session.plaintext.starts_with(SESSION_TOKEN_PREFIX));
        assert!(
            !looks_like_api_key(&session.plaintext),
            "a session token must never pass as an API key"
        );
    }

    #[test]
    fn key_shape_validation_rejects_junk() {
        assert!(!looks_like_api_key(""));
        assert!(!looks_like_api_key("aegis_sk_"));
        assert!(!looks_like_api_key("sk-openai-style-key"));
        assert!(!looks_like_api_key(&format!(
            "{API_KEY_PREFIX}{}",
            "a".repeat(10)
        )));
        // Right length, wrong characters.
        assert!(!looks_like_api_key(&format!(
            "{API_KEY_PREFIX}{}",
            "-".repeat(43)
        )));
    }

    #[test]
    fn hashing_is_deterministic_and_one_way() {
        let key = generate_api_key();
        assert_eq!(hash_token(&key.plaintext), key.hash);
        assert_ne!(key.hash, key.plaintext);
        assert!(!key.hash.contains(&key.plaintext[9..20]));
    }

    #[test]
    fn secure_compare_matches_equality_semantics() {
        assert!(secure_compare("abc", "abc"));
        assert!(!secure_compare("abc", "abd"));
        assert!(!secure_compare("abc", "abcd"));
        assert!(secure_compare("", ""));
    }

    #[test]
    fn encryption_round_trips() {
        let key = [42u8; 32];
        let secret = "sk-proj-a-real-looking-openai-key-000111222";
        let blob = encrypt(&key, secret.as_bytes()).unwrap();
        assert_eq!(decrypt(&key, &blob).unwrap(), secret.as_bytes());
    }

    #[test]
    fn ciphertext_never_contains_the_plaintext() {
        let key = [42u8; 32];
        let secret = "sk-proj-supersecretvalue";
        let blob = encrypt(&key, secret.as_bytes()).unwrap();
        let haystack = String::from_utf8_lossy(&blob);
        assert!(!haystack.contains("supersecret"));
    }

    #[test]
    fn each_encryption_uses_a_fresh_nonce() {
        // Identical plaintext must produce different ciphertext. Equal outputs would mean
        // a fixed nonce, which breaks GCM completely.
        let key = [42u8; 32];
        let a = encrypt(&key, b"same input").unwrap();
        let b = encrypt(&key, b"same input").unwrap();
        assert_ne!(a, b);
        assert_ne!(a[..NONCE_LEN], b[..NONCE_LEN]);
    }

    #[test]
    fn decryption_fails_with_the_wrong_key() {
        let blob = encrypt(&[1u8; 32], b"secret").unwrap();
        assert!(decrypt(&[2u8; 32], &blob).is_err());
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        // GCM authenticates: flipping any byte must fail rather than yield garbage.
        let key = [42u8; 32];
        let mut blob = encrypt(&key, b"provider credential").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        assert!(decrypt(&key, &blob).is_err());

        let mut nonce_tampered = encrypt(&key, b"provider credential").unwrap();
        nonce_tampered[0] ^= 0x01;
        assert!(decrypt(&key, &nonce_tampered).is_err());
    }

    #[test]
    fn truncated_blobs_are_rejected_without_panicking() {
        let key = [42u8; 32];
        assert!(decrypt(&key, b"").is_err());
        assert!(decrypt(&key, &[0u8; NONCE_LEN]).is_err());
        assert!(decrypt(&key, &[0u8; NONCE_LEN - 1]).is_err());
    }

    #[test]
    fn string_helpers_round_trip_through_base64() {
        let key = [7u8; 32];
        let encoded = encrypt_string(&key, "sk-ant-api03-value").unwrap();
        assert!(!encoded.contains("sk-ant"));
        assert_eq!(
            decrypt_string(&key, &encoded).unwrap(),
            "sk-ant-api03-value"
        );
        assert!(decrypt_string(&key, "not-base64!!").is_err());
    }

    #[test]
    fn tenant_keys_are_deterministic_and_isolated() {
        let master = [9u8; 32];
        let org_a = "11111111-1111-1111-1111-111111111111";
        let org_b = "22222222-2222-2222-2222-222222222222";

        assert_eq!(
            derive_tenant_key(&master, org_a),
            derive_tenant_key(&master, org_a)
        );
        assert_ne!(
            derive_tenant_key(&master, org_a),
            derive_tenant_key(&master, org_b)
        );
        assert_ne!(derive_tenant_key(&master, org_a), master);
    }

    #[test]
    fn tenant_key_cannot_decrypt_another_tenants_data() {
        // The property that makes per-tenant keys worth having.
        let master = [9u8; 32];
        let key_a = derive_tenant_key(&master, "org-a");
        let key_b = derive_tenant_key(&master, "org-b");
        let blob = encrypt(&key_a, b"org a private prompt").unwrap();
        assert!(decrypt(&key_b, &blob).is_err());
    }

    #[test]
    fn password_hashing_round_trips() {
        let hash = hash_password("correct horse battery staple").unwrap();
        assert!(verify_password("correct horse battery staple", &hash));
        assert!(!verify_password("wrong password", &hash));
    }

    #[test]
    fn password_hashes_are_salted() {
        let a = hash_password("same password").unwrap();
        let b = hash_password("same password").unwrap();
        assert_ne!(a, b, "identical hashes mean the salt is not random");
        assert!(verify_password("same password", &a));
        assert!(verify_password("same password", &b));
    }

    #[test]
    fn password_hash_is_argon2id() {
        let hash = hash_password("x").unwrap();
        assert!(
            hash.starts_with("$argon2id$"),
            "unexpected algorithm: {hash}"
        );
    }

    #[test]
    fn corrupt_stored_hash_denies_access_rather_than_granting_it() {
        assert!(!verify_password("anything", ""));
        assert!(!verify_password("anything", "not-a-hash"));
        assert!(!verify_password("anything", "$argon2id$garbage"));
    }

    /// `scripts/seed.sql`'s hardcoded hash for `dev@aegis.local` must actually verify
    /// against the password its own comment says it is.
    ///
    /// It didn't, for as long as the file existed: the literal had the correct shape
    /// (`$argon2id$v=19$m=...$salt$hash`, valid base64, right length) but was hand-typed
    /// rather than produced by ever actually calling `hash_password`, so it was fabricated
    /// text that merely looked like a real hash — `dev@aegis.local` had never been able to
    /// log in with the password its own seed file documented. No test read the SQL file at
    /// all, so nothing caught it until a real login attempt against a real database did,
    /// this session. This test is what should have existed from the start: read the exact
    /// same file a human would edit, extract the exact same hash a real login checks
    /// against, and assert the promise the comment makes actually holds.
    #[test]
    fn seed_sql_dev_account_hash_verifies_against_its_documented_password() {
        let seed_sql_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/seed.sql");
        let seed_sql = std::fs::read_to_string(&seed_sql_path)
            .unwrap_or_else(|e| panic!("could not read {}: {e}", seed_sql_path.display()));

        // Find the hash by its own unmistakable shape, anchored to the start of a SQL
        // string literal (`'$argon2id$`) rather than a bare `$argon2id$` — a comment
        // above this exact INSERT used to illustrate the hash's shape as prose
        // (`($argon2id$v=19$m=...$salt$hash)`) and a bare search matched *that* first,
        // extracting eleven characters of English instead of a hash. Anchoring on the
        // opening quote is what a real SQL string literal always has and free-form
        // comment prose never does.
        let insert_start = seed_sql
            .find("'dev@aegis.local'")
            .expect("seed.sql no longer seeds dev@aegis.local at all");
        let after_email = &seed_sql[insert_start..];
        let hash_start = after_email
            .find("'$argon2id$")
            .expect("no quoted $argon2id$ hash literal found after dev@aegis.local in seed.sql")
            + 1; // past the opening quote itself
        let hash_region = &after_email[hash_start..];
        let hash_end = hash_region
            .find('\'')
            .expect("the $argon2id$ hash literal in seed.sql is not closed by a following quote");
        let hash = &hash_region[..hash_end];

        assert!(
            verify_password("aegis-development-password", hash),
            "scripts/seed.sql's hash for dev@aegis.local does not verify against \
             \"aegis-development-password\" — the documented dev login is broken. \
             Regenerate it with crypto::hash_password(\"aegis-development-password\"), \
             never hand-write one."
        );
    }
}
