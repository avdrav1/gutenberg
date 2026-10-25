//! Keys for MCP servers, by name (`JINA_API_KEY`): from the environment,
//! else from the system keyring over the Secret Service API (GNOME Keyring,
//! KWallet). The store and `config.toml` only ever hold the name.
//!
//! Keyring items carry the attributes `application=gutenberg` and
//! `key=<name>`, so `secret-tool lookup application gutenberg key <name>`
//! finds one too.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use futures::future::BoxFuture;
use secret_service::{EncryptionType, SecretService};

/// Where keys are kept.
pub trait Keyring: Send + Sync {
    fn get<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<String>, String>>;
    fn set<'a>(&'a self, name: &'a str, value: &'a str) -> BoxFuture<'a, Result<(), String>>;
}

static KEYRING: OnceLock<Box<dyn Keyring>> = OnceLock::new();

fn keyring() -> &'static dyn Keyring {
    KEYRING.get_or_init(|| Box::new(System)).as_ref()
}

/// Keeps keys in memory for the rest of the process instead of the system
/// keyring, for tests. Has no effect once a key has been read or saved.
pub fn use_memory_keyring() {
    let _ = KEYRING.set(Box::new(Memory::default()));
}

/// Whether `name` can name a key: an environment variable's letters,
/// digits and underscores, not starting with a digit.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The key named `name`: the environment variable if it's set and not
/// empty, else the keyring's. `Ok(None)` when neither has it.
pub async fn lookup(name: &str) -> Result<Option<String>, String> {
    if let Some(value) = std::env::var(name).ok().filter(|v| !v.trim().is_empty()) {
        return Ok(Some(value.trim().to_owned()));
    }
    keyring().get(name).await
}

/// Saves `value` in the keyring as `name`, replacing what was there.
pub async fn save(name: &str, value: &str) -> Result<(), String> {
    let value = value.trim();
    if !valid_name(name) {
        return Err(format!("{name:?} isn't a key name"));
    }
    if value.is_empty() {
        return Err("the key is empty".into());
    }
    keyring().set(name, value).await
}

/// The Secret Service on the session bus.
struct System;

fn attributes(name: &str) -> HashMap<&str, &str> {
    HashMap::from([("application", "gutenberg"), ("key", name)])
}

fn keyring_error(e: secret_service::Error) -> String {
    match e {
        secret_service::Error::Unavailable => "no keyring is running".into(),
        secret_service::Error::Locked => "the keyring is locked".into(),
        secret_service::Error::Prompt => "the keyring stayed locked".into(),
        e => format!("the keyring: {e}"),
    }
}

impl Keyring for System {
    fn get<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(async move {
            let service = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(keyring_error)?;
            let found = service
                .search_items(attributes(name))
                .await
                .map_err(keyring_error)?;
            // A refresh never asks to unlock the keyring; saving a key does.
            let Some(item) = found.unlocked.first() else {
                return match found.locked.is_empty() {
                    true => Ok(None),
                    false => Err(keyring_error(secret_service::Error::Locked)),
                };
            };
            let secret = item.get_secret().await.map_err(keyring_error)?;
            String::from_utf8(secret)
                .map(Some)
                .map_err(|_| format!("{name} in the keyring isn't text"))
        })
    }

    fn set<'a>(&'a self, name: &'a str, value: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let service = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(keyring_error)?;
            let collection = service
                .get_default_collection()
                .await
                .map_err(keyring_error)?;
            collection.ensure_unlocked().await.map_err(keyring_error)?;
            collection
                .create_item(
                    &format!("Gutenberg: {name}"),
                    attributes(name),
                    value.as_bytes(),
                    true,
                    "text/plain",
                )
                .await
                .map(|_| ())
                .map_err(keyring_error)
        })
    }
}

#[derive(Default)]
struct Memory(Mutex<HashMap<String, String>>);

impl Keyring for Memory {
    fn get<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<String>, String>> {
        let value = self.0.lock().expect("not poisoned").get(name).cloned();
        Box::pin(async move { Ok(value) })
    }

    fn set<'a>(&'a self, name: &'a str, value: &'a str) -> BoxFuture<'a, Result<(), String>> {
        self.0
            .lock()
            .expect("not poisoned")
            .insert(name.to_owned(), value.to_owned());
        Box::pin(async { Ok(()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_environment_variables() {
        assert!(valid_name("JINA_API_KEY"));
        assert!(valid_name("_x1"));
        assert!(!valid_name(""));
        assert!(!valid_name("1KEY"));
        assert!(!valid_name("MY-KEY"));
        assert!(!valid_name("A B"));
    }
}
