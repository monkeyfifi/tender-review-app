use crate::error::AppError;

const SERVICE_NAME: &str = "cn.local.tenderreview";
const ACCOUNT_NAME: &str = "model-api-key";

pub trait CredentialStore: Send + Sync {
    fn save_key(&self, api_key: &str) -> Result<(), AppError>;
    fn load_key(&self) -> Result<Option<String>, AppError>;
    fn delete_key(&self) -> Result<(), AppError>;
}

pub struct KeyringCredentialStore;

impl CredentialStore for KeyringCredentialStore {
    fn save_key(&self, api_key: &str) -> Result<(), AppError> {
        keyring::Entry::new(SERVICE_NAME, ACCOUNT_NAME)
            .map_err(AppError::credential)?
            .set_password(api_key)
            .map_err(AppError::credential)
    }

    fn load_key(&self) -> Result<Option<String>, AppError> {
        let entry =
            keyring::Entry::new(SERVICE_NAME, ACCOUNT_NAME).map_err(AppError::credential)?;
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(AppError::credential(error)),
        }
    }

    fn delete_key(&self) -> Result<(), AppError> {
        let entry =
            keyring::Entry::new(SERVICE_NAME, ACCOUNT_NAME).map_err(AppError::credential)?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(AppError::credential(error)),
        }
    }
}

#[cfg(test)]
use std::sync::Mutex;

#[cfg(test)]
pub struct MemoryCredentialStore(Mutex<Option<String>>);

#[cfg(test)]
impl MemoryCredentialStore {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }
}

#[cfg(test)]
impl Default for MemoryCredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
impl CredentialStore for MemoryCredentialStore {
    fn save_key(&self, api_key: &str) -> Result<(), AppError> {
        *self.0.lock().map_err(AppError::credential)? = Some(api_key.to_owned());
        Ok(())
    }

    fn load_key(&self) -> Result<Option<String>, AppError> {
        Ok(self.0.lock().map_err(AppError::credential)?.clone())
    }

    fn delete_key(&self) -> Result<(), AppError> {
        *self.0.lock().map_err(AppError::credential)? = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_credential_store_saves_loads_and_deletes_a_key() {
        let store = MemoryCredentialStore::new();

        assert_eq!(store.load_key().unwrap(), None);
        store.save_key("sk-secret-value").unwrap();
        assert_eq!(
            store.load_key().unwrap().as_deref(),
            Some("sk-secret-value")
        );
        store.delete_key().unwrap();
        assert_eq!(store.load_key().unwrap(), None);
    }

    #[test]
    fn memory_credential_store_default_starts_without_a_key() {
        let store = MemoryCredentialStore::default();

        assert_eq!(store.load_key().unwrap(), None);
    }
}
