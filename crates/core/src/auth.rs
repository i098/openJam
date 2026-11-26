use crate::{Error, Result};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use rand::{Rng, distr::Alphanumeric};
use sha2::{Digest, Sha256};

pub const API_KEY_PREFIX: &str = "sk_";
/// Characters of a key kept in clear to recognize it in lists: `sk_` plus 8.
const KEY_PREFIX_LEN: usize = 11;

pub fn hash_password(password: &str) -> Result<String> {
    Argon2::default()
        .hash_password(password.as_bytes(), &SaltString::generate(&mut OsRng))
        .map(|h| h.to_string())
        .map_err(|e| Error::Internal(format!("password hash: {e}")))
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).is_ok_and(|h| {
        Argon2::default()
            .verify_password(password.as_bytes(), &h)
            .is_ok()
    })
}

/// SHA-256 as hex. Session tokens and API keys are random, so a fast hash is enough.
pub fn sha256_hex(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

fn random_token(len: usize) -> String {
    rand::rng()
        .sample_iter(Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

pub fn new_session_token() -> String {
    random_token(48)
}

pub struct NewApiKey {
    pub secret: String,
    pub prefix: String,
    pub hash: String,
}

pub fn new_api_key() -> NewApiKey {
    let secret = format!("{API_KEY_PREFIX}{}", random_token(40));
    NewApiKey {
        prefix: secret[..KEY_PREFIX_LEN].to_string(),
        hash: sha256_hex(&secret),
        secret,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip() {
        let hash = hash_password("correct horse").unwrap();
        assert!(verify_password("correct horse", &hash));
        assert!(!verify_password("wrong", &hash));
        assert!(!verify_password("x", "not a hash"));
    }

    #[test]
    fn api_key_shape_and_hash() {
        let key = new_api_key();
        assert!(key.secret.starts_with("sk_"));
        assert_eq!(key.secret.len(), 43);
        assert_eq!(key.prefix, key.secret[..11]);
        assert_eq!(key.hash, sha256_hex(&key.secret));
        assert_ne!(new_api_key().secret, key.secret);
    }
}
