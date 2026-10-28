//! Keys for MCP servers, by name (`JINA_API_KEY`): from the environment,
//! else from the system keyring. On Linux that's the Secret Service API
//! (GNOME Keyring, KWallet); on macOS, the login keychain. The store and
//! `config.toml` only ever hold the name.
//!
//! Keyring items carry the attributes `application=gutenberg` and
//! `key=<name>`, so `secret-tool lookup application gutenberg key <name>`
//! finds one too. Keychain items are generic passwords with service
//! `gutenberg` and account `<name>`, so
//! `security find-generic-password -s gutenberg -a <name> -w` finds one.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use futures::future::BoxFuture;
#[cfg(target_os = "linux")]
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

/// The system keyring: the Secret Service on the session bus on Linux, the
/// login keychain on macOS.
struct System;

#[cfg(target_os = "linux")]
fn attributes(name: &str) -> HashMap<&str, &str> {
    HashMap::from([("application", "gutenberg"), ("key", name)])
}

#[cfg(target_os = "linux")]
fn keyring_error(e: secret_service::Error) -> String {
    match e {
        secret_service::Error::Unavailable => "no keyring is running".into(),
        secret_service::Error::Locked => "the keyring is locked".into(),
        secret_service::Error::Prompt => "the keyring stayed locked".into(),
        e => format!("the keyring: {e}"),
    }
}

#[cfg(target_os = "linux")]
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

/// The keychain item's service; the account is the key's name.
#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "gutenberg";

/// `errSecItemNotFound`: no item has that service and account.
#[cfg(target_os = "macos")]
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

/// `errSecUserCanceled`: the keychain asked for its password and the user
/// cancelled.
#[cfg(target_os = "macos")]
const ERR_SEC_USER_CANCELED: i32 = -128;

/// `errSecAuthFailed`: the user chose Deny when the keychain asked whether
/// this binary may read the item. The framework's own text for it talks of
/// a wrong password, which the user never typed.
#[cfg(target_os = "macos")]
const ERR_SEC_AUTH_FAILED: i32 = -25293;

/// `errSecInteractionNotAllowed`: the keychain needed to ask and had no
/// session to ask in, as from `gutenberg --refresh` over ssh.
#[cfg(target_os = "macos")]
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;

#[cfg(target_os = "macos")]
fn keyring_error(e: security_framework::base::Error) -> String {
    match e.code() {
        ERR_SEC_USER_CANCELED => "the keychain stayed locked".into(),
        ERR_SEC_AUTH_FAILED => {
            "the keychain refused this build; choose Always Allow when it asks".into()
        }
        ERR_SEC_INTERACTION_NOT_ALLOWED => {
            "the keychain can only ask in a logged-in session; run this from the desktop".into()
        }
        _ => format!("the keychain: {e}"),
    }
}

// The Security framework's calls block and may show a dialog: the keychain
// asks before a binary it hasn't seen reads an item, and an ad-hoc signed
// build is a new binary after every update. So they run on the blocking
// pool, off the async threads.
#[cfg(target_os = "macos")]
impl Keyring for System {
    fn get<'a>(&'a self, name: &'a str) -> BoxFuture<'a, Result<Option<String>, String>> {
        Box::pin(async move {
            let account = name.to_owned();
            let found = tokio::task::spawn_blocking(move || {
                security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, &account)
            })
            .await
            .map_err(|e| format!("the keychain: {e}"))?;
            match found {
                Ok(secret) => String::from_utf8(secret)
                    .map(Some)
                    .map_err(|_| format!("{name} in the keychain isn't text")),
                Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
                Err(e) => Err(keyring_error(e)),
            }
        })
    }

    fn set<'a>(&'a self, name: &'a str, value: &'a str) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let account = name.to_owned();
            let secret = value.as_bytes().to_vec();
            // Creates the item, or replaces the password of the one there.
            tokio::task::spawn_blocking(move || {
                security_framework::passwords::set_generic_password(
                    KEYCHAIN_SERVICE,
                    &account,
                    &secret,
                )
            })
            .await
            .map_err(|e| format!("the keychain: {e}"))?
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
