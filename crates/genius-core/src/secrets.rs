//! OS-backed secret storage.
//!
//! * Windows: Credential Manager (`keyring` windows-native)
//! * macOS: Keychain (`keyring` apple-native)
//! * Linux: Secret Service / GNOME Keyring / KWallet (`keyring` sync-secret-service)
//!
//! Secrets never touch SQLite, settings files or the frontend. The frontend can
//! only ask whether a secret exists (and see a masked hint); the raw value is
//! read exclusively inside the Rust backend when a provider request is made.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::error::{GeniusError, Result};
use crate::redact::{register_secret, unregister_secret};

pub const SERVICE: &str = "dev.genius.studio";
pub const NVIDIA_KEY: &str = "nvidia-api-key";

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    fn delete(&self, name: &str) -> Result<()>;
    fn backend_name(&self) -> &'static str;
}

pub struct KeyringStore;

impl KeyringStore {
    fn entry(name: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, name).map_err(|e| GeniusError::Secret(e.to_string()))
    }
}

impl SecretStore for KeyringStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        match Self::entry(name)?.get_password() {
            Ok(v) => {
                register_secret(&v);
                Ok(Some(v))
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(GeniusError::Secret(format!("could not read from the system credential store: {e}"))),
        }
    }

    fn set(&self, name: &str, value: &str) -> Result<()> {
        Self::entry(name)?.set_password(value).map_err(|e| GeniusError::Secret(format!("could not save to the system credential store: {e}")))?;
        register_secret(value);
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<()> {
        let entry = Self::entry(name)?;
        if let Ok(v) = entry.get_password() {
            unregister_secret(&v);
        }
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(GeniusError::Secret(format!("could not remove credential: {e}"))),
        }
    }

    fn backend_name(&self) -> &'static str {
        if cfg!(windows) {
            "Windows Credential Manager"
        } else if cfg!(target_os = "macos") {
            "macOS Keychain"
        } else {
            "Secret Service (system keyring)"
        }
    }
}

/// In-memory store for tests and for environments without a keyring daemon.
#[derive(Default)]
pub struct MemoryStore {
    map: Mutex<HashMap<String, String>>,
}

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(self.map.lock().unwrap().get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        register_secret(value);
        self.map.lock().unwrap().insert(name.into(), value.into());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<()> {
        if let Some(v) = self.map.lock().unwrap().remove(name) {
            unregister_secret(&v);
        }
        Ok(())
    }
    fn backend_name(&self) -> &'static str {
        "in-memory (not persisted)"
    }
}

/// A short, non-reversible hint such as `nvapi-…k3Xq` so users can tell which key is stored.
pub fn mask(value: &str) -> String {
    let tail: String = value.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    let head = if value.starts_with("nvapi-") { "nvapi-" } else { "" };
    format!("{head}…{tail}")
}

pub fn validate_nvidia_key(key: &str) -> Result<String> {
    let k = key.trim();
    if k.is_empty() {
        return Err(GeniusError::Validation("The API key is empty.".into()));
    }
    if k.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(GeniusError::Validation("The API key contains whitespace or control characters.".into()));
    }
    if k.len() < 16 || k.len() > 512 {
        return Err(GeniusError::Validation("The API key length looks wrong. Copy the full key from build.nvidia.com.".into()));
    }
    Ok(k.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_never_reveals_key() {
        let k = "nvapi-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123";
        let m = mask(k);
        assert_eq!(m, "nvapi-…0123");
        assert!(!m.contains("ABCDEF"));
    }

    #[test]
    fn validation_rejects_bad_keys() {
        assert!(validate_nvidia_key("").is_err());
        assert!(validate_nvidia_key("short").is_err());
        assert!(validate_nvidia_key("nvapi-abc def ghijklmnopqrstu").is_err());
        assert_eq!(validate_nvidia_key("  nvapi-abcdefghijklmnopqrstu \n").unwrap(), "nvapi-abcdefghijklmnopqrstu");
    }

    #[test]
    fn memory_store_roundtrip_and_redaction() {
        let s = MemoryStore::default();
        s.set(NVIDIA_KEY, "nvapi-secretvalue-0001").unwrap();
        assert_eq!(s.get(NVIDIA_KEY).unwrap().as_deref(), Some("nvapi-secretvalue-0001"));
        assert!(!crate::redact::redact("k=nvapi-secretvalue-0001").contains("secretvalue"));
        s.delete(NVIDIA_KEY).unwrap();
        assert_eq!(s.get(NVIDIA_KEY).unwrap(), None);
    }
}
