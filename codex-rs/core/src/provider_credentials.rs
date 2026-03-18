use crate::auth::CodexAuth;
use crate::error::CodexErr;
use crate::error::Result;
use crate::model_provider_info::ModelProviderAuthStrategy;
use crate::model_provider_info::ModelProviderInfo;
use crate::provider_login_capabilities::provider_login_capabilities;
use crate::provider_login_capabilities::provider_oauth_url;
use codex_rmcp_client::OAuthCredentialsStoreMode;
use codex_rmcp_client::delete_oauth_tokens;
use codex_rmcp_client::has_oauth_tokens;
use codex_rmcp_client::load_oauth_access_token;
use codex_secrets::SecretName;
use codex_secrets::SecretScope;
use codex_secrets::SecretsBackendKind;
use codex_secrets::SecretsManager;
use sha2::Digest;
use sha2::Sha256;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderCredentialMode {
    ApiKey,
    Chatgpt,
    OAuth,
}

#[derive(Debug, Clone, Default)]
pub struct ResolvedProviderCredential {
    pub auth_mode: Option<ProviderCredentialMode>,
    pub token: Option<String>,
    pub account_id: Option<String>,
}

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

fn provider_secret_name(provider_id: &str) -> Result<SecretName> {
    let sanitized: String = provider_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    let digest = Sha256::digest(provider_id.as_bytes());
    let digest = format!("{digest:x}").to_ascii_uppercase();
    SecretName::new(&format!(
        "MODEL_PROVIDER_{sanitized}_{}_API_KEY",
        &digest[..12]
    ))
    .map_err(secret_error)
}

fn provider_oauth_server_name(provider_id: &str) -> String {
    format!("model-provider:{provider_id}")
}

fn secret_error(err: anyhow::Error) -> CodexErr {
    CodexErr::Io(io::Error::other(err.to_string()))
}

fn secrets_manager(codex_home: &Path) -> SecretsManager {
    SecretsManager::new(codex_home.to_path_buf(), SecretsBackendKind::Local)
}

pub fn read_provider_api_key(codex_home: &Path, provider_id: &str) -> Result<Option<String>> {
    let secret_name = provider_secret_name(provider_id)?;
    secrets_manager(codex_home)
        .get(&SecretScope::Global, &secret_name)
        .map_err(secret_error)
}

pub fn store_provider_api_key(codex_home: &Path, provider_id: &str, api_key: &str) -> Result<()> {
    let secret_name = provider_secret_name(provider_id)?;
    secrets_manager(codex_home)
        .set(&SecretScope::Global, &secret_name, api_key.trim())
        .map_err(secret_error)
}

pub fn clear_provider_api_key(codex_home: &Path, provider_id: &str) -> Result<bool> {
    let secret_name = provider_secret_name(provider_id)?;
    secrets_manager(codex_home)
        .delete(&SecretScope::Global, &secret_name)
        .map_err(secret_error)
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
    let capabilities = provider_login_capabilities(provider_id, provider);

    if provider.resolved_auth_strategy() == ModelProviderAuthStrategy::OpenAi
        && capabilities.uses_openai_auth()
    {
        return Ok(auth.map(credential_mode_from_openai_auth));
    }

    if read_provider_api_key(codex_home, provider_id)?.is_some() {
        return Ok(Some(ProviderCredentialMode::ApiKey));
    }

    if capabilities.oauth && has_provider_oauth_tokens(provider_id, provider, oauth_store_mode)? {
        return Ok(Some(ProviderCredentialMode::OAuth));
    }

    if capabilities.api_key
        && (provider.api_key_from_env()?.is_some() || provider.inline_api_key().is_some())
    {
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
    let removed_api_key = clear_provider_api_key(codex_home, provider_id)?;
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
    let capabilities = provider_login_capabilities(provider_id, provider);

    if provider.resolved_auth_strategy() == ModelProviderAuthStrategy::OpenAi
        && capabilities.uses_openai_auth()
        && let Some(auth) = auth
    {
        return Ok(ResolvedProviderCredential {
            auth_mode: Some(credential_mode_from_openai_auth(&auth)),
            token: Some(auth.get_token()?),
            account_id: auth.get_account_id(),
        });
    }

    if let Some(api_key) = read_provider_api_key(codex_home, provider_id)? {
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

    if capabilities.api_key
        && let Some(api_key) = provider.inline_api_key()
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
