use super::*;
use codex_core::ModelProviderAuthStrategy;
use codex_core::config::ConfigBuilder;
use codex_core::config::ConfigOverrides;
use codex_protocol::config_types::TrustLevel;
use pretty_assertions::assert_eq;
use serde_json::json;
use std::collections::HashMap;
use std::time::Duration;

#[test]
fn request_placeholders_replace_url_headers_and_body() {
    let plan = ScriptRequestPlan {
        method: Some("POST".to_string()),
        url: "{{baseUrl}}/usage".to_string(),
        headers: Some(HashMap::from([
            ("Authorization".to_string(), "Bearer {{apiKey}}".to_string()),
            ("X-Provider".to_string(), "{{providerId}}".to_string()),
        ])),
        body_text: Some(
            "user={{providerName}}&acct={{accountId}}&token={{accessToken}}".to_string(),
        ),
        body_json: Some(json!({
            "token": "{{bearerToken}}",
            "provider": "{{providerId}}",
            "user": "{{userId}}",
        })),
    };
    let placeholders = vec![
        ("{{baseUrl}}", "https://example.test".to_string()),
        ("{{apiKey}}", "secret-key".to_string()),
        ("{{providerId}}", "su8".to_string()),
        ("{{providerName}}", "SU8".to_string()),
        ("{{bearerToken}}", "chatgpt-token".to_string()),
        ("{{accessToken}}", "chatgpt-token".to_string()),
        ("{{accountId}}", "acct-123".to_string()),
        ("{{userId}}", "user-123".to_string()),
    ];

    let plan = apply_request_placeholders(plan, &placeholders).expect("placeholders should apply");

    assert_eq!(plan.url, "https://example.test/usage");
    assert_eq!(
        plan.headers,
        Some(HashMap::from([
            ("Authorization".to_string(), "Bearer secret-key".to_string()),
            ("X-Provider".to_string(), "su8".to_string()),
        ]))
    );
    assert_eq!(
        plan.body_text,
        Some("user=SU8&acct=acct-123&token=chatgpt-token".to_string())
    );
    assert_eq!(
        plan.body_json,
        Some(json!({
            "token": "chatgpt-token",
            "provider": "su8",
            "user": "user-123",
        }))
    );
}

#[test]
fn request_placeholders_reject_missing_user_id_placeholder_value() {
    let plan = ScriptRequestPlan {
        method: None,
        url: "https://example.test/users/{{userId}}/usage".to_string(),
        headers: None,
        body_text: None,
        body_json: None,
    };
    let placeholders = vec![("{{accountId}}", "acct-123".to_string())];

    assert_eq!(
        apply_request_placeholders(plan, &placeholders),
        Err(
            "request uses `{{userId}}`, but the current auth state does not provide a ChatGPT user id"
                .to_string()
        )
    );
}

#[test]
fn script_request_headers_include_provider_defaults_even_without_script_headers() {
    let provider = ModelProviderInfo {
        name: "OpenAI".to_string(),
        base_url: Some("https://example.test".to_string()),
        auth_strategy: ModelProviderAuthStrategy::None,
        oauth: None,
        api_key: None,
        env_key: None,
        env_key_instructions: None,
        experimental_bearer_token: None,
        wire_api: codex_core::WireApi::Responses,
        query_params: None,
        http_headers: Some(HashMap::from([(
            "Authorization".to_string(),
            "Bearer provider-token".to_string(),
        )])),
        env_http_headers: Some(HashMap::from([(
            "X-API-Key".to_string(),
            "PROVIDER_API_KEY".to_string(),
        )])),
        request_max_retries: None,
        stream_max_retries: None,
        stream_idle_timeout_ms: None,
        requires_openai_auth: false,
        supports_websockets: false,
    };

    let headers = build_script_request_headers(&provider, None, &|name| match name {
        "PROVIDER_API_KEY" => Ok("env-token".to_string()),
        other => Err(std::env::VarError::NotPresent).map_err(|_| {
            panic!("unexpected env lookup: {other}");
        }),
    })
    .expect("provider headers should build");

    assert_eq!(
        headers
            .get("Authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer provider-token")
    );
    assert_eq!(
        headers
            .get("X-API-Key")
            .and_then(|value| value.to_str().ok()),
        Some("env-token")
    );
}

#[test]
fn script_request_headers_allow_script_values_to_override_provider_defaults() {
    let provider = ModelProviderInfo {
        name: "OpenAI".to_string(),
        base_url: Some("https://example.test".to_string()),
        auth_strategy: ModelProviderAuthStrategy::None,
        oauth: None,
        api_key: None,
        env_key: None,
        env_key_instructions: None,
        experimental_bearer_token: None,
        wire_api: codex_core::WireApi::Responses,
        query_params: None,
        http_headers: Some(HashMap::from([
            (
                "Authorization".to_string(),
                "Bearer provider-token".to_string(),
            ),
            ("X-Provider".to_string(), "provider-default".to_string()),
        ])),
        env_http_headers: None,
        request_max_retries: None,
        stream_max_retries: None,
        stream_idle_timeout_ms: None,
        requires_openai_auth: false,
        supports_websockets: false,
    };
    let request_headers = HashMap::from([
        (
            "Authorization".to_string(),
            "Bearer script-token".to_string(),
        ),
        ("X-Script".to_string(), "script-only".to_string()),
    ]);

    let headers = build_script_request_headers(&provider, Some(&request_headers), &|_| {
        Err(std::env::VarError::NotPresent)
    })
    .expect("script headers should build");

    assert_eq!(
        headers
            .get("Authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer script-token")
    );
    assert_eq!(
        headers
            .get("X-Provider")
            .and_then(|value| value.to_str().ok()),
        Some("provider-default")
    );
    assert_eq!(
        headers
            .get("X-Script")
            .and_then(|value| value.to_str().ok()),
        Some("script-only")
    );
}

#[test]
fn duplicate_provider_base_path_reports_a_fix() {
    assert_eq!(
        duplicate_provider_base_path_message(
            Some("https://www.su8.codes/codex/v1"),
            "https://www.su8.codes/codex/v1/codex/v1/usage"
        ),
        Some(
            "request.url duplicates provider base path; provider `base_url` already ends with `/codex/v1`. Use `{{baseUrl}}/usage` instead."
                .to_string()
        )
    );
}

#[test]
fn duplicate_provider_base_path_check_ignores_valid_urls() {
    assert_eq!(
        duplicate_provider_base_path_message(
            Some("https://www.su8.codes/codex/v1"),
            "https://www.su8.codes/codex/v1/usage"
        ),
        None
    );
    assert_eq!(
        duplicate_provider_base_path_message(
            Some("https://www.su8.codes/codex/v1"),
            "https://mirror.su8.codes/codex/v1/codex/v1/usage"
        ),
        None
    );
}

#[test]
fn scripted_rows_fall_back_to_aggregated_amounts() {
    let output = json!([
        {
            "planName": "月付套餐",
            "remaining": 10.0,
            "used": 1.25,
            "unit": "USD",
            "isValid": true
        },
        {
            "planName": "余额",
            "remaining": 2.0,
            "unit": "USD",
            "isValid": true
        }
    ]);

    let snapshot = normalize_script_output(output).expect("snapshot should exist");
    let ProviderUsageRefreshResult::Updated(snapshot) = snapshot else {
        panic!("expected updated snapshot");
    };

    assert_eq!(
        snapshot.remote_usage_summary(),
        Some("rem 12.00 USD | used 1.25 USD".to_string())
    );
}

#[test]
fn invalid_rows_are_filtered_and_preserve_valid_rows() {
    let output = json!([
        {
            "planName": "总览",
            "remaining": 9.5,
            "used": 1.25,
            "unit": "USD",
            "isValid": true,
            "extra": "套餐并发: 1 / 3"
        },
        {
            "planName": "不可用套餐",
            "remaining": 0.0,
            "unit": "USD",
            "isValid": false
        }
    ]);

    assert_eq!(
        normalize_script_output(output),
        Some(ProviderUsageRefreshResult::Updated(ProviderUsageSnapshot {
            plans: vec![ProviderUsagePlan {
                plan_name: Some("总览".to_string()),
                remaining: Some(9.5),
                used: Some(1.25),
                total: None,
                unit: Some("USD".to_string()),
                extra: Some("套餐并发: 1 / 3".to_string()),
            }],
            error_message: None,
        }))
    );
}

#[test]
fn invalid_script_object_becomes_failed_refresh() {
    let output = json!({
        "isValid": false,
        "invalidCode": "NO_QUOTA",
        "invalidMessage": "No available quota"
    });

    assert_eq!(
        normalize_script_output(output),
        Some(ProviderUsageRefreshResult::Failed(
            "No available quota (NO_QUOTA)".to_string()
        ))
    );
}

#[test]
fn null_script_output_becomes_skipped_refresh() {
    assert_eq!(
        normalize_script_output(JsonValue::Null),
        Some(ProviderUsageRefreshResult::Skipped)
    );
}

#[test]
fn invalid_rows_payload_becomes_failed_refresh() {
    let output = json!({
        "remaining": 12.0
    });

    let result = normalize_script_output(output).expect("result should exist");
    let ProviderUsageRefreshResult::Failed(message) = result else {
        panic!("expected failed refresh");
    };
    assert!(message.contains("extractor returned an invalid payload"));
}

#[tokio::test]
async fn provider_usage_enabled_prefers_current_trusted_cwd_over_stale_project_layer() {
    let codex_home = tempfile::tempdir().expect("temp dir");
    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("config");
    let project_root = tempfile::tempdir().expect("project root");
    let providers_dir = project_root.path().join(".codex/providers/openai");
    std::fs::create_dir_all(&providers_dir).expect("create providers dir");
    std::fs::write(
        providers_dir.join("usage.js"),
        "({ request: { url: 'https://example.test' }, extractor: () => null })",
    )
    .expect("write usage script");

    config.cwd = project_root.path().to_path_buf();
    config.active_project.trust_level = Some(TrustLevel::Trusted);

    assert!(provider_usage_enabled(&config));
    assert_eq!(
        provider_usage_poll_interval(&config),
        Some(Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS))
    );
}

#[tokio::test]
async fn trusted_project_root_prefers_trusted_subproject_over_git_root_fallback() {
    let codex_home = tempfile::tempdir().expect("temp dir");
    let repo_root = tempfile::tempdir().expect("repo root");
    std::fs::create_dir(repo_root.path().join(".git")).expect("create .git dir");

    let trusted_subproject = repo_root.path().join("apps/child");
    std::fs::create_dir_all(trusted_subproject.join(".codex/providers/openai"))
        .expect("create trusted project providers dir");
    std::fs::create_dir_all(trusted_subproject.join(".realmx"))
        .expect("create trusted project config dir");
    std::fs::write(trusted_subproject.join(".realmx/config.toml"), "")
        .expect("write trusted project config");

    let mut config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(ConfigOverrides {
            cwd: Some(trusted_subproject.clone()),
            ..Default::default()
        })
        .build()
        .await
        .expect("config");

    config.active_project.trust_level = Some(TrustLevel::Trusted);

    assert_eq!(trusted_project_root(&config), Some(trusted_subproject));
}
