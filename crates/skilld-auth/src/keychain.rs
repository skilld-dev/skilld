#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use crate::model::CredentialStore;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod supported {
    use keyring::{Entry, Error};
    use serde::{Deserialize, Serialize};
    use zeroize::Zeroize;

    use crate::model::{
        BoundaryError, BoundaryErrorKind, CredentialStore, SKILLD_ORIGIN, SecretString,
        StoredCredential,
    };

    const SERVICE: &str = "skilld.dev";
    const ACTIVE_ACCOUNT: &str = "https://skilld.dev:active-account";

    #[derive(Clone, Copy, Debug, Default)]
    pub struct KeychainCredentialStore;

    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct PersistedCredential {
        origin: String,
        account: String,
        access_token: String,
        refresh_token: Option<String>,
        expires_at: u64,
        scopes: Option<String>,
    }

    impl KeychainCredentialStore {
        #[must_use]
        pub const fn new() -> Self {
            Self
        }

        fn entry() -> Result<Entry, BoundaryError> {
            Entry::new(SERVICE, ACTIVE_ACCOUNT).map_err(|_| failed())
        }
    }

    impl CredentialStore for KeychainCredentialStore {
        fn load(&self, origin: &str) -> Result<Option<StoredCredential>, BoundaryError> {
            if origin != SKILLD_ORIGIN {
                return Err(failed());
            }
            let mut encoded = match Self::entry()?.get_password() {
                Ok(value) => value,
                Err(Error::NoEntry) => return Ok(None),
                Err(_) => return Err(failed()),
            };
            let parsed = serde_json::from_str::<PersistedCredential>(&encoded);
            encoded.zeroize();
            let value = parsed.map_err(|_| failed())?;
            if value.origin != origin {
                return Err(failed());
            }
            Ok(Some(StoredCredential {
                origin: value.origin,
                account: value.account,
                access_token: SecretString::new(value.access_token),
                refresh_token: value.refresh_token.map(SecretString::new),
                expires_at: value.expires_at,
                scopes: value.scopes,
            }))
        }

        fn save(&self, credential: &StoredCredential) -> Result<(), BoundaryError> {
            if credential.origin != SKILLD_ORIGIN {
                return Err(failed());
            }
            let entry = Self::entry()?;
            let mut value = PersistedCredential {
                origin: credential.origin.clone(),
                account: credential.account.clone(),
                access_token: credential.access_token.expose_secret().to_owned(),
                refresh_token: credential
                    .refresh_token
                    .as_ref()
                    .map(|token| token.expose_secret().to_owned()),
                expires_at: credential.expires_at,
                scopes: credential.scopes.clone(),
            };
            let encoded = serde_json::to_string(&value);
            value.access_token.zeroize();
            if let Some(token) = &mut value.refresh_token {
                token.zeroize();
            }
            let mut encoded = encoded.map_err(|_| failed())?;
            let saved = entry.set_password(&encoded);
            encoded.zeroize();
            saved.map_err(|_| failed())
        }

        fn delete(&self, origin: &str, account: &str) -> Result<(), BoundaryError> {
            if origin != SKILLD_ORIGIN {
                return Err(failed());
            }
            if self
                .load(origin)?
                .as_ref()
                .is_some_and(|credential| credential.account != account)
            {
                return Err(failed());
            }
            match Self::entry()?.delete_credential() {
                Ok(()) | Err(Error::NoEntry) => Ok(()),
                Err(_) => Err(failed()),
            }
        }
    }

    const fn failed() -> BoundaryError {
        BoundaryError::new(BoundaryErrorKind::Failed)
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub use supported::KeychainCredentialStore;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
#[derive(Clone, Copy, Debug, Default)]
pub struct KeychainCredentialStore;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl KeychainCredentialStore {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl CredentialStore for KeychainCredentialStore {
    fn load(
        &self,
        _origin: &str,
    ) -> Result<Option<crate::model::StoredCredential>, crate::model::BoundaryError> {
        Err(crate::model::BoundaryError::new(
            crate::model::BoundaryErrorKind::Unsupported,
        ))
    }

    fn save(
        &self,
        _credential: &crate::model::StoredCredential,
    ) -> Result<(), crate::model::BoundaryError> {
        Err(crate::model::BoundaryError::new(
            crate::model::BoundaryErrorKind::Unsupported,
        ))
    }

    fn delete(&self, _origin: &str, _account: &str) -> Result<(), crate::model::BoundaryError> {
        Err(crate::model::BoundaryError::new(
            crate::model::BoundaryErrorKind::Unsupported,
        ))
    }
}
