use std::collections::HashMap;

use codex_core::ModelProviderAuthStrategy;
use codex_core::ModelProviderInfo;
use codex_core::ModelProviderOAuthConfig;
use codex_core::WireApi;

use crate::provider_flow::ProviderDraft;
use crate::provider_flow::ProviderField;
use crate::provider_flow_view::provider_clear_sentinel;
use crate::settings::data::parse_toml_fragment;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProviderCreateSubmission {
    pub(crate) id: String,
    pub(crate) provider: ModelProviderInfo,
    pub(crate) api_key_input: ProviderSecretInput,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ProviderEditSubmission {
    pub(crate) id: String,
    pub(crate) provider: ModelProviderInfo,
    pub(crate) api_key_input: Option<ProviderSecretInput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProviderSecretInput {
    KeepExisting,
    Set(String),
    Clear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ProviderFieldValue {
    Visible(String),
    Hidden {
        placeholder: String,
        current_status: String,
    },
}

impl ProviderFieldValue {
    pub(crate) fn initial_text(&self) -> String {
        match self {
            Self::Visible(value) => value.clone(),
            Self::Hidden { .. } => String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProviderSubmissionMode {
    Create,
    Edit,
}

pub(crate) fn provider_field_groups() -> &'static [&'static [ProviderField]] {
    &[
        &[
            ProviderField::Id,
            ProviderField::Name,
            ProviderField::BaseUrl,
            ProviderField::ApiKey,
        ],
        &[
            ProviderField::WireApi,
            ProviderField::RequiresOpenAiAuth,
            ProviderField::AuthStrategy,
            ProviderField::OAuth,
        ],
        &[
            ProviderField::EnvKey,
            ProviderField::EnvKeyInstructions,
            ProviderField::ExperimentalBearerToken,
        ],
        &[
            ProviderField::QueryParams,
            ProviderField::HttpHeaders,
            ProviderField::EnvHttpHeaders,
        ],
        &[
            ProviderField::RequestMaxRetries,
            ProviderField::StreamMaxRetries,
            ProviderField::StreamIdleTimeoutMs,
            ProviderField::SupportsWebsockets,
        ],
    ]
}

pub(crate) fn default_create_provider() -> ModelProviderInfo {
    ModelProviderInfo {
        name: String::new(),
        base_url: None,
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
        requires_openai_auth: true,
        supports_websockets: false,
    }
}

pub(crate) fn parse_create_draft(
    draft: &ProviderDraft,
) -> Result<ProviderCreateSubmission, String> {
    let id = normalized_required_text(ProviderField::Id, &draft.id)?;
    let mut provider = default_create_provider();
    apply_edit_value(
        &mut provider,
        ProviderField::Name,
        &draft.name,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::BaseUrl,
        &draft.base_url,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::WireApi,
        &draft.wire_api,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::RequiresOpenAiAuth,
        &draft.requires_openai_auth,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::AuthStrategy,
        &draft.auth_strategy,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::OAuth,
        &draft.oauth,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::EnvKey,
        &draft.env_key,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::EnvKeyInstructions,
        &draft.env_key_instructions,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::ExperimentalBearerToken,
        &draft.experimental_bearer_token,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::QueryParams,
        &draft.query_params,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::HttpHeaders,
        &draft.http_headers,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::EnvHttpHeaders,
        &draft.env_http_headers,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::RequestMaxRetries,
        &draft.request_max_retries,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::StreamMaxRetries,
        &draft.stream_max_retries,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::StreamIdleTimeoutMs,
        &draft.stream_idle_timeout_ms,
        ProviderSubmissionMode::Create,
    )?;
    apply_edit_value(
        &mut provider,
        ProviderField::SupportsWebsockets,
        &draft.supports_websockets,
        ProviderSubmissionMode::Create,
    )?;

    Ok(ProviderCreateSubmission {
        id,
        provider,
        api_key_input: parse_secret_input(&draft.api_key),
    })
}

pub(crate) fn parse_edit_submission(
    provider_id: &str,
    provider: &ModelProviderInfo,
    field: ProviderField,
    value: &str,
) -> Result<ProviderEditSubmission, String> {
    let mut provider = provider.clone();
    let mut id = provider_id.to_string();
    let mut api_key_input = None;

    match field {
        ProviderField::Id => {
            id = normalized_required_text(field, value)?;
        }
        ProviderField::ApiKey => {
            api_key_input = Some(parse_secret_input(value));
        }
        _ => apply_edit_value(&mut provider, field, value, ProviderSubmissionMode::Edit)?,
    }

    Ok(ProviderEditSubmission {
        id,
        provider,
        api_key_input,
    })
}

pub(crate) fn provider_field_value(
    provider_id: &str,
    provider: &ModelProviderInfo,
    field: ProviderField,
    has_secure_api_key: bool,
) -> ProviderFieldValue {
    match field {
        ProviderField::Id => ProviderFieldValue::Visible(provider_id.to_string()),
        ProviderField::Name => ProviderFieldValue::Visible(provider.name.clone()),
        ProviderField::BaseUrl => {
            ProviderFieldValue::Visible(provider.base_url.clone().unwrap_or_default())
        }
        ProviderField::ApiKey => ProviderFieldValue::Hidden {
            placeholder: format!(
                "Enter an API key, leave blank to keep the current value, or type {} to remove it.",
                provider_clear_sentinel()
            ),
            current_status: if has_secure_api_key {
                "A secure API key is already stored for this provider.".to_string()
            } else {
                "No secure API key is stored for this provider.".to_string()
            },
        },
        ProviderField::WireApi => ProviderFieldValue::Visible(provider.wire_api.to_string()),
        ProviderField::RequiresOpenAiAuth => {
            ProviderFieldValue::Visible(provider.requires_openai_auth.to_string())
        }
        ProviderField::AuthStrategy => {
            ProviderFieldValue::Visible(auth_strategy_to_string(provider.auth_strategy))
        }
        ProviderField::OAuth => ProviderFieldValue::Visible(provider_oauth_to_toml(provider)),
        ProviderField::EnvKey => {
            ProviderFieldValue::Visible(provider.env_key.clone().unwrap_or_default())
        }
        ProviderField::EnvKeyInstructions => {
            ProviderFieldValue::Visible(provider.env_key_instructions.clone().unwrap_or_default())
        }
        ProviderField::ExperimentalBearerToken => ProviderFieldValue::Hidden {
            placeholder: format!(
                "Enter a bearer token or type {} to clear it.",
                provider_clear_sentinel()
            ),
            current_status: provider
                .experimental_bearer_token
                .as_ref()
                .map(|_| "A bearer token is currently saved in config.toml.".to_string())
                .unwrap_or_else(|| "No bearer token is currently saved.".to_string()),
        },
        ProviderField::QueryParams => {
            ProviderFieldValue::Visible(map_to_toml(provider.query_params.as_ref()))
        }
        ProviderField::HttpHeaders => {
            ProviderFieldValue::Visible(map_to_toml(provider.http_headers.as_ref()))
        }
        ProviderField::EnvHttpHeaders => {
            ProviderFieldValue::Visible(map_to_toml(provider.env_http_headers.as_ref()))
        }
        ProviderField::RequestMaxRetries => ProviderFieldValue::Visible(
            provider
                .request_max_retries
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        ProviderField::StreamMaxRetries => ProviderFieldValue::Visible(
            provider
                .stream_max_retries
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        ProviderField::StreamIdleTimeoutMs => ProviderFieldValue::Visible(
            provider
                .stream_idle_timeout_ms
                .map(|value| value.to_string())
                .unwrap_or_default(),
        ),
        ProviderField::SupportsWebsockets => {
            ProviderFieldValue::Visible(provider.supports_websockets.to_string())
        }
    }
}

pub(crate) fn provider_field_placeholder(field: ProviderField) -> String {
    match field {
        ProviderField::Id => "Enter a provider ID using lowercase letters, digits, '-' or '_'.".to_string(),
        ProviderField::Name => "Enter a display name.".to_string(),
        ProviderField::BaseUrl => "Enter the provider base URL.".to_string(),
        ProviderField::ApiKey => format!(
            "Enter an API key, leave blank to keep the current value, or type {} to remove it.",
            provider_clear_sentinel()
        ),
        ProviderField::WireApi => "Enter the wire API. Supported values: responses.".to_string(),
        ProviderField::RequiresOpenAiAuth => "Enter true or false.".to_string(),
        ProviderField::AuthStrategy => {
            "Enter one of: none, openai, api_key, oauth, oauth_or_api_key.".to_string()
        }
        ProviderField::OAuth => {
            "Enter a TOML inline table, for example { url = \"https://example.com/oauth\", scopes = [\"scope\"] }.".to_string()
        }
        ProviderField::EnvKey => "Enter the environment variable name to read an API key from.".to_string(),
        ProviderField::EnvKeyInstructions => {
            "Enter help text that explains how to obtain or set the environment variable.".to_string()
        }
        ProviderField::ExperimentalBearerToken => format!(
            "Enter a bearer token or type {} to clear it.",
            provider_clear_sentinel()
        ),
        ProviderField::QueryParams => {
            "Enter a TOML inline table, for example { api-version = \"2025-04-01-preview\" }.".to_string()
        }
        ProviderField::HttpHeaders | ProviderField::EnvHttpHeaders => {
            "Enter a TOML inline table of string key/value pairs.".to_string()
        }
        ProviderField::RequestMaxRetries
        | ProviderField::StreamMaxRetries
        | ProviderField::StreamIdleTimeoutMs => "Enter a non-negative integer.".to_string(),
        ProviderField::SupportsWebsockets => "Enter true or false.".to_string(),
    }
}

pub(crate) fn is_hidden_provider_field(field: ProviderField) -> bool {
    matches!(
        field,
        ProviderField::ApiKey | ProviderField::ExperimentalBearerToken
    )
}

fn parse_secret_input(value: &str) -> ProviderSecretInput {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case(provider_clear_sentinel()) {
        ProviderSecretInput::Clear
    } else if trimmed.is_empty() {
        ProviderSecretInput::KeepExisting
    } else {
        ProviderSecretInput::Set(trimmed.to_string())
    }
}

fn normalized_required_text(field: ProviderField, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(match field {
            ProviderField::Id => "Provider ID is required.".to_string(),
            ProviderField::Name => "Display name is required.".to_string(),
            ProviderField::BaseUrl => "Base URL is required.".to_string(),
            _ => format!("{} is required.", field.label()),
        });
    }
    Ok(trimmed.to_string())
}

fn apply_edit_value(
    provider: &mut ModelProviderInfo,
    field: ProviderField,
    value: &str,
    mode: ProviderSubmissionMode,
) -> Result<(), String> {
    let trimmed = value.trim();
    let clear = trimmed.eq_ignore_ascii_case(provider_clear_sentinel());
    if clear && mode == ProviderSubmissionMode::Create {
        return Err(format!(
            "{} cannot be cleared during creation.",
            field.label()
        ));
    }
    let keep_existing_hidden_value = mode == ProviderSubmissionMode::Edit
        && is_hidden_provider_field(field)
        && trimmed.is_empty();

    match field {
        ProviderField::Id | ProviderField::ApiKey => {}
        ProviderField::Name => {
            provider.name = normalized_required_text(field, trimmed)?;
        }
        ProviderField::BaseUrl => {
            provider.base_url = Some(normalized_required_text(field, trimmed)?);
        }
        ProviderField::WireApi => {
            provider.wire_api = parse_wire_api(trimmed)?;
        }
        ProviderField::RequiresOpenAiAuth => {
            provider.requires_openai_auth = parse_bool(trimmed, field)?;
        }
        ProviderField::AuthStrategy => {
            provider.auth_strategy = if clear || trimmed.is_empty() {
                ModelProviderAuthStrategy::None
            } else {
                parse_auth_strategy(trimmed)?
            };
        }
        ProviderField::OAuth => {
            provider.oauth = parse_optional_oauth(trimmed, clear)?;
        }
        ProviderField::EnvKey => {
            provider.env_key = parse_optional_string(trimmed, clear);
        }
        ProviderField::EnvKeyInstructions => {
            provider.env_key_instructions = parse_optional_string(trimmed, clear);
        }
        ProviderField::ExperimentalBearerToken => {
            if !keep_existing_hidden_value {
                parse_optional_string(trimmed, clear)
                    .clone_into(&mut provider.experimental_bearer_token);
            }
        }
        ProviderField::QueryParams => {
            provider.query_params = parse_optional_string_map(trimmed, clear)?;
        }
        ProviderField::HttpHeaders => {
            provider.http_headers = parse_optional_string_map(trimmed, clear)?;
        }
        ProviderField::EnvHttpHeaders => {
            provider.env_http_headers = parse_optional_string_map(trimmed, clear)?;
        }
        ProviderField::RequestMaxRetries => {
            provider.request_max_retries = parse_optional_u64(trimmed, clear, field)?;
        }
        ProviderField::StreamMaxRetries => {
            provider.stream_max_retries = parse_optional_u64(trimmed, clear, field)?;
        }
        ProviderField::StreamIdleTimeoutMs => {
            provider.stream_idle_timeout_ms = parse_optional_u64(trimmed, clear, field)?;
        }
        ProviderField::SupportsWebsockets => {
            provider.supports_websockets =
                if mode == ProviderSubmissionMode::Create && trimmed.is_empty() {
                    false
                } else {
                    parse_bool(trimmed, field)?
                };
        }
    }

    Ok(())
}

fn parse_wire_api(value: &str) -> Result<WireApi, String> {
    match value {
        "" | "responses" => Ok(WireApi::Responses),
        _ => Err("Supported wire_api values: responses.".to_string()),
    }
}

fn parse_bool(value: &str, field: ProviderField) -> Result<bool, String> {
    value
        .parse::<bool>()
        .map_err(|_| format!("{} must be `true` or `false`.", field.label()))
}

fn parse_auth_strategy(value: &str) -> Result<ModelProviderAuthStrategy, String> {
    match value {
        "none" => Ok(ModelProviderAuthStrategy::None),
        "openai" => Ok(ModelProviderAuthStrategy::OpenAi),
        "api_key" => Ok(ModelProviderAuthStrategy::ApiKey),
        "oauth" => Ok(ModelProviderAuthStrategy::OAuth),
        "oauth_or_api_key" => Ok(ModelProviderAuthStrategy::OAuthOrApiKey),
        _ => Err(
            "auth_strategy must be one of: none, openai, api_key, oauth, oauth_or_api_key."
                .to_string(),
        ),
    }
}

fn parse_optional_oauth(
    value: &str,
    clear: bool,
) -> Result<Option<ModelProviderOAuthConfig>, String> {
    if clear || value.is_empty() {
        return Ok(None);
    }

    let parsed = parse_toml_fragment(value)?;
    parsed
        .try_into()
        .map_err(|err| format!("Invalid oauth config: {err}"))
}

fn parse_optional_string(value: &str, clear: bool) -> Option<String> {
    if clear || value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_optional_u64(
    value: &str,
    clear: bool,
    field: ProviderField,
) -> Result<Option<u64>, String> {
    if clear || value.is_empty() {
        return Ok(None);
    }

    value
        .parse::<u64>()
        .map(Some)
        .map_err(|_| format!("{} must be a non-negative integer.", field.label()))
}

fn parse_optional_string_map(
    value: &str,
    clear: bool,
) -> Result<Option<HashMap<String, String>>, String> {
    if clear || value.is_empty() {
        return Ok(None);
    }

    let parsed = parse_toml_fragment(value)?;
    let table = parsed
        .as_table()
        .ok_or_else(|| "Expected a TOML inline table of string pairs.".to_string())?;
    let mut map = HashMap::with_capacity(table.len());
    for (key, value) in table {
        let Some(value) = value.as_str() else {
            return Err("Expected all keys and values to be strings.".to_string());
        };
        map.insert(key.clone(), value.to_string());
    }
    Ok(Some(map))
}

fn auth_strategy_to_string(strategy: ModelProviderAuthStrategy) -> String {
    match strategy {
        ModelProviderAuthStrategy::None => "none".to_string(),
        ModelProviderAuthStrategy::OpenAi => "openai".to_string(),
        ModelProviderAuthStrategy::ApiKey => "api_key".to_string(),
        ModelProviderAuthStrategy::OAuth => "oauth".to_string(),
        ModelProviderAuthStrategy::OAuthOrApiKey => "oauth_or_api_key".to_string(),
    }
}

fn provider_oauth_to_toml(provider: &ModelProviderInfo) -> String {
    let Some(oauth) = provider.oauth.as_ref() else {
        return String::new();
    };

    let mut table = toml::map::Map::new();
    if let Some(url) = oauth.url.as_ref() {
        table.insert("url".to_string(), toml::Value::String(url.clone()));
    }
    if let Some(scopes) = oauth.scopes.as_ref() {
        table.insert(
            "scopes".to_string(),
            toml::Value::Array(scopes.iter().cloned().map(toml::Value::String).collect()),
        );
    }
    if let Some(resource) = oauth.oauth_resource.as_ref() {
        table.insert(
            "oauth_resource".to_string(),
            toml::Value::String(resource.clone()),
        );
    }
    toml::Value::Table(table).to_string()
}

fn map_to_toml(map: Option<&HashMap<String, String>>) -> String {
    let Some(map) = map else {
        return String::new();
    };

    let mut ordered = toml::map::Map::new();
    let mut entries = map.iter().collect::<Vec<_>>();
    entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    for (key, value) in entries {
        ordered.insert((*key).clone(), toml::Value::String((*value).clone()));
    }
    toml::Value::Table(ordered).to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::ProviderSecretInput;
    use super::default_create_provider;
    use super::parse_create_draft;
    use super::parse_edit_submission;
    use super::provider_field_value;
    use crate::provider_flow::ProviderDraft;
    use crate::provider_flow::ProviderField;
    use pretty_assertions::assert_eq;

    #[test]
    fn default_create_provider_matches_requested_defaults() {
        let provider = default_create_provider();
        assert_eq!(provider.wire_api.to_string(), "responses");
        assert!(provider.requires_openai_auth);
        assert_eq!(
            provider.auth_strategy,
            codex_core::ModelProviderAuthStrategy::None
        );
        assert_eq!(provider.base_url, None);
    }

    #[test]
    fn parse_create_draft_keeps_advanced_defaults_when_blank() {
        let mut draft = ProviderDraft::new();
        draft.id = "acme".to_string();
        draft.name = "Acme".to_string();
        draft.base_url = "https://acme.example/v1".to_string();

        let submission = parse_create_draft(&draft).expect("valid create draft");

        assert_eq!(submission.id, "acme");
        assert_eq!(submission.provider.name, "Acme");
        assert_eq!(
            submission.provider.base_url.as_deref(),
            Some("https://acme.example/v1")
        );
        assert_eq!(
            submission.provider.auth_strategy,
            codex_core::ModelProviderAuthStrategy::None
        );
        assert!(submission.provider.requires_openai_auth);
        assert!(!submission.provider.supports_websockets);
        assert_eq!(submission.api_key_input, ProviderSecretInput::KeepExisting);
    }

    #[test]
    fn create_draft_defaults_only_prefill_requested_fields() {
        let draft = ProviderDraft::new();

        assert_eq!(draft.wire_api, "responses");
        assert_eq!(draft.requires_openai_auth, "true");
        assert!(draft.supports_websockets.is_empty());
        assert!(draft.auth_strategy.is_empty());
        assert!(draft.experimental_bearer_token.is_empty());
    }

    #[test]
    fn parse_edit_submission_supports_id_rename_and_structured_fields() {
        let mut provider = default_create_provider();
        provider.name = "Acme".to_string();
        provider.base_url = Some("https://acme.example/v1".to_string());

        let rename =
            parse_edit_submission("acme", &provider, ProviderField::Id, "acme-2").expect("rename");
        assert_eq!(rename.id, "acme-2");
        assert_eq!(rename.provider, provider);

        let edit = parse_edit_submission(
            "acme",
            &provider,
            ProviderField::QueryParams,
            "{ api-version = \"2025-04-01-preview\" }",
        )
        .expect("query params");
        assert_eq!(
            edit.provider.query_params,
            Some(HashMap::from([(
                "api-version".to_string(),
                "2025-04-01-preview".to_string()
            )]))
        );
    }

    #[test]
    fn parse_edit_submission_keeps_hidden_values_when_left_blank() {
        let mut provider = default_create_provider();
        provider.name = "Acme".to_string();
        provider.base_url = Some("https://acme.example/v1".to_string());
        provider.experimental_bearer_token = Some("secret".to_string());

        let edit = parse_edit_submission(
            "acme",
            &provider,
            ProviderField::ExperimentalBearerToken,
            "",
        )
        .expect("blank hidden edit should keep existing value");

        assert_eq!(
            edit.provider.experimental_bearer_token.as_deref(),
            Some("secret")
        );
    }

    #[test]
    fn provider_field_value_hides_secret_initial_text() {
        let mut provider = default_create_provider();
        provider.experimental_bearer_token = Some("secret".to_string());

        let value = provider_field_value(
            "acme",
            &provider,
            ProviderField::ExperimentalBearerToken,
            false,
        );
        assert!(matches!(value, super::ProviderFieldValue::Hidden { .. }));
        assert_eq!(value.initial_text(), "");
        assert_eq!(
            value,
            super::ProviderFieldValue::Hidden {
                placeholder: format!(
                    "Enter a bearer token or type {} to clear it.",
                    crate::provider_flow_view::provider_clear_sentinel()
                ),
                current_status: "A bearer token is currently saved in config.toml.".to_string(),
            }
        );
    }
}
