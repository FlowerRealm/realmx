use std::collections::HashSet;

use codex_core::ModelProviderInfo;
use codex_core::OPENAI_PROVIDER_ID;
use codex_core::built_in_model_providers;
use codex_core::config::Config;
use codex_core::read_provider_api_key;
use codex_core::validate_model_provider_id;

use crate::provider_edit::default_create_provider;
use crate::settings::data::SettingsScope;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderFlowSource {
    SlashCommand,
    SettingsModel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderField {
    Id,
    Name,
    BaseUrl,
    ApiKey,
    WireApi,
    RequiresOpenAiAuth,
    AuthStrategy,
    OAuth,
    EnvKey,
    EnvKeyInstructions,
    ExperimentalBearerToken,
    QueryParams,
    HttpHeaders,
    EnvHttpHeaders,
    RequestMaxRetries,
    StreamMaxRetries,
    StreamIdleTimeoutMs,
    SupportsWebsockets,
}

impl ProviderField {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Id => "Provider ID",
            Self::Name => "Display name",
            Self::BaseUrl => "Base URL",
            Self::ApiKey => "API key",
            Self::WireApi => "wire_api",
            Self::RequiresOpenAiAuth => "requires_openai_auth",
            Self::AuthStrategy => "auth_strategy",
            Self::OAuth => "oauth",
            Self::EnvKey => "env_key",
            Self::EnvKeyInstructions => "env_key_instructions",
            Self::ExperimentalBearerToken => "experimental_bearer_token",
            Self::QueryParams => "query_params",
            Self::HttpHeaders => "http_headers",
            Self::EnvHttpHeaders => "env_http_headers",
            Self::RequestMaxRetries => "request_max_retries",
            Self::StreamMaxRetries => "stream_max_retries",
            Self::StreamIdleTimeoutMs => "stream_idle_timeout_ms",
            Self::SupportsWebsockets => "supports_websockets",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProviderScreen {
    Root,
    Detail { provider_id: String },
    Create,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderFlowLocation {
    pub(crate) source: ProviderFlowSource,
    pub(crate) scope: SettingsScope,
    pub(crate) screen: ProviderScreen,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProviderFlowNavigation {
    ExitFlow,
    ReturnToRoot {
        source: ProviderFlowSource,
        scope: SettingsScope,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderDraft {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) wire_api: String,
    pub(crate) requires_openai_auth: String,
    pub(crate) auth_strategy: String,
    pub(crate) oauth: String,
    pub(crate) env_key: String,
    pub(crate) env_key_instructions: String,
    pub(crate) experimental_bearer_token: String,
    pub(crate) query_params: String,
    pub(crate) http_headers: String,
    pub(crate) env_http_headers: String,
    pub(crate) request_max_retries: String,
    pub(crate) stream_max_retries: String,
    pub(crate) stream_idle_timeout_ms: String,
    pub(crate) supports_websockets: String,
}

impl ProviderDraft {
    pub(crate) fn new() -> Self {
        let defaults = default_create_provider();
        Self {
            id: String::new(),
            name: String::new(),
            base_url: String::new(),
            api_key: String::new(),
            wire_api: defaults.wire_api.to_string(),
            requires_openai_auth: defaults.requires_openai_auth.to_string(),
            auth_strategy: String::new(),
            oauth: String::new(),
            env_key: String::new(),
            env_key_instructions: String::new(),
            experimental_bearer_token: String::new(),
            query_params: String::new(),
            http_headers: String::new(),
            env_http_headers: String::new(),
            request_max_retries: String::new(),
            stream_max_retries: String::new(),
            stream_idle_timeout_ms: String::new(),
            supports_websockets: String::new(),
        }
    }

    pub(crate) fn field_value(&self, field: ProviderField) -> &str {
        match field {
            ProviderField::Id => &self.id,
            ProviderField::Name => &self.name,
            ProviderField::BaseUrl => &self.base_url,
            ProviderField::ApiKey => &self.api_key,
            ProviderField::WireApi => &self.wire_api,
            ProviderField::RequiresOpenAiAuth => &self.requires_openai_auth,
            ProviderField::AuthStrategy => &self.auth_strategy,
            ProviderField::OAuth => &self.oauth,
            ProviderField::EnvKey => &self.env_key,
            ProviderField::EnvKeyInstructions => &self.env_key_instructions,
            ProviderField::ExperimentalBearerToken => &self.experimental_bearer_token,
            ProviderField::QueryParams => &self.query_params,
            ProviderField::HttpHeaders => &self.http_headers,
            ProviderField::EnvHttpHeaders => &self.env_http_headers,
            ProviderField::RequestMaxRetries => &self.request_max_retries,
            ProviderField::StreamMaxRetries => &self.stream_max_retries,
            ProviderField::StreamIdleTimeoutMs => &self.stream_idle_timeout_ms,
            ProviderField::SupportsWebsockets => &self.supports_websockets,
        }
    }

    pub(crate) fn update_field(&mut self, field: ProviderField, value: String) {
        match field {
            ProviderField::Id => self.id = value,
            ProviderField::Name => self.name = value,
            ProviderField::BaseUrl => self.base_url = value,
            ProviderField::ApiKey => self.api_key = value,
            ProviderField::WireApi => self.wire_api = value,
            ProviderField::RequiresOpenAiAuth => self.requires_openai_auth = value,
            ProviderField::AuthStrategy => self.auth_strategy = value,
            ProviderField::OAuth => self.oauth = value,
            ProviderField::EnvKey => self.env_key = value,
            ProviderField::EnvKeyInstructions => self.env_key_instructions = value,
            ProviderField::ExperimentalBearerToken => self.experimental_bearer_token = value,
            ProviderField::QueryParams => self.query_params = value,
            ProviderField::HttpHeaders => self.http_headers = value,
            ProviderField::EnvHttpHeaders => self.env_http_headers = value,
            ProviderField::RequestMaxRetries => self.request_max_retries = value,
            ProviderField::StreamMaxRetries => self.stream_max_retries = value,
            ProviderField::StreamIdleTimeoutMs => self.stream_idle_timeout_ms = value,
            ProviderField::SupportsWebsockets => self.supports_websockets = value,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProviderFlowRow {
    pub(crate) id: String,
    pub(crate) provider: ModelProviderInfo,
    pub(crate) is_builtin: bool,
    pub(crate) is_default: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProviderFlowData {
    pub(crate) rows: Vec<ProviderFlowRow>,
    pub(crate) create_draft: ProviderDraft,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProviderDetailRuntimeState {
    pub(crate) has_secure_api_key: bool,
    pub(crate) can_edit_usage_scripts: bool,
}

impl ProviderDetailRuntimeState {
    pub(crate) fn from_config(
        config: &Config,
        provider_id: &str,
        provider: &ModelProviderInfo,
    ) -> Self {
        let has_secure_api_key = read_provider_api_key(&config.codex_home, provider_id)
            .ok()
            .flatten()
            .is_some()
            || provider.inline_api_key().is_some();
        Self {
            has_secure_api_key,
            can_edit_usage_scripts: crate::provider_usage::can_edit_provider_usage_scripts(config),
        }
    }
}

impl ProviderFlowData {
    pub(crate) fn from_config(config: &Config, scope: SettingsScope) -> Self {
        let scope = scope.normalized(config.active_profile.as_deref());
        let default_provider_id = current_provider_id_for_scope(config, scope);
        let builtin_ids: HashSet<String> = built_in_model_providers(/*openai_base_url*/ None)
            .keys()
            .cloned()
            .collect();
        let mut rows: Vec<ProviderFlowRow> = config
            .model_providers
            .iter()
            .map(|(id, provider)| ProviderFlowRow {
                id: id.clone(),
                provider: provider.clone(),
                is_builtin: builtin_ids.contains(id),
                is_default: id == &default_provider_id,
            })
            .collect();
        rows.sort_by(|left, right| left.id.cmp(&right.id));

        Self {
            rows,
            create_draft: ProviderDraft::new(),
        }
    }

    pub(crate) fn row(&self, provider_id: &str) -> Option<&ProviderFlowRow> {
        self.rows.iter().find(|row| row.id == provider_id)
    }

    pub(crate) fn create_field_value(&self, field: ProviderField) -> &str {
        self.create_draft.field_value(field)
    }
}

pub(crate) fn current_provider_id_for_scope(config: &Config, scope: SettingsScope) -> String {
    let effective_config = config.config_layer_stack.effective_config();
    let global_provider_id = value_for_path(&effective_config, Some("model_provider"))
        .and_then(toml::Value::as_str)
        .unwrap_or(OPENAI_PROVIDER_ID);

    match scope {
        SettingsScope::Global => global_provider_id.to_string(),
        SettingsScope::ActiveProfile => config
            .active_profile
            .as_deref()
            .and_then(|profile| {
                let profile_key = format!("profiles.{profile}.model_provider");
                value_for_path(&effective_config, Some(profile_key.as_str()))
                    .and_then(toml::Value::as_str)
            })
            .unwrap_or(global_provider_id)
            .to_string(),
    }
}

fn value_for_path<'a>(value: &'a toml::Value, key_path: Option<&str>) -> Option<&'a toml::Value> {
    let key_path = key_path?;
    let mut current = value;
    for segment in key_path.split('.') {
        current = current.as_table()?.get(segment)?;
    }
    Some(current)
}

pub(crate) fn validate_provider_id(
    provider_id: &str,
    rows: &[ProviderFlowRow],
    current_id: Option<&str>,
) -> Result<(), String> {
    validate_model_provider_id(provider_id)?;
    if rows
        .iter()
        .any(|row| row.is_builtin && row.id == provider_id)
        && current_id != Some(provider_id)
    {
        return Err("Provider ID collides with a built-in provider.".to_string());
    }
    if rows.iter().any(|row| row.id == provider_id) && current_id != Some(provider_id) {
        return Err("Provider ID already exists.".to_string());
    }
    Ok(())
}
