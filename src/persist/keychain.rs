//! Saved passwords in the OS keychain (C5): service `queryx`, account `connection-<id>`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::model::{ConnectionId, Secret};

/// Where saved passwords live; tests use `MemKeychain`. Errors are messages to show.
pub trait Keychain: Send + Sync {
    /// The stored password, `None` when there is no entry.
    fn get(&self, id: ConnectionId) -> Result<Option<Secret>, String>;
    fn set(&self, id: ConnectionId, password: &Secret) -> Result<(), String>;
    /// Removes the entry; no entry is not an error.
    fn delete(&self, id: ConnectionId) -> Result<(), String>;
}

/// The platform keychain (Secret Service, macOS Keychain, Windows Credential Manager).
pub struct OsKeychain;

impl OsKeychain {
    fn entry(id: ConnectionId) -> Result<keyring::Entry, String> {
        keyring::Entry::new("queryx", &format!("connection-{}", id.0)).map_err(|e| e.to_string())
    }
}

impl Keychain for OsKeychain {
    fn get(&self, id: ConnectionId) -> Result<Option<Secret>, String> {
        match Self::entry(id)?.get_password() {
            Ok(password) => Ok(Some(Secret::new(password))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn set(&self, id: ConnectionId, password: &Secret) -> Result<(), String> {
        Self::entry(id)?
            .set_password(password.expose())
            .map_err(|e| e.to_string())
    }

    fn delete(&self, id: ConnectionId) -> Result<(), String> {
        match Self::entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// A keychain in memory, by account name.
#[derive(Default)]
pub struct MemKeychain(pub Mutex<HashMap<String, Secret>>);

impl MemKeychain {
    fn account(id: ConnectionId) -> String {
        format!("connection-{}", id.0)
    }
}

impl Keychain for MemKeychain {
    fn get(&self, id: ConnectionId) -> Result<Option<Secret>, String> {
        Ok(self.0.lock().unwrap().get(&Self::account(id)).cloned())
    }

    fn set(&self, id: ConnectionId, password: &Secret) -> Result<(), String> {
        let mut entries = self.0.lock().unwrap();
        entries.insert(Self::account(id), password.clone());
        Ok(())
    }

    fn delete(&self, id: ConnectionId) -> Result<(), String> {
        self.0.lock().unwrap().remove(&Self::account(id));
        Ok(())
    }
}

/// The app's keychain, and whether a keychain error was already shown.
pub struct Keys {
    pub keychain: Arc<dyn Keychain>,
    warned: AtomicBool,
}

impl gpui_kit::Global for Keys {}

impl Keys {
    pub fn new(keychain: Arc<dyn Keychain>) -> Self {
        Self {
            keychain,
            warned: AtomicBool::new(false),
        }
    }

    /// The installed keychain; without one, an empty throwaway that saves nothing.
    pub fn of(cx: &gpui_kit::App) -> Arc<dyn Keychain> {
        cx.try_global::<Self>().map_or_else(
            || Arc::new(MemKeychain::default()) as _,
            |k| k.keychain.clone(),
        )
    }

    /// The notice for `error`, only the first time one happens.
    pub fn first_error(cx: &gpui_kit::App, error: &str) -> Option<String> {
        let keys = cx.try_global::<Self>()?;
        (!keys.warned.swap(true, Ordering::Relaxed)).then(|| {
            format!("The keychain is unavailable ({error}); passwords will be asked on connect.")
        })
    }
}
