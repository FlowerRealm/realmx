use super::*;
use codex_core::config::ConfigBuilder;
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
        ("{{userId}}", "acct-123".to_string()),
    ];

    let plan = apply_request_placeholders(plan, &placeholders);

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
            "user": "acct-123",
        }))
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
