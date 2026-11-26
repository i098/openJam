//! Secrets at rest: XChaCha20-Poly1305 with a versioned keyring.
//!
//! A sealed blob is `version (1 byte) | nonce (24 bytes) | ciphertext`. The associated data binds
//! the blob to its row id and key version, so a blob copied to another row does not open.
//! The keyring text is `v1:<base64 of 32 bytes>,v2:...`. The highest version seals; all open.

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use sqlx::PgPool;
use std::{collections::BTreeMap, fmt, fs, io::Write, os::unix::fs::OpenOptionsExt, path::Path};
use uuid::Uuid;

const AAD_PREFIX: &[u8] = b"openjam/v1";
const NONCE_LEN: usize = 24;

/// A secret in memory. `Debug` prints nothing useful, so logs never hold it.
#[derive(Clone)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("master keys are invalid: {0}")]
    Keyring(String),
    #[error(
        "a stored secret exists but no master key is set; set OPENJAM_MASTER_KEYS or restore {0}"
    )]
    KeyMissing(String),
    #[error("cannot open the secret: wrong key, wrong row or damaged data")]
    Open,
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct Keyring {
    keys: BTreeMap<u8, [u8; 32]>,
}

impl fmt::Debug for Keyring {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Keyring(versions {:?})",
            self.keys.keys().collect::<Vec<_>>()
        )
    }
}

impl Keyring {
    pub fn parse(text: &str) -> Result<Self, SecretError> {
        let mut keys = BTreeMap::new();
        for part in text
            .split([',', '\n'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            let (version, key) = part.split_once(':').ok_or_else(|| {
                SecretError::Keyring(format!("bad entry, want v1:<base64>: {part:.4}…"))
            })?;
            let version: u8 = version
                .strip_prefix('v')
                .and_then(|v| v.parse().ok())
                .filter(|v| *v > 0)
                .ok_or_else(|| SecretError::Keyring(format!("bad version {version}")))?;
            let bytes = STANDARD
                .decode(key)
                .map_err(|_| SecretError::Keyring(format!("v{version} is not base64")))?;
            let key: [u8; 32] = bytes
                .try_into()
                .map_err(|_| SecretError::Keyring(format!("v{version} is not 32 bytes")))?;
            keys.insert(version, key);
        }
        if keys.is_empty() {
            return Err(SecretError::Keyring("no keys".into()));
        }
        Ok(Self { keys })
    }

    /// A new keyring with one random key at version 1.
    pub fn generate() -> (Self, String) {
        let key: [u8; 32] = rand::random();
        let text = format!("v1:{}", STANDARD.encode(key));
        (
            Self {
                keys: BTreeMap::from([(1, key)]),
            },
            text,
        )
    }

    /// The keyring from `OPENJAM_MASTER_KEYS`, else from `file`. When neither exists and the
    /// database holds no secret yet, make `file` with a new key. When the database holds a secret
    /// and no key exists, stop: a new key would not open it.
    pub async fn load(pool: &PgPool, file: &Path) -> Result<Self, SecretError> {
        if let Ok(text) = std::env::var("OPENJAM_MASTER_KEYS")
            && !text.trim().is_empty()
        {
            return Self::parse(&text);
        }
        if let Ok(text) = fs::read_to_string(file) {
            return Self::parse(&text);
        }
        let stored: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM engine_accounts WHERE credential_ciphertext IS NOT NULL)",
        )
        .fetch_one(pool)
        .await?;
        if stored {
            return Err(SecretError::KeyMissing(file.display().to_string()));
        }
        let (keyring, text) = Self::generate();
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        // create_new: when the API and the worker start together, one wins and the other reads.
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(file)
        {
            Ok(mut f) => {
                f.write_all(text.as_bytes())?;
                Ok(keyring)
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Self::parse(&fs::read_to_string(file)?)
            }
            Err(e) => Err(e.into()),
        }
    }

    fn aad(id: Uuid, version: u8) -> Vec<u8> {
        [AAD_PREFIX, id.as_bytes(), &[version]].concat()
    }

    /// Seal `plaintext` for the row `id`.
    pub fn seal(&self, id: Uuid, plaintext: &[u8]) -> Vec<u8> {
        let (&version, key) = self.keys.last_key_value().expect("keyring is never empty");
        let nonce: [u8; NONCE_LEN] = rand::random();
        let ct = XChaCha20Poly1305::new(key.into())
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &Self::aad(id, version),
                },
            )
            .expect("encrypting in memory cannot fail");
        [&[version][..], &nonce, &ct].concat()
    }

    pub fn open(&self, id: Uuid, blob: &[u8]) -> Result<Vec<u8>, SecretError> {
        let (&version, rest) = blob.split_first().ok_or(SecretError::Open)?;
        let key = self.keys.get(&version).ok_or(SecretError::Open)?;
        if rest.len() < NONCE_LEN {
            return Err(SecretError::Open);
        }
        let (nonce, ct) = rest.split_at(NONCE_LEN);
        XChaCha20Poly1305::new(key.into())
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ct,
                    aad: &Self::aad(id, version),
                },
            )
            .map_err(|_| SecretError::Open)
    }

    pub fn open_string(&self, id: Uuid, blob: &[u8]) -> Result<Secret, SecretError> {
        let bytes = self.open(id, blob)?;
        String::from_utf8(bytes)
            .map(Secret)
            .map_err(|_| SecretError::Open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(text: &str) -> Keyring {
        Keyring::parse(text).unwrap()
    }

    #[test]
    fn round_trip_uses_a_new_nonce_every_time() {
        let k = Keyring::generate().0;
        let id = Uuid::new_v4();
        let (a, b) = (k.seal(id, b"sk-secret"), k.seal(id, b"sk-secret"));
        assert_ne!(a, b, "a nonce is never reused");
        assert_eq!(k.open(id, &a).unwrap(), b"sk-secret");
        assert_eq!(k.open_string(id, &b).unwrap().expose(), "sk-secret");
    }

    #[test]
    fn a_blob_does_not_open_for_another_row_or_a_damaged_byte() {
        let k = Keyring::generate().0;
        let blob = k.seal(Uuid::new_v4(), b"sk-secret");
        assert!(k.open(Uuid::new_v4(), &blob).is_err());
        let id = Uuid::new_v4();
        let mut blob = k.seal(id, b"sk-secret");
        *blob.last_mut().unwrap() ^= 1;
        assert!(k.open(id, &blob).is_err());
    }

    #[test]
    fn the_highest_version_seals_and_every_version_opens() {
        let (k1, t1) = Keyring::generate();
        let (_, t2) = Keyring::generate();
        let id = Uuid::new_v4();
        let old = k1.seal(id, b"old");
        let both = ring(&format!("{t1},{}", t2.replace("v1:", "v2:")));
        assert_eq!(both.open(id, &old).unwrap(), b"old");
        assert_eq!(both.seal(id, b"new")[0], 2);
    }

    #[test]
    fn a_bad_keyring_is_refused() {
        assert!(Keyring::parse("v1:abc").is_err());
        assert!(Keyring::parse("").is_err());
        assert!(Keyring::parse("key").is_err());
    }
}
