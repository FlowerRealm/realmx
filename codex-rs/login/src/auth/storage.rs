use chrono::DateTime;
use chrono::Utc;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashMap;
use std::fmt::Debug;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Read;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use tracing::warn;

use crate::token_data::TokenData;
use codex_app_server_protocol::AuthMode;
use codex_keyring_store::DefaultKeyringStore;
use codex_keyring_store::KeyringStore;
use once_cell::sync::Lazy;

/// Determine where Codex should store CLI auth credentials.
#[derive(Debug, Default, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum AuthCredentialsStoreMode {
    #[default]
    /// Persist credentials in CODEX_HOME/auth.json.
    File,
    /// Persist credentials in the keyring. Fail if unavailable.
    Keyring,
    /// Use keyring when available; otherwise, fall back to a file in CODEX_HOME.
    Auto,
    /// Store credentials in memory only for the current process.
    Ephemeral,
}

/// Expected structure for $CODEX_HOME/auth.json.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct AuthDotJson {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<AuthMode>,

    #[serde(
        default,
        alias = "OPENAI_API_KEY",
        skip_serializing_if = "Option::is_none"
    )]
    pub api_key: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<TokenData>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum AuthScope {
    #[default]
    Default,
    Provider(String),
}

impl AuthScope {
    pub fn from_provider_id(provider_id: Option<&str>) -> Self {
        match provider_id {
            Some(provider_id) => Self::Provider(provider_id.to_string()),
            None => Self::Default,
        }
    }

    pub fn provider(provider_id: impl Into<String>) -> Self {
        Self::Provider(provider_id.into())
    }

    pub fn provider_id(&self) -> Option<&str> {
        match self {
            Self::Default => None,
            Self::Provider(provider_id) => Some(provider_id.as_str()),
        }
    }
}

/// Provider-aware structure for $CODEX_HOME/auth.json.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
pub struct AuthStoreJson {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<AuthDotJson>,

    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub providers: HashMap<String, AuthDotJson>,
}

impl AuthStoreJson {
    pub fn from_default(auth: AuthDotJson) -> Self {
        Self {
            default: Some(auth),
            providers: HashMap::new(),
        }
    }

    pub fn get(&self, scope: &AuthScope) -> Option<&AuthDotJson> {
        match scope {
            AuthScope::Default => self.default.as_ref(),
            AuthScope::Provider(provider_id) => self.providers.get(provider_id),
        }
    }

    pub fn upsert(&mut self, scope: &AuthScope, auth: AuthDotJson) {
        match scope {
            AuthScope::Default => {
                self.default = Some(auth);
            }
            AuthScope::Provider(provider_id) => {
                self.providers.insert(provider_id.clone(), auth);
            }
        }
    }

    pub fn remove(&mut self, scope: &AuthScope) -> bool {
        match scope {
            AuthScope::Default => self.default.take().is_some(),
            AuthScope::Provider(provider_id) => self.providers.remove(provider_id).is_some(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.default.is_none() && self.providers.is_empty()
    }
}

pub(super) fn get_auth_file(codex_home: &Path) -> PathBuf {
    codex_home.join("auth.json")
}

pub(super) fn delete_file_if_exists(codex_home: &Path) -> std::io::Result<bool> {
    let auth_file = get_auth_file(codex_home);
    match std::fs::remove_file(&auth_file) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

pub(super) trait AuthStorageBackend: Debug + Send + Sync {
    fn load(&self) -> std::io::Result<Option<AuthStoreJson>>;
    fn save(&self, auth: &AuthStoreJson) -> std::io::Result<()>;
    fn delete(&self) -> std::io::Result<bool>;
}

#[derive(Clone, Debug)]
pub(super) struct FileAuthStorage {
    codex_home: PathBuf,
}

impl FileAuthStorage {
    pub(super) fn new(codex_home: PathBuf) -> Self {
        Self { codex_home }
    }

    /// Attempt to read and parse the `auth.json` file in the given `CODEX_HOME` directory.
    /// Supports both the current provider-aware envelope and the legacy flat auth payload.
    pub(super) fn try_read_auth_store_json(
        &self,
        auth_file: &Path,
    ) -> std::io::Result<AuthStoreJson> {
        let mut file = File::open(auth_file)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        parse_auth_store_json(&contents)
    }
}

fn parse_auth_store_json(contents: &str) -> std::io::Result<AuthStoreJson> {
    let value = serde_json::from_str::<serde_json::Value>(contents).map_err(|err| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("failed to parse auth.json: {err}"),
        )
    })?;

    let is_provider_aware = matches!(
        &value,
        serde_json::Value::Object(map)
            if map.contains_key("default") || map.contains_key("providers")
    );

    if is_provider_aware {
        return serde_json::from_value(value).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to parse provider-aware auth.json: {err}"),
            )
        });
    }

    serde_json::from_value(value)
        .map(AuthStoreJson::from_default)
        .map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("failed to parse legacy auth.json: {err}"),
            )
        })
}

impl AuthStorageBackend for FileAuthStorage {
    fn load(&self) -> std::io::Result<Option<AuthStoreJson>> {
        let auth_file = get_auth_file(&self.codex_home);
        let auth_store_json = match self.try_read_auth_store_json(&auth_file) {
            Ok(auth) => auth,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        Ok(Some(auth_store_json))
    }

    fn save(&self, auth_store_json: &AuthStoreJson) -> std::io::Result<()> {
        let auth_file = get_auth_file(&self.codex_home);

        if let Some(parent) = auth_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json_data = serde_json::to_string_pretty(auth_store_json)?;
        let mut options = OpenOptions::new();
        options.truncate(true).write(true).create(true);
        #[cfg(unix)]
        {
            options.mode(0o600);
        }
        let mut file = options.open(auth_file)?;
        file.write_all(json_data.as_bytes())?;
        file.flush()?;
        Ok(())
    }

    fn delete(&self) -> std::io::Result<bool> {
        delete_file_if_exists(&self.codex_home)
    }
}

const KEYRING_SERVICE: &str = "Codex Auth";

// turns codex_home path into a stable, short key string
fn compute_store_key(codex_home: &Path) -> std::io::Result<String> {
    let canonical = codex_home
        .canonicalize()
        .unwrap_or_else(|_| codex_home.to_path_buf());
    let path_str = canonical.to_string_lossy();
    let mut hasher = Sha256::new();
    hasher.update(path_str.as_bytes());
    let digest = hasher.finalize();
    let hex = format!("{digest:x}");
    let truncated = hex.get(..16).unwrap_or(&hex);
    Ok(format!("cli|{truncated}"))
}

#[derive(Clone, Debug)]
struct KeyringAuthStorage {
    codex_home: PathBuf,
    keyring_store: Arc<dyn KeyringStore>,
}

impl KeyringAuthStorage {
    fn new(codex_home: PathBuf, keyring_store: Arc<dyn KeyringStore>) -> Self {
        Self {
            codex_home,
            keyring_store,
        }
    }

    fn load_from_keyring(&self, key: &str) -> std::io::Result<Option<AuthStoreJson>> {
        match self.keyring_store.load(KEYRING_SERVICE, key) {
            Ok(Some(serialized)) => parse_auth_store_json(&serialized).map(Some).map_err(|err| {
                std::io::Error::other(format!(
                    "failed to deserialize CLI auth from keyring: {err}"
                ))
            }),
            Ok(None) => Ok(None),
            Err(error) => Err(std::io::Error::other(format!(
                "failed to load CLI auth from keyring: {}",
                error.message()
            ))),
        }
    }

    fn save_to_keyring(&self, key: &str, value: &str) -> std::io::Result<()> {
        match self.keyring_store.save(KEYRING_SERVICE, key, value) {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = format!(
                    "failed to write OAuth tokens to keyring: {}",
                    error.message()
                );
                warn!("{message}");
                Err(std::io::Error::other(message))
            }
        }
    }
}

impl AuthStorageBackend for KeyringAuthStorage {
    fn load(&self) -> std::io::Result<Option<AuthStoreJson>> {
        let key = compute_store_key(&self.codex_home)?;
        self.load_from_keyring(&key)
    }

    fn save(&self, auth: &AuthStoreJson) -> std::io::Result<()> {
        let key = compute_store_key(&self.codex_home)?;
        let serialized = serde_json::to_string(auth).map_err(std::io::Error::other)?;
        self.save_to_keyring(&key, &serialized)?;
        if let Err(err) = delete_file_if_exists(&self.codex_home) {
            warn!("failed to remove CLI auth fallback file: {err}");
        }
        Ok(())
    }

    fn delete(&self) -> std::io::Result<bool> {
        let key = compute_store_key(&self.codex_home)?;
        let keyring_removed = self
            .keyring_store
            .delete(KEYRING_SERVICE, &key)
            .map_err(|err| {
                std::io::Error::other(format!("failed to delete auth from keyring: {err}"))
            })?;
        let file_removed = delete_file_if_exists(&self.codex_home)?;
        Ok(keyring_removed || file_removed)
    }
}

#[derive(Clone, Debug)]
struct AutoAuthStorage {
    keyring_storage: Arc<KeyringAuthStorage>,
    file_storage: Arc<FileAuthStorage>,
}

impl AutoAuthStorage {
    fn new(codex_home: PathBuf, keyring_store: Arc<dyn KeyringStore>) -> Self {
        Self {
            keyring_storage: Arc::new(KeyringAuthStorage::new(codex_home.clone(), keyring_store)),
            file_storage: Arc::new(FileAuthStorage::new(codex_home)),
        }
    }
}

impl AuthStorageBackend for AutoAuthStorage {
    fn load(&self) -> std::io::Result<Option<AuthStoreJson>> {
        match self.keyring_storage.load() {
            Ok(Some(auth)) => Ok(Some(auth)),
            Ok(None) => self.file_storage.load(),
            Err(err) => {
                warn!("failed to load CLI auth from keyring, falling back to file storage: {err}");
                self.file_storage.load()
            }
        }
    }

    fn save(&self, auth: &AuthStoreJson) -> std::io::Result<()> {
        match self.keyring_storage.save(auth) {
            Ok(()) => Ok(()),
            Err(err) => {
                warn!("failed to save auth to keyring, falling back to file storage: {err}");
                self.file_storage.save(auth)
            }
        }
    }

    fn delete(&self) -> std::io::Result<bool> {
        // Keyring storage will delete from disk as well
        self.keyring_storage.delete()
    }
}

// A global in-memory store for mapping codex_home -> AuthStoreJson.
static EPHEMERAL_AUTH_STORE: Lazy<Mutex<HashMap<String, AuthStoreJson>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Clone, Debug)]
struct EphemeralAuthStorage {
    codex_home: PathBuf,
}

impl EphemeralAuthStorage {
    fn new(codex_home: PathBuf) -> Self {
        Self { codex_home }
    }

    fn with_store<F, T>(&self, action: F) -> std::io::Result<T>
    where
        F: FnOnce(&mut HashMap<String, AuthStoreJson>, String) -> std::io::Result<T>,
    {
        let key = compute_store_key(&self.codex_home)?;
        let mut store = EPHEMERAL_AUTH_STORE
            .lock()
            .map_err(|_| std::io::Error::other("failed to lock ephemeral auth storage"))?;
        action(&mut store, key)
    }
}

impl AuthStorageBackend for EphemeralAuthStorage {
    fn load(&self) -> std::io::Result<Option<AuthStoreJson>> {
        self.with_store(|store, key| Ok(store.get(&key).cloned()))
    }

    fn save(&self, auth: &AuthStoreJson) -> std::io::Result<()> {
        self.with_store(|store, key| {
            store.insert(key, auth.clone());
            Ok(())
        })
    }

    fn delete(&self) -> std::io::Result<bool> {
        self.with_store(|store, key| Ok(store.remove(&key).is_some()))
    }
}

pub(super) fn create_auth_storage(
    codex_home: PathBuf,
    mode: AuthCredentialsStoreMode,
) -> Arc<dyn AuthStorageBackend> {
    let keyring_store: Arc<dyn KeyringStore> = Arc::new(DefaultKeyringStore);
    create_auth_storage_with_keyring_store(codex_home, mode, keyring_store)
}

fn create_auth_storage_with_keyring_store(
    codex_home: PathBuf,
    mode: AuthCredentialsStoreMode,
    keyring_store: Arc<dyn KeyringStore>,
) -> Arc<dyn AuthStorageBackend> {
    match mode {
        AuthCredentialsStoreMode::File => Arc::new(FileAuthStorage::new(codex_home)),
        AuthCredentialsStoreMode::Keyring => {
            Arc::new(KeyringAuthStorage::new(codex_home, keyring_store))
        }
        AuthCredentialsStoreMode::Auto => Arc::new(AutoAuthStorage::new(codex_home, keyring_store)),
        AuthCredentialsStoreMode::Ephemeral => Arc::new(EphemeralAuthStorage::new(codex_home)),
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
