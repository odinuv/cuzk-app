//! API key storage. The key lives only in the OS credential store, never in SQLite,
//! logs or error messages.

use crate::error::CoreError;

type Result<T> = std::result::Result<T, CoreError>;

const SERVICE: &str = "strazce-katastru";
const USER: &str = "api-key";

pub trait SecretStore: Send + Sync {
    /// `Ok(None)` when no key is stored.
    fn get(&self) -> Result<Option<String>>;
    fn set(&self, key: &str) -> Result<()>;
    /// Succeeds when there is nothing to delete.
    fn delete(&self) -> Result<()>;
}

/// Windows Credential Manager (via `keyring`). keyring v3 names the generic credential
/// `<user>.<service>`, i.e. `api-key.strazce-katastru` (that is what the README tells users).
#[derive(Debug, Default, Clone, Copy)]
pub struct KeyringStore;

fn err(what: &str) -> CoreError {
    // Fixed text on purpose: keyring error values can carry the stored bytes.
    CoreError::Db(format!("Úložiště přihlašovacích údajů: {what}"))
}

fn entry() -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, USER).map_err(|_| err("položku nelze otevřít"))
}

impl SecretStore for KeyringStore {
    fn get(&self) -> Result<Option<String>> {
        match entry()?.get_password() {
            Ok(k) => Ok(Some(k)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(err("klíč nelze přečíst")),
        }
    }

    fn set(&self, key: &str) -> Result<()> {
        entry()?
            .set_password(key)
            .map_err(|_| err("klíč nelze uložit"))
    }

    fn delete(&self) -> Result<()> {
        match entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(err("klíč nelze smazat")),
        }
    }
}

#[cfg(any(test, feature = "test-util"))]
pub use mem::MemoryStore;

#[cfg(any(test, feature = "test-util"))]
mod mem {
    use super::*;
    use std::sync::Mutex;

    /// In-memory store for tests.
    #[derive(Debug, Default)]
    pub struct MemoryStore(Mutex<Option<String>>);

    impl SecretStore for MemoryStore {
        fn get(&self) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().clone())
        }
        fn set(&self, key: &str) -> Result<()> {
            *self.0.lock().unwrap() = Some(key.to_string());
            Ok(())
        }
        fn delete(&self) -> Result<()> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip() {
        let s = MemoryStore::default();
        assert_eq!(s.get().unwrap(), None);
        s.set("k").unwrap();
        assert_eq!(s.get().unwrap().as_deref(), Some("k"));
        s.delete().unwrap();
        s.delete().unwrap();
        assert_eq!(s.get().unwrap(), None);
    }
}
