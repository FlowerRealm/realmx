use crate::auth::AuthCredentialsStoreMode;
use crate::auth::AuthScope;
use crate::auth::CodexAuth;
use crate::auth::load_auth_dot_json_for_exact_scope;
use crate::auth::load_auth_selection_for_scope;
use crate::auth::login_with_api_key_for_scope;
use crate::auth::logout_for_scope;
use crate::config::CONFIG_TOML_FILE;
use crate::config::ConfigToml;
use crate::config::deserialize_config_toml_with_base;
use crate::config::edit::ConfigEdit;
use crate::config::edit::ConfigEditsBuilder;
use crate::error::CodexErr;
use crate::error::Result;
use crate::model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
use crate::model_provider_info::ModelProviderInfo;
use crate::model_provider_info::OLLAMA_OSS_PROVIDER_ID;
use crate::model_provider_info::OPENAI_PROVIDER_ID;
use crate::provider_login_capabilities::provider_login_capabilities;
use crate::provider_login_capabilities::provider_oauth_url;
use codex_rmcp_client::OAuthCredentialsStoreMode;
use codex_rmcp_client::delete_oauth_tokens;
use codex_rmcp_client::has_oauth_tokens;
use codex_rmcp_client::load_oauth_access_token;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCredentialMode {
    ApiKey,
    Chatgpt,
    OAuth,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedProviderCredential {
    pub auth_mode: Option<ProviderCredentialMode>,
    pub token: Option<String>,
    pub account_id: Option<String>,
}

const PROVIDER_API_KEY_STORE_MODE: AuthCredentialsStoreMode = AuthCredentialsStoreMode::File;

fn credential_mode_from_openai_auth(auth: &CodexAuth) -> ProviderCredentialMode {
    match auth.api_auth_mode() {
        codex_app_server_protocol::AuthMode::ApiKey => ProviderCredentialMode::ApiKey,
        codex_app_server_protocol::AuthMode::Oauth => {
            unreachable!("provider OAuth credentials are not represented by CodexAuth")
        }
        codex_app_server_protocol::AuthMode::Chatgpt
        | codex_app_server_protocol::AuthMode::ChatgptAuthTokens => ProviderCredentialMode::Chatgpt,
    }
}

fn openai_auth_credential(auth: Option<CodexAuth>) -> Result<ResolvedProviderCredential> {
    match auth {
        Some(auth) => Ok(ResolvedProviderCredential {
            auth_mode: Some(credential_mode_from_openai_auth(&auth)),
            token: Some(auth.get_token()?),
            account_id: auth.get_account_id(),
        }),
        None => Ok(ResolvedProviderCredential::default()),
    }
}

fn provider_oauth_server_name(provider_id: &str) -> String {
    format!("model-provider:{provider_id}")
}

fn is_custom_provider(provider_id: &str) -> bool {
    !matches!(
        provider_id,
        OPENAI_PROVIDER_ID | OLLAMA_OSS_PROVIDER_ID | LMSTUDIO_OSS_PROVIDER_ID
    )
}

fn config_error(err: std::io::Error) -> CodexErr {
    CodexErr::Io(io::Error::other(err.to_string()))
}

fn read_config_toml(codex_home: &Path) -> Result<ConfigToml> {
    let config_path = codex_home.join(CONFIG_TOML_FILE);
    if !config_path.exists() {
        return Ok(ConfigToml::default());
    }

    let contents = std::fs::read_to_string(&config_path).map_err(config_error)?;
    let root_value = toml::from_str::<toml::Value>(&contents)
        .map_err(|err| config_error(io::Error::new(io::ErrorKind::InvalidData, err)))?;
    deserialize_config_toml_with_base(root_value, codex_home).map_err(config_error)
}

fn load_config_provider(codex_home: &Path, provider_id: &str) -> Result<Option<ModelProviderInfo>> {
    Ok(read_config_toml(codex_home)?
        .model_providers
        .remove(provider_id))
}

fn require_config_provider(codex_home: &Path, provider_id: &str) -> Result<()> {
    if load_config_provider(codex_home, provider_id)?.is_some() {
        return Ok(());
    }

    Err(CodexErr::Io(io::Error::new(
        io::ErrorKind::NotFound,
        format!("provider `{provider_id}` not found in config.toml"),
    )))
}

fn provider_scope(provider_id: &str) -> AuthScope {
    AuthScope::provider(provider_id.to_string())
}

fn clear_legacy_provider_api_key_from_config(codex_home: &Path, provider_id: &str) -> Result<bool> {
    let Some(mut provider) = load_config_provider(codex_home, provider_id)? else {
        return Ok(false);
    };

    if provider.inline_api_key().is_none() {
        return Ok(false);
    }

    provider.api_key = None;
    ConfigEditsBuilder::new(codex_home)
        .with_edits(vec![ConfigEdit::SetModelProvider {
            id: provider_id.to_string(),
            provider: Box::new(provider),
        }])
        .apply_blocking()
        .map_err(|err: anyhow::Error| CodexErr::Io(io::Error::other(err.to_string())))?;
    Ok(true)
}

fn migrate_legacy_provider_api_key(codex_home: &Path, provider_id: &str) -> Result<Option<String>> {
    let scope = provider_scope(provider_id);
    let stored_api_key =
        load_auth_dot_json_for_exact_scope(codex_home, &scope, PROVIDER_API_KEY_STORE_MODE)
            .map_err(config_error)?
            .and_then(|auth| auth.api_key);
    if stored_api_key.is_some() {
        let _ = clear_legacy_provider_api_key_from_config(codex_home, provider_id)?;
        return Ok(None);
    }

    let Some(provider) = load_config_provider(codex_home, provider_id)? else {
        return Ok(None);
    };
    let Some(api_key) = provider.inline_api_key() else {
        return Ok(None);
    };

    login_with_api_key_for_scope(codex_home, &scope, &api_key, PROVIDER_API_KEY_STORE_MODE)
        .map_err(config_error)?;
    let _ = clear_legacy_provider_api_key_from_config(codex_home, provider_id)?;
    Ok(Some(api_key))
}

fn load_explicit_provider_api_key(codex_home: &Path, provider_id: &str) -> Result<Option<String>> {
    let _ = migrate_legacy_provider_api_key(codex_home, provider_id)?;
    load_auth_dot_json_for_exact_scope(
        codex_home,
        &provider_scope(provider_id),
        PROVIDER_API_KEY_STORE_MODE,
    )
    .map(|auth| auth.and_then(|auth| auth.api_key))
    .map_err(config_error)
}

fn resolve_provider_api_key(
    codex_home: &Path,
    provider_id: &str,
) -> Result<Option<(AuthScope, String)>> {
    let _ = migrate_legacy_provider_api_key(codex_home, provider_id)?;
    load_auth_selection_for_scope(
        codex_home,
        &provider_scope(provider_id),
        PROVIDER_API_KEY_STORE_MODE,
    )
    .map(|selection| {
        selection.and_then(|(scope, auth)| auth.api_key.map(|api_key| (scope, api_key)))
    })
    .map_err(config_error)
}

pub fn read_provider_api_key(codex_home: &Path, provider_id: &str) -> Result<Option<String>> {
    load_explicit_provider_api_key(codex_home, provider_id)
}

pub fn store_provider_api_key(codex_home: &Path, provider_id: &str, api_key: &str) -> Result<()> {
    require_config_provider(codex_home, provider_id)?;
    login_with_api_key_for_scope(
        codex_home,
        &provider_scope(provider_id),
        api_key.trim(),
        PROVIDER_API_KEY_STORE_MODE,
    )
    .map_err(config_error)?;
    let _ = clear_legacy_provider_api_key_from_config(codex_home, provider_id)?;
    Ok(())
}

pub fn clear_provider_api_key(codex_home: &Path, provider_id: &str) -> Result<bool> {
    let removed_auth = logout_for_scope(
        codex_home,
        &provider_scope(provider_id),
        PROVIDER_API_KEY_STORE_MODE,
    )
    .map_err(config_error)?;
    let removed_legacy = clear_legacy_provider_api_key_from_config(codex_home, provider_id)?;
    Ok(removed_auth || removed_legacy)
}

pub fn rename_provider_api_key(
    codex_home: &Path,
    old_provider_id: &str,
    new_provider_id: &str,
) -> Result<bool> {
    let Some(api_key) = read_provider_api_key(codex_home, old_provider_id)? else {
        return Ok(false);
    };

    store_provider_api_key(codex_home, new_provider_id, &api_key)?;
    if old_provider_id != new_provider_id {
        let _ = clear_provider_api_key(codex_home, old_provider_id)?;
    }
    Ok(true)
}

fn clear_resolved_provider_api_key(codex_home: &Path, provider_id: &str) -> Result<bool> {
    let Some((scope, _)) = resolve_provider_api_key(codex_home, provider_id)? else {
        return clear_legacy_provider_api_key_from_config(
            codex_home,
            provider_id,
        );
    };

    logout_for_scope(codex_home, &scope, PROVIDER_API_KEY_STORE_MODE).map_err(config_error)
}

pub fn has_provider_oauth_tokens(
    provider_id: &str,
    provider: &ModelProviderInfo,
    oauth_store_mode: OAuthCredentialsStoreMode,
) -> Result<bool> {
    let Some(url) = provider_oauth_url(provider_id, provider) else {
        return Ok(false);
    };
    has_oauth_tokens(
        &provider_oauth_server_name(provider_id),
        url,
        oauth_store_mode,
    )
    .map_err(|err| CodexErr::Io(io::Error::other(err.to_string())))
}

pub fn detect_provider_credential_mode(
    codex_home: &Path,
    provider_id: &str,
    provider: &ModelProviderInfo,
    auth: Option<&CodexAuth>,
    oauth_store_mode: OAuthCredentialsStoreMode,
) -> Result<Option<ProviderCredentialMode>> {
    let config_provider = load_config_provider(codex_home, provider_id)?;
    let provider = config_provider.as_ref().unwrap_or(provider);
    let capabilities = provider_login_capabilities(provider_id, provider);
    let explicit_provider_api_key = load_explicit_provider_api_key(codex_home, provider_id)?;
    let resolved_provider_api_key = resolve_provider_api_key(codex_home, provider_id)?;

    if is_custom_provider(provider_id)
        && capabilities.api_key
        && explicit_provider_api_key.is_some()
    {
        return Ok(Some(ProviderCredentialMode::ApiKey));
    }

    if capabilities.uses_openai_auth() {
        return Ok(auth.map(credential_mode_from_openai_auth));
    }

    if resolved_provider_api_key.is_some() {
        return Ok(Some(ProviderCredentialMode::ApiKey));
    }

    if capabilities.oauth && has_provider_oauth_tokens(provider_id, provider, oauth_store_mode)? {
        return Ok(Some(ProviderCredentialMode::OAuth));
    }

    if capabilities.api_key && provider.api_key_from_env()?.is_some() {
        return Ok(Some(ProviderCredentialMode::ApiKey));
    }

    Ok(None)
}

pub fn clear_provider_oauth_tokens(
    provider_id: &str,
    provider: &ModelProviderInfo,
    oauth_store_mode: OAuthCredentialsStoreMode,
) -> Result<bool> {
    let Some(url) = provider_oauth_url(provider_id, provider) else {
        return Ok(false);
    };
    delete_oauth_tokens(
        &provider_oauth_server_name(provider_id),
        url,
        oauth_store_mode,
    )
    .map_err(|err| CodexErr::Io(io::Error::other(err.to_string())))
}

pub fn activate_provider_api_key(
    codex_home: &Path,
    provider_id: &str,
    provider: &ModelProviderInfo,
    oauth_store_mode: OAuthCredentialsStoreMode,
    api_key: &str,
) -> Result<()> {
    store_provider_api_key(codex_home, provider_id, api_key)?;
    let _ = clear_provider_oauth_tokens(provider_id, provider, oauth_store_mode)?;
    Ok(())
}

pub fn clear_provider_credentials(
    codex_home: &Path,
    provider_id: &str,
    provider: &ModelProviderInfo,
    oauth_store_mode: OAuthCredentialsStoreMode,
) -> Result<bool> {
    let removed_api_key = clear_resolved_provider_api_key(codex_home, provider_id)?;
    let removed_oauth = clear_provider_oauth_tokens(provider_id, provider, oauth_store_mode)?;
    Ok(removed_api_key || removed_oauth)
}

pub async fn resolve_provider_credential(
    codex_home: &Path,
    provider_id: &str,
    provider: &ModelProviderInfo,
    auth: Option<CodexAuth>,
    oauth_store_mode: OAuthCredentialsStoreMode,
) -> Result<ResolvedProviderCredential> {
    let config_provider = load_config_provider(codex_home, provider_id)?;
    let provider = config_provider.as_ref().unwrap_or(provider);
    let capabilities = provider_login_capabilities(provider_id, provider);
    let explicit_provider_api_key = load_explicit_provider_api_key(codex_home, provider_id)?;
    let resolved_provider_api_key = resolve_provider_api_key(codex_home, provider_id)?;

    if is_custom_provider(provider_id)
        && capabilities.api_key
        && let Some(api_key) = explicit_provider_api_key
    {
        return Ok(ResolvedProviderCredential {
            auth_mode: Some(ProviderCredentialMode::ApiKey),
            token: Some(api_key),
            account_id: None,
        });
    }

    if capabilities.uses_openai_auth() {
        return openai_auth_credential(auth);
    }

    if let Some((_, api_key)) = resolved_provider_api_key {
        return Ok(ResolvedProviderCredential {
            auth_mode: Some(ProviderCredentialMode::ApiKey),
            token: Some(api_key),
            account_id: None,
        });
    }

    if capabilities.oauth
        && let Some(url) = provider_oauth_url(provider_id, provider)
    {
        let token = load_oauth_access_token(
            &provider_oauth_server_name(provider_id),
            url,
            oauth_store_mode,
            provider.http_headers.clone(),
            provider.env_http_headers.clone(),
        )
        .await
        .map_err(|err| CodexErr::Io(io::Error::other(err.to_string())))?;
        if let Some(token) = token {
            return Ok(ResolvedProviderCredential {
                auth_mode: Some(ProviderCredentialMode::OAuth),
                token: Some(token),
                account_id: None,
            });
        }
    }

    if capabilities.api_key
        && let Some(api_key) = provider.api_key_from_env()?
    {
        return Ok(ResolvedProviderCredential {
            auth_mode: Some(ProviderCredentialMode::ApiKey),
            token: Some(api_key),
            account_id: None,
        });
    }

    if let Some(token) = provider.experimental_bearer_token.clone() {
        return Ok(ResolvedProviderCredential {
            auth_mode: None,
            token: Some(token),
            account_id: None,
        });
    }

    Ok(ResolvedProviderCredential::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthCredentialsStoreMode;
    use crate::auth::AuthScope;
    use crate::auth::CodexAuth;
    use crate::auth::login_with_api_key;
    use crate::auth::login_with_api_key_for_scope;
    use crate::built_in_model_providers;
    use crate::model_provider_info::ModelProviderAuthStrategy;
    use crate::model_provider_info::WireApi;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;

    fn custom_provider() -> ModelProviderInfo {
        ModelProviderInfo {
            name: "FlowerRealm".to_string(),
            base_url: Some("https://flowerrealm.top/v1".to_string()),
            auth_strategy: ModelProviderAuthStrategy::None,
            oauth: None,
            api_key: None,
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: true,
            supports_websockets: false,
        }
    }

    fn config_provider() -> ModelProviderInfo {
        ModelProviderInfo {
            name: "Acme".to_string(),
            base_url: Some("https://acme.example/v1".to_string()),
            auth_strategy: ModelProviderAuthStrategy::ApiKey,
            oauth: None,
            api_key: Some("secret-inline".to_string()),
            env_key: None,
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            websocket_connect_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: false,
        }
    }

    fn write_provider_config(codex_home: &Path, provider: ModelProviderInfo) {
        ConfigEditsBuilder::new(codex_home)
            .with_edits(vec![ConfigEdit::SetModelProvider {
                id: "acme".to_string(),
                provider: Box::new(provider),
            }])
            .apply_blocking()
            .expect("write provider config");
    }

    #[test]
    fn detect_provider_credential_mode_uses_openai_api_key_auth() {
        let codex_home = tempdir().expect("temp dir");
        let provider = custom_provider();
        let mode = detect_provider_credential_mode(
            codex_home.path(),
            "FlowerRealm",
            &provider,
            Some(&CodexAuth::create_dummy_chatgpt_auth_for_testing()),
            OAuthCredentialsStoreMode::default(),
        )
        .expect("credential mode");

        assert_eq!(mode, Some(ProviderCredentialMode::Chatgpt));
    }

    #[test]
    fn detect_provider_credential_mode_prefers_custom_provider_stored_api_key_over_openai_auth() {
        let codex_home = tempdir().expect("temp dir");
        let provider = custom_provider();
        login_with_api_key_for_scope(
            codex_home.path(),
            &AuthScope::provider("FlowerRealm"),
            "secret-inline",
            AuthCredentialsStoreMode::File,
        )
        .expect("write auth.json");

        let mode = detect_provider_credential_mode(
            codex_home.path(),
            "FlowerRealm",
            &provider,
            Some(&CodexAuth::create_dummy_chatgpt_auth_for_testing()),
            OAuthCredentialsStoreMode::default(),
        )
        .expect("credential mode");

        assert_eq!(mode, Some(ProviderCredentialMode::ApiKey));
    }

    #[tokio::test]
    async fn resolve_provider_credential_uses_openai_auth_token_for_custom_provider() {
        let codex_home = tempdir().expect("temp dir");
        let provider = custom_provider();

        let resolved = resolve_provider_credential(
            codex_home.path(),
            "FlowerRealm",
            &provider,
            Some(CodexAuth::from_api_key("sk-auth-json")),
            OAuthCredentialsStoreMode::default(),
        )
        .await
        .expect("resolve credential");

        assert_eq!(resolved.auth_mode, Some(ProviderCredentialMode::ApiKey));
        assert_eq!(resolved.token.as_deref(), Some("sk-auth-json"));
    }

    #[tokio::test]
    async fn resolve_provider_credential_prefers_custom_provider_stored_api_key_over_openai_auth() {
        let codex_home = tempdir().expect("temp dir");
        let provider = custom_provider();
        login_with_api_key_for_scope(
            codex_home.path(),
            &AuthScope::provider("FlowerRealm"),
            "secret-inline",
            AuthCredentialsStoreMode::File,
        )
        .expect("write auth.json");

        let resolved = resolve_provider_credential(
            codex_home.path(),
            "FlowerRealm",
            &provider,
            Some(CodexAuth::create_dummy_chatgpt_auth_for_testing()),
            OAuthCredentialsStoreMode::default(),
        )
        .await
        .expect("resolve credential");

        assert_eq!(resolved.auth_mode, Some(ProviderCredentialMode::ApiKey));
        assert_eq!(resolved.token.as_deref(), Some("secret-inline"));
    }

    #[tokio::test]
    async fn resolve_provider_credential_uses_default_auth_json_api_key_for_unconfigured_provider()
    {
        let codex_home = tempdir().expect("temp dir");
        let provider = config_provider();
        write_provider_config(codex_home.path(), provider.clone());
        login_with_api_key(
            codex_home.path(),
            "sk-default",
            AuthCredentialsStoreMode::File,
        )
        .expect("write default auth");

        let resolved = resolve_provider_credential(
            codex_home.path(),
            "acme",
            &provider,
            None,
            OAuthCredentialsStoreMode::default(),
        )
        .await
        .expect("resolve credential");

        assert_eq!(resolved.auth_mode, Some(ProviderCredentialMode::ApiKey));
        assert_eq!(resolved.token.as_deref(), Some("sk-default"));
    }

    #[test]
    fn read_provider_api_key_migrates_inline_value_from_config() {
        let codex_home = tempdir().expect("temp dir");
        write_provider_config(codex_home.path(), config_provider());

        let api_key = read_provider_api_key(codex_home.path(), "acme").expect("read api key");
        assert_eq!(api_key, Some("secret-inline".to_string()));

        let config = read_config_toml(codex_home.path()).expect("read config");
        let provider = config.model_providers.get("acme").expect("provider");
        assert_eq!(provider.api_key, None);
    }

    #[test]
    fn store_provider_api_key_persists_to_auth_json() {
        let codex_home = tempdir().expect("temp dir");
        let mut provider = config_provider();
        provider.api_key = None;
        write_provider_config(codex_home.path(), provider);

        store_provider_api_key(codex_home.path(), "acme", "new-secret").expect("store api key");

        let config = read_config_toml(codex_home.path()).expect("read config");
        let provider = config.model_providers.get("acme").expect("provider");
        assert_eq!(provider.api_key, None);
        assert_eq!(
            read_provider_api_key(codex_home.path(), "acme").expect("read stored api key"),
            Some("new-secret".to_string())
        );
    }

    #[test]
    fn clear_provider_api_key_removes_provider_auth_json_entry() {
        let codex_home = tempdir().expect("temp dir");
        let mut provider = config_provider();
        provider.api_key = None;
        write_provider_config(codex_home.path(), provider);
        store_provider_api_key(codex_home.path(), "acme", "new-secret").expect("store api key");

        let cleared = clear_provider_api_key(codex_home.path(), "acme").expect("clear api key");
        assert_eq!(cleared, true);

        let config = read_config_toml(codex_home.path()).expect("read config");
        let provider = config.model_providers.get("acme").expect("provider");
        assert_eq!(provider.api_key, None);
        assert_eq!(
            read_provider_api_key(codex_home.path(), "acme").expect("read api key"),
            None
        );
    }

    #[tokio::test]
    async fn resolve_provider_credential_prefers_auth_json_api_key_over_stale_in_memory_provider() {
        let codex_home = tempdir().expect("temp dir");
        let mut stored_provider = config_provider();
        stored_provider.api_key = None;
        write_provider_config(codex_home.path(), stored_provider);
        store_provider_api_key(codex_home.path(), "acme", "new-secret").expect("store api key");

        let mut stale_provider = config_provider();
        stale_provider.api_key = Some("old-secret".to_string());

        let resolved = resolve_provider_credential(
            codex_home.path(),
            "acme",
            &stale_provider,
            None,
            OAuthCredentialsStoreMode::default(),
        )
        .await
        .expect("resolve credential");

        assert_eq!(resolved.auth_mode, Some(ProviderCredentialMode::ApiKey));
        assert_eq!(resolved.token.as_deref(), Some("new-secret"));
    }

    #[tokio::test]
    async fn resolve_provider_credential_keeps_builtin_openai_provider_on_auth_storage() {
        let codex_home = tempdir().expect("temp dir");
        let mut provider =
            built_in_model_providers(/* openai_base_url */ None)[OPENAI_PROVIDER_ID].clone();
        provider.api_key = Some("config-key".to_string());

        let resolved = resolve_provider_credential(
            codex_home.path(),
            OPENAI_PROVIDER_ID,
            &provider,
            Some(CodexAuth::from_api_key("sk-auth-json")),
            OAuthCredentialsStoreMode::default(),
        )
        .await
        .expect("resolve credential");

        assert_eq!(resolved.auth_mode, Some(ProviderCredentialMode::ApiKey));
        assert_eq!(resolved.token.as_deref(), Some("sk-auth-json"));
    }

    #[test]
    fn detect_provider_credential_mode_does_not_use_stale_in_memory_api_key_when_disk_key_is_cleared()
     {
        let codex_home = tempdir().expect("temp dir");
        let mut stored_provider = config_provider();
        stored_provider.api_key = None;
        write_provider_config(codex_home.path(), stored_provider);

        let mut stale_provider = config_provider();
        stale_provider.api_key = Some("old-secret".to_string());

        let mode = detect_provider_credential_mode(
            codex_home.path(),
            "acme",
            &stale_provider,
            None,
            OAuthCredentialsStoreMode::default(),
        )
        .expect("credential mode");

        assert_eq!(mode, None);
    }

    #[test]
    fn clear_provider_credentials_removes_default_api_key_when_provider_inherits_it() {
        let codex_home = tempdir().expect("temp dir");
        let mut provider = config_provider();
        provider.api_key = None;
        write_provider_config(codex_home.path(), provider.clone());
        login_with_api_key(
            codex_home.path(),
            "sk-default",
            AuthCredentialsStoreMode::File,
        )
        .expect("write default auth");

        let cleared = clear_provider_credentials(
            codex_home.path(),
            "acme",
            &provider,
            OAuthCredentialsStoreMode::default(),
        )
        .expect("clear credentials");

        assert_eq!(cleared, true);
        let resolved =
            resolve_provider_api_key(codex_home.path(), "acme").expect("resolve api key");
        assert_eq!(resolved, None);
    }
}
