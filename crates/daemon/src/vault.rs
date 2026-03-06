//! API Key Vault — `age`-encrypted file storage for provider API keys.
//!
//! All keys are stored in a single `age` passphrase-encrypted file as a
//! JSON-serialized [`BTreeMap<String, String>`]. At runtime, keys are held in
//! [`ApiKey`] (backed by `SecretString`) and are never logged or written to
//! disk in plaintext.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use modelsentry_common::error::{ModelSentryError, Result};
use modelsentry_common::types::ApiKey;
use secrecy::{ExposeSecret, SecretString};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// A map of provider identifiers to their `age`-encrypted API keys.
///
/// Use [`Vault::create`] to create a new vault file, and [`Vault::open`] to
/// open and verify an existing one.
pub struct Vault {
    path: PathBuf,
    passphrase: SecretString,
}

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vault")
            .field("path", &self.path)
            .field("passphrase", &"[redacted]")
            .finish()
    }
}

impl Vault {
    /// Open an existing vault file and verify the passphrase by attempting a
    /// trial decryption.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Vault`] if the file cannot be read, the
    ///   passphrase is wrong, or the ciphertext is malformed.
    pub fn open(path: &Path, passphrase: SecretString) -> Result<Self> {
        let vault = Self {
            path: path.to_path_buf(),
            passphrase,
        };
        // Trial decryption to validate the passphrase up-front.
        vault.decrypt_vault()?;
        Ok(vault)
    }

    /// Create a new, empty vault file at `path`.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Vault`] if the file cannot be written or
    ///   encryption fails.
    pub fn create(path: &Path, passphrase: SecretString) -> Result<Self> {
        let vault = Self {
            path: path.to_path_buf(),
            passphrase,
        };
        vault.encrypt_and_write(&BTreeMap::new())?;
        Ok(vault)
    }

    /// Retrieve the API key for `provider_id`, or `None` if not set.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Vault`] if decryption fails.
    pub fn get_key(&self, provider_id: &str) -> Result<Option<ApiKey>> {
        let map = self.decrypt_vault()?;
        Ok(map.get(provider_id).map(|k| ApiKey::new(k.clone())))
    }

    /// Store (or overwrite) the API key for `provider_id`.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Vault`] if decryption or re-encryption fails.
    pub fn set_key(&self, provider_id: &str, key: &ApiKey) -> Result<()> {
        let mut map = self.decrypt_vault()?;
        map.insert(provider_id.to_string(), key.expose().to_string());
        self.encrypt_and_write(&map)
    }

    /// Delete the API key for `provider_id`. Returns `true` if the key was
    /// present, `false` if it was not found.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Vault`] if decryption or re-encryption fails.
    pub fn delete_key(&self, provider_id: &str) -> Result<bool> {
        let mut map = self.decrypt_vault()?;
        let removed = map.remove(provider_id).is_some();
        if removed {
            self.encrypt_and_write(&map)?;
        }
        Ok(removed)
    }

    /// List all provider IDs stored in the vault.
    ///
    /// # Errors
    ///
    /// - [`ModelSentryError::Vault`] if decryption fails.
    pub fn list_providers(&self) -> Result<Vec<String>> {
        let map = self.decrypt_vault()?;
        Ok(map.into_keys().collect())
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

impl Vault {
    fn decrypt_vault(&self) -> Result<BTreeMap<String, String>> {
        let ciphertext = std::fs::read(&self.path).map_err(|e| ModelSentryError::Vault {
            message: format!("cannot read vault file: {e}"),
        })?;

        let identity = age::scrypt::Identity::new(SecretString::new(
            self.passphrase.expose_secret().to_string().into(),
        ));

        let decryptor =
            age::Decryptor::new(&ciphertext[..]).map_err(|e| ModelSentryError::Vault {
                message: format!("malformed vault file: {e}"),
            })?;

        let mut reader = decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .map_err(|e| ModelSentryError::Vault {
                message: format!("decryption failed (wrong passphrase?): {e}"),
            })?;

        let mut plaintext = Vec::new();
        reader
            .read_to_end(&mut plaintext)
            .map_err(|e| ModelSentryError::Vault {
                message: format!("cannot read decrypted data: {e}"),
            })?;

        serde_json::from_slice(&plaintext).map_err(|e| ModelSentryError::Vault {
            message: format!("vault data is not valid JSON: {e}"),
        })
    }

    fn encrypt_and_write(&self, data: &BTreeMap<String, String>) -> Result<()> {
        let plaintext = serde_json::to_vec(data).map_err(|e| ModelSentryError::Vault {
            message: format!("cannot serialize vault data: {e}"),
        })?;

        let encryptor = age::Encryptor::with_user_passphrase(SecretString::new(
            self.passphrase.expose_secret().to_string().into(),
        ));

        let mut ciphertext = Vec::new();
        let mut writer =
            encryptor
                .wrap_output(&mut ciphertext)
                .map_err(|e| ModelSentryError::Vault {
                    message: format!("encryption setup failed: {e}"),
                })?;
        writer
            .write_all(&plaintext)
            .map_err(|e| ModelSentryError::Vault {
                message: format!("encryption write failed: {e}"),
            })?;
        writer.finish().map_err(|e| ModelSentryError::Vault {
            message: format!("encryption finalisation failed: {e}"),
        })?;

        std::fs::write(&self.path, &ciphertext).map_err(|e| ModelSentryError::Vault {
            message: format!("cannot write vault file: {e}"),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn passphrase(s: &str) -> SecretString {
        SecretString::new(s.to_string().into())
    }

    /// Round-trip: create vault, write key, reopen, read back.
    #[test]
    fn create_and_reopen_vault_retrieves_key() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.age");

        let vault = Vault::create(&path, passphrase("hunter2")).unwrap();
        vault
            .set_key("openai", &ApiKey::new("sk-test-key".to_string()))
            .unwrap();

        // Reopen with correct passphrase
        let vault2 = Vault::open(&path, passphrase("hunter2")).unwrap();
        let key = vault2.get_key("openai").unwrap().expect("key should exist");
        assert_eq!(key.expose(), "sk-test-key");
    }

    /// Opening with a wrong passphrase must return a `Vault` error.
    #[test]
    fn wrong_passphrase_returns_vault_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.age");
        Vault::create(&path, passphrase("correct")).unwrap();

        let result = Vault::open(&path, passphrase("wrong"));
        assert!(
            matches!(result, Err(ModelSentryError::Vault { .. })),
            "expected Vault error, got {result:?}"
        );
    }

    /// The vault file must not contain the plaintext key bytes.
    #[test]
    fn vault_file_is_opaque_binary_not_plaintext() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.age");
        let vault = Vault::create(&path, passphrase("p@ss")).unwrap();
        vault
            .set_key("anthropic", &ApiKey::new("my-secret-key".to_string()))
            .unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert!(
            !bytes
                .windows("my-secret-key".len())
                .any(|w| w == b"my-secret-key"),
            "vault file must not contain plaintext key"
        );
    }

    /// `delete_key` must return `false` when the key was not present.
    #[test]
    fn delete_key_returns_false_when_absent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.age");
        let vault = Vault::create(&path, passphrase("abc")).unwrap();
        assert!(!vault.delete_key("nonexistent").unwrap());
    }

    /// `set_key` must overwrite an existing key.
    #[test]
    fn set_key_overwrites_existing_key() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.age");
        let vault = Vault::create(&path, passphrase("abc")).unwrap();

        vault
            .set_key("openai", &ApiKey::new("old-key".to_string()))
            .unwrap();
        vault
            .set_key("openai", &ApiKey::new("new-key".to_string()))
            .unwrap();

        let key = vault.get_key("openai").unwrap().unwrap();
        assert_eq!(key.expose(), "new-key");
    }

    /// `list_providers` returns all provider IDs, sorted (`BTreeMap` order).
    #[test]
    fn list_providers_returns_all_ids() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("vault.age");
        let vault = Vault::create(&path, passphrase("abc")).unwrap();

        vault
            .set_key("anthropic", &ApiKey::new("k1".to_string()))
            .unwrap();
        vault
            .set_key("openai", &ApiKey::new("k2".to_string()))
            .unwrap();

        let providers = vault.list_providers().unwrap();
        assert_eq!(providers, vec!["anthropic", "openai"]);
    }
}
