use crate::model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
use crate::model_provider_info::ModelProviderAuthStrategy;
use crate::model_provider_info::ModelProviderInfo;
use crate::model_provider_info::OLLAMA_OSS_PROVIDER_ID;
use crate::model_provider_info::OPENAI_PROVIDER_ID;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProviderLoginCapabilities {
    pub api_key: bool,
    pub chatgpt: bool,
    pub device_code: bool,
    pub oauth: bool,
}

impl ProviderLoginCapabilities {
    pub fn requires_auth(self) -> bool {
        self.api_key || self.chatgpt || self.device_code || self.oauth
    }

    pub fn uses_openai_auth(self) -> bool {
        self.chatgpt || self.device_code
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct BuiltInProviderLoginCapabilities {
    oauth: bool,
}

fn built_in_provider_login_capabilities(
    provider_id: &str,
) -> Option<BuiltInProviderLoginCapabilities> {
    match provider_id {
        OPENAI_PROVIDER_ID | OLLAMA_OSS_PROVIDER_ID | LMSTUDIO_OSS_PROVIDER_ID => {
            Some(BuiltInProviderLoginCapabilities { oauth: false })
        }
        _ => None,
    }
}

fn normalized_provider_auth_strategy(
    provider_id: &str,
    provider: &ModelProviderInfo,
) -> ModelProviderAuthStrategy {
    if built_in_provider_login_capabilities(provider_id).is_some() {
        return provider.resolved_auth_strategy();
    }

    if matches!(
        provider.resolved_auth_strategy(),
        ModelProviderAuthStrategy::OpenAi
    ) || provider.requires_openai_auth
    {
        return ModelProviderAuthStrategy::OpenAi;
    }

    if provider.inline_api_key().is_some()
        || provider.env_key.is_some()
        || matches!(
            provider.resolved_auth_strategy(),
            ModelProviderAuthStrategy::ApiKey
                | ModelProviderAuthStrategy::OAuth
                | ModelProviderAuthStrategy::OAuthOrApiKey
        )
        || provider.oauth.is_some()
    {
        return ModelProviderAuthStrategy::ApiKey;
    }

    ModelProviderAuthStrategy::None
}

pub fn provider_login_capabilities(
    provider_id: &str,
    provider: &ModelProviderInfo,
) -> ProviderLoginCapabilities {
    let auth_strategy = normalized_provider_auth_strategy(provider_id, provider);
    let built_in = built_in_provider_login_capabilities(provider_id).unwrap_or_default();
    let uses_openai_auth = auth_strategy == ModelProviderAuthStrategy::OpenAi;

    ProviderLoginCapabilities {
        api_key: if uses_openai_auth {
            true
        } else {
            matches!(
                auth_strategy,
                ModelProviderAuthStrategy::ApiKey
                    | ModelProviderAuthStrategy::OAuth
                    | ModelProviderAuthStrategy::OAuthOrApiKey
            )
        },
        chatgpt: uses_openai_auth,
        device_code: uses_openai_auth,
        oauth: built_in.oauth
            && matches!(
                auth_strategy,
                ModelProviderAuthStrategy::OAuth | ModelProviderAuthStrategy::OAuthOrApiKey
            )
            && provider.oauth.is_some(),
    }
}

pub fn provider_oauth_url<'a>(
    provider_id: &str,
    provider: &'a ModelProviderInfo,
) -> Option<&'a str> {
    if !provider_login_capabilities(provider_id, provider).oauth {
        return None;
    }

    provider
        .oauth
        .as_ref()
        .and_then(|oauth| oauth.url.as_deref())
        .or(provider.base_url.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_provider_info::ModelProviderOAuthConfig;
    use crate::model_provider_info::WireApi;
    use pretty_assertions::assert_eq;

    fn provider() -> ModelProviderInfo {
        ModelProviderInfo {
            name: "Example".to_string(),
            base_url: Some("https://example.com/v1".to_string()),
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
            requires_openai_auth: false,
            supports_websockets: false,
        }
    }

    #[test]
    fn custom_provider_does_not_expose_oauth_from_oauth_config_alone() {
        let mut provider = provider();
        provider.oauth = Some(ModelProviderOAuthConfig {
            url: Some("https://example.com/oauth".to_string()),
            scopes: None,
            oauth_resource: None,
        });

        let capabilities = provider_login_capabilities("custom-provider", &provider);

        assert_eq!(
            capabilities,
            ProviderLoginCapabilities {
                api_key: false,
                chatgpt: false,
                device_code: false,
                oauth: false,
            }
        );
        assert_eq!(provider_oauth_url("custom-provider", &provider), None);
    }

    #[test]
    fn custom_provider_normalizes_explicit_oauth_to_api_key_login() {
        let mut provider = provider();
        provider.auth_strategy = ModelProviderAuthStrategy::OAuth;
        provider.oauth = Some(ModelProviderOAuthConfig {
            url: Some("https://example.com/oauth".to_string()),
            scopes: None,
            oauth_resource: None,
        });

        let capabilities = provider_login_capabilities("custom-provider", &provider);

        assert_eq!(
            capabilities,
            ProviderLoginCapabilities {
                api_key: true,
                chatgpt: false,
                device_code: false,
                oauth: false,
            }
        );
    }

    #[test]
    fn custom_provider_preserves_openai_auth_strategy() {
        let mut provider = provider();
        provider.auth_strategy = ModelProviderAuthStrategy::OpenAi;

        let capabilities = provider_login_capabilities("custom-provider", &provider);

        assert_eq!(
            capabilities,
            ProviderLoginCapabilities {
                api_key: true,
                chatgpt: true,
                device_code: true,
                oauth: false,
            }
        );
    }

    #[test]
    fn custom_provider_preserves_requires_openai_auth() {
        let mut provider = provider();
        provider.requires_openai_auth = true;

        let capabilities = provider_login_capabilities("custom-provider", &provider);

        assert_eq!(
            capabilities,
            ProviderLoginCapabilities {
                api_key: true,
                chatgpt: true,
                device_code: true,
                oauth: false,
            }
        );
    }
}
