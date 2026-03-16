use crate::provider_usage::ProviderUsagePlan;
use crate::provider_usage::ProviderUsageRefreshResult;
use crate::provider_usage::ProviderUsageSnapshot;
use codex_core::CodexAuth;
use codex_core::ModelProviderInfo;
use codex_core::default_client::build_reqwest_client;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderName;
use reqwest::header::HeaderValue;
use serde::Deserialize;

const SU8_PROVIDER_ID: &str = "su8";

pub(crate) fn is_legacy_su8_provider(provider_id: &str) -> bool {
    provider_id.eq_ignore_ascii_case(SU8_PROVIDER_ID)
}

#[derive(Debug, Deserialize)]
struct Su8UsageResponse {
    remaining: f64,
    #[serde(rename = "todayLimit")]
    today_limit: Option<f64>,
    #[serde(rename = "todayRemaining")]
    today_remaining: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Su8UsageRequestConfig {
    url: String,
    headers: HeaderMap,
    bearer_token: Option<String>,
    account_id: Option<String>,
}

fn su8_usage_url(provider: &ModelProviderInfo) -> Option<String> {
    let base_url = provider.base_url.clone()?;
    let mut url = url::Url::parse(&base_url).ok()?;
    {
        let mut segments = url.path_segments_mut().ok()?;
        segments.pop_if_empty();
        segments.push("usage");
    }
    if let Some(query_params) = &provider.query_params {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query_params {
            pairs.append_pair(key, value);
        }
    }
    Some(url.to_string())
}

fn su8_usage_request_config(
    provider: &ModelProviderInfo,
    auth: Option<&CodexAuth>,
) -> Option<Su8UsageRequestConfig> {
    su8_usage_request_config_with_env(provider, auth, |name| std::env::var(name).ok())
}

fn su8_usage_request_config_with_env<F>(
    provider: &ModelProviderInfo,
    auth: Option<&CodexAuth>,
    env_lookup: F,
) -> Option<Su8UsageRequestConfig>
where
    F: Fn(&str) -> Option<String>,
{
    let url = su8_usage_url(provider)?;
    let capacity = provider
        .http_headers
        .as_ref()
        .map_or(0, std::collections::HashMap::len)
        + provider
            .env_http_headers
            .as_ref()
            .map_or(0, std::collections::HashMap::len);
    let mut headers = HeaderMap::with_capacity(capacity);
    if let Some(static_headers) = &provider.http_headers {
        for (name, value) in static_headers {
            if let (Ok(name), Ok(value)) =
                (HeaderName::try_from(name), HeaderValue::try_from(value))
            {
                headers.insert(name, value);
            }
        }
    }
    if let Some(env_headers) = &provider.env_http_headers {
        for (name, env_var) in env_headers {
            if let Some(value) = env_lookup(env_var)
                && !value.trim().is_empty()
                && let (Ok(name), Ok(value)) =
                    (HeaderName::try_from(name), HeaderValue::try_from(value))
            {
                headers.insert(name, value);
            }
        }
    }

    let provider_api_key = match &provider.env_key {
        Some(env_key) => Some(env_lookup(env_key).filter(|value| !value.trim().is_empty())?),
        None => None,
    };
    let (bearer_token, account_id) = if let Some(api_key) = provider_api_key {
        (Some(api_key), None)
    } else if let Some(token) = provider.experimental_bearer_token.clone() {
        (Some(token), None)
    } else if let Some(auth) = auth {
        let token = auth.get_token().ok()?;
        (Some(token), auth.get_account_id())
    } else {
        (None, None)
    };

    Some(Su8UsageRequestConfig {
        url,
        headers,
        bearer_token,
        account_id,
    })
}

pub(crate) async fn fetch_legacy_su8_provider_usage_snapshot(
    provider: ModelProviderInfo,
    auth: Option<CodexAuth>,
) -> Option<ProviderUsageRefreshResult> {
    let request_config = su8_usage_request_config(&provider, auth.as_ref())?;
    let client = build_reqwest_client();
    let mut request = client.get(request_config.url);

    if let Some(bearer_token) = request_config.bearer_token {
        request = request.bearer_auth(bearer_token);
    }
    if let Some(account_id) = request_config.account_id {
        request = request.header("ChatGPT-Account-ID", account_id);
    }
    request = request.headers(request_config.headers);

    let response = request.send().await.ok()?;
    if response.status() == StatusCode::UNAUTHORIZED || response.status() == StatusCode::FORBIDDEN {
        return None;
    }
    if !response.status().is_success() {
        return None;
    }

    let payload = response.json::<Su8UsageResponse>().await.ok()?;
    let today_used = match (payload.today_limit, payload.today_remaining) {
        (Some(today_limit), Some(today_remaining)) => {
            Some((today_limit - today_remaining).max(0.0))
        }
        _ => None,
    };

    Some(ProviderUsageRefreshResult::Updated(ProviderUsageSnapshot {
        plans: vec![ProviderUsagePlan {
            plan_name: None,
            remaining: Some(payload.remaining),
            used: today_used,
            total: None,
            unit: Some("USD".to_string()),
            extra: None,
        }],
        error_message: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_core::CodexAuth;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;

    fn provider(
        base_url: &str,
        env_key: Option<&str>,
        query_params: Option<HashMap<String, String>>,
        http_headers: Option<HashMap<String, String>>,
        env_http_headers: Option<HashMap<String, String>>,
    ) -> ModelProviderInfo {
        ModelProviderInfo {
            name: "SU8".to_string(),
            base_url: Some(base_url.to_string()),
            api_key: None,
            env_key: env_key.map(ToString::to_string),
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: Default::default(),
            query_params,
            http_headers,
            env_http_headers,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: false,
        }
    }

    #[test]
    fn su8_usage_url_normalizes_trailing_slash() {
        let provider = provider(
            "https://example.com/codex/v1/",
            Some("SU8_API_KEY"),
            None,
            None,
            None,
        );

        assert_eq!(
            su8_usage_url(&provider),
            Some("https://example.com/codex/v1/usage".to_string())
        );
    }

    #[test]
    fn su8_usage_url_preserves_query_params() {
        let provider = provider(
            "https://example.com/codex/v1",
            Some("SU8_API_KEY"),
            Some(HashMap::from([(
                "project".to_string(),
                "alpha".to_string(),
            )])),
            None,
            None,
        );

        assert_eq!(
            su8_usage_url(&provider),
            Some("https://example.com/codex/v1/usage?project=alpha".to_string())
        );
    }

    #[test]
    fn su8_request_config_env_headers_override_static_headers() {
        let provider = provider(
            "https://example.com/codex/v1",
            Some("SU8_API_KEY"),
            None,
            Some(HashMap::from([
                ("X-Static".to_string(), "static".to_string()),
                ("Authorization".to_string(), "Bearer old".to_string()),
            ])),
            Some(HashMap::from([(
                "Authorization".to_string(),
                "SU8_AUTH_HEADER".to_string(),
            )])),
        );

        let config = su8_usage_request_config_with_env(&provider, None, |name| match name {
            "SU8_API_KEY" => Some("api-key".to_string()),
            "SU8_AUTH_HEADER" => Some("Bearer new".to_string()),
            _ => None,
        })
        .expect("config");

        assert_eq!(config.bearer_token, Some("api-key".to_string()));
        assert_eq!(
            config
                .headers
                .get("authorization")
                .and_then(|value| value.to_str().ok()),
            Some("Bearer new")
        );
        assert_eq!(
            config
                .headers
                .get("x-static")
                .and_then(|value| value.to_str().ok()),
            Some("static")
        );
    }

    #[test]
    fn su8_request_config_drops_chatgpt_fallback_when_env_key_missing() {
        let provider = provider(
            "https://example.com/codex/v1",
            Some("SU8_API_KEY"),
            None,
            None,
            None,
        );
        let auth = CodexAuth::from_api_key("fallback");

        assert_eq!(su8_usage_request_config(&provider, Some(&auth)), None);
    }

    #[test]
    fn detects_legacy_su8_provider_case_insensitively() {
        assert!(is_legacy_su8_provider("su8"));
        assert!(is_legacy_su8_provider("SU8"));
        assert!(!is_legacy_su8_provider("openai"));
    }
}
