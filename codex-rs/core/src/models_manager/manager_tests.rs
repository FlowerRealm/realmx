use super::*;
use crate::CodexAuth;
use crate::auth::AuthCredentialsStoreMode;
use crate::config::ConfigBuilder;
use crate::model_provider_info::ModelProviderAuthStrategy;
use crate::model_provider_info::WireApi;
use crate::models_manager::manager::GPT_5_4_MODEL;
use crate::models_manager::manager::GPT_5_4_ONE_MILLION_MODEL;
use chrono::Utc;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use core_test_support::responses::mount_models_once;
use pretty_assertions::assert_eq;
use serde_json::json;
use tempfile::tempdir;
use wiremock::Mock;
use wiremock::MockServer;
use wiremock::ResponseTemplate;
use wiremock::matchers::method;
use wiremock::matchers::path;

fn remote_model(slug: &str, display: &str, priority: i32) -> ModelInfo {
    remote_model_with_visibility(slug, display, priority, "list")
}

fn remote_model_with_visibility(
    slug: &str,
    display: &str,
    priority: i32,
    visibility: &str,
) -> ModelInfo {
    serde_json::from_value(json!({
            "slug": slug,
            "display_name": display,
            "description": format!("{display} desc"),
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": [{"effort": "low", "description": "low"}, {"effort": "medium", "description": "medium"}],
            "shell_type": "shell_command",
            "visibility": visibility,
            "minimal_client_version": [0, 1, 0],
            "supported_in_api": true,
            "priority": priority,
            "upgrade": null,
            "base_instructions": "base instructions",
            "supports_reasoning_summaries": false,
            "support_verbosity": false,
            "default_verbosity": null,
            "apply_patch_tool_type": null,
            "truncation_policy": {"mode": "bytes", "limit": 10_000},
            "supports_parallel_tool_calls": false,
            "supports_image_detail_original": false,
            "context_window": 272_000,
            "experimental_supported_tools": [],
        }))
        .expect("valid model")
}

fn assert_models_contain(actual: &[ModelInfo], expected: &[ModelInfo]) {
    for model in expected {
        assert!(
            actual.iter().any(|candidate| candidate.slug == model.slug),
            "expected model {} in cached list",
            model.slug
        );
    }
}

fn provider_for(base_url: String) -> ModelProviderInfo {
    ModelProviderInfo {
        name: "mock".into(),
        base_url: Some(base_url),
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
        request_max_retries: Some(0),
        stream_max_retries: Some(0),
        stream_idle_timeout_ms: Some(5_000),
        requires_openai_auth: false,
        supports_websockets: false,
    }
}

async fn mount_openai_models_once(server: &MockServer, model_ids: &[&str]) {
    let data: Vec<_> = model_ids
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "object": "model",
                "created": 0,
                "owned_by": "openai"
            })
        })
        .collect();

    Mock::given(method("GET"))
        .and(path("/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({
                    "object": "list",
                    "data": data,
                })),
        )
        .up_to_n_times(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn get_model_info_tracks_fallback_usage() {
    let codex_home = tempdir().expect("temp dir");
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("load default test config");
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let manager = ModelsManager::new(
        codex_home.path().to_path_buf(),
        auth_manager,
        None,
        CollaborationModesConfig::default(),
    );
    let known_slug = manager
        .get_remote_models()
        .await
        .first()
        .expect("bundled models should include at least one model")
        .slug
        .clone();

    let known = manager.get_model_info(known_slug.as_str(), &config).await;
    assert!(!known.used_fallback_model_metadata);
    assert_eq!(known.slug, known_slug);

    let unknown = manager
        .get_model_info("model-that-does-not-exist", &config)
        .await;
    assert!(unknown.used_fallback_model_metadata);
    assert_eq!(unknown.slug, "model-that-does-not-exist");
}

#[tokio::test]
async fn get_model_info_uses_custom_catalog() {
    let codex_home = tempdir().expect("temp dir");
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("load default test config");
    let mut overlay = remote_model("gpt-overlay", "Overlay", 0);
    overlay.supports_image_detail_original = true;

    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let manager = ModelsManager::new(
        codex_home.path().to_path_buf(),
        auth_manager,
        Some(ModelsResponse {
            models: vec![overlay],
        }),
        CollaborationModesConfig::default(),
    );

    let model_info = manager
        .get_model_info("gpt-overlay-experiment", &config)
        .await;

    assert_eq!(model_info.slug, "gpt-overlay-experiment");
    assert_eq!(model_info.display_name, "Overlay");
    assert_eq!(model_info.context_window, Some(272_000));
    assert!(model_info.supports_image_detail_original);
    assert!(!model_info.supports_parallel_tool_calls);
    assert!(!model_info.used_fallback_model_metadata);
}

#[tokio::test]
async fn gpt_5_4_one_million_alias_is_available_with_canonical_api_slug() {
    let codex_home = tempdir().expect("temp dir");
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("load default test config");
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let manager = ModelsManager::new(
        codex_home.path().to_path_buf(),
        auth_manager,
        None,
        CollaborationModesConfig::default(),
    );

    let available = manager.list_models(RefreshStrategy::Offline).await;
    assert!(
        available
            .iter()
            .any(|preset| preset.model == GPT_5_4_ONE_MILLION_MODEL),
        "expected derived 1M alias in available models"
    );

    let model_info = manager
        .get_model_info(GPT_5_4_ONE_MILLION_MODEL, &config)
        .await;
    assert_eq!(model_info.slug, GPT_5_4_ONE_MILLION_MODEL);
    assert_eq!(model_info.display_name, GPT_5_4_ONE_MILLION_MODEL);
    assert_eq!(model_info.context_window, Some(1_050_000));
    assert_eq!(model_info.api_model_slug.as_deref(), Some(GPT_5_4_MODEL));
    assert_eq!(model_info.api_model_slug(), GPT_5_4_MODEL);
}

#[test]
fn derived_aliases_preserve_server_supplied_gpt_5_4_one_million_entry() {
    let mut server_alias = remote_model("gpt-5.4[1m]", "Server Alias", 7);
    server_alias.api_model_slug = Some("server-api-slug".to_string());
    server_alias.context_window = Some(2_000_000);
    server_alias.description = Some("server supplied alias".to_string());

    let models = ModelsManager::with_derived_aliases_for_tests(vec![
        remote_model(GPT_5_4_MODEL, "Base", 1),
        server_alias.clone(),
    ]);

    assert_eq!(models.len(), 2);
    assert_eq!(
        models
            .iter()
            .find(|model| model.slug == GPT_5_4_ONE_MILLION_MODEL),
        Some(&server_alias)
    );
}

#[tokio::test]
async fn get_model_info_matches_namespaced_suffix() {
    let codex_home = tempdir().expect("temp dir");
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("load default test config");
    let mut remote = remote_model("gpt-image", "Image", 0);
    remote.supports_image_detail_original = true;
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let manager = ModelsManager::new(
        codex_home.path().to_path_buf(),
        auth_manager,
        Some(ModelsResponse {
            models: vec![remote],
        }),
        CollaborationModesConfig::default(),
    );
    let namespaced_model = "custom/gpt-image".to_string();

    let model_info = manager.get_model_info(&namespaced_model, &config).await;

    assert_eq!(model_info.slug, namespaced_model);
    assert!(model_info.supports_image_detail_original);
    assert!(!model_info.used_fallback_model_metadata);
}

#[tokio::test]
async fn get_model_info_rejects_multi_segment_namespace_suffix_matching() {
    let codex_home = tempdir().expect("temp dir");
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("load default test config");
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let manager = ModelsManager::new(
        codex_home.path().to_path_buf(),
        auth_manager,
        None,
        CollaborationModesConfig::default(),
    );
    let known_slug = manager
        .get_remote_models()
        .await
        .first()
        .expect("bundled models should include at least one model")
        .slug
        .clone();
    let namespaced_model = format!("ns1/ns2/{known_slug}");

    let model_info = manager.get_model_info(&namespaced_model, &config).await;

    assert_eq!(model_info.slug, namespaced_model);
    assert!(model_info.used_fallback_model_metadata);
}

#[tokio::test]
async fn refresh_available_models_sorts_by_priority() {
    let server = MockServer::start().await;
    let remote_models = vec![
        remote_model("priority-low", "Low", 1),
        remote_model("priority-high", "High", 0),
    ];
    let models_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: remote_models.clone(),
        },
    )
    .await;

    let codex_home = tempdir().expect("temp dir");
    let auth_manager =
        AuthManager::from_auth_for_testing(CodexAuth::create_dummy_chatgpt_auth_for_testing());
    let provider = provider_for(server.uri());
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );

    manager
        .refresh_available_models(RefreshStrategy::OnlineIfUncached)
        .await
        .expect("refresh succeeds");
    let cached_remote = manager.get_remote_models().await;
    assert_models_contain(&cached_remote, &remote_models);

    let available = manager.list_models(RefreshStrategy::OnlineIfUncached).await;
    let high_idx = available
        .iter()
        .position(|model| model.model == "priority-high")
        .expect("priority-high should be listed");
    let low_idx = available
        .iter()
        .position(|model| model.model == "priority-low")
        .expect("priority-low should be listed");
    assert!(
        high_idx < low_idx,
        "higher priority should be listed before lower priority"
    );
    assert_eq!(
        models_mock.requests().len(),
        1,
        "expected a single /models request"
    );
}

#[tokio::test]
async fn refresh_available_models_uses_cache_when_fresh() {
    let server = MockServer::start().await;
    let remote_models = vec![remote_model("cached", "Cached", 5)];
    let models_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: remote_models.clone(),
        },
    )
    .await;

    let codex_home = tempdir().expect("temp dir");
    let auth_manager =
        AuthManager::from_auth_for_testing(CodexAuth::create_dummy_chatgpt_auth_for_testing());
    let provider = provider_for(server.uri());
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );

    manager
        .refresh_available_models(RefreshStrategy::OnlineIfUncached)
        .await
        .expect("first refresh succeeds");
    assert_models_contain(&manager.get_remote_models().await, &remote_models);

    // Second call should read from cache and avoid the network.
    manager
        .refresh_available_models(RefreshStrategy::OnlineIfUncached)
        .await
        .expect("cached refresh succeeds");
    assert_models_contain(&manager.get_remote_models().await, &remote_models);
    assert_eq!(
        models_mock.requests().len(),
        1,
        "cache hit should avoid a second /models request"
    );
}

#[tokio::test]
async fn refresh_available_models_refetches_when_cache_stale() {
    let server = MockServer::start().await;
    let initial_models = vec![remote_model("stale", "Stale", 1)];
    let initial_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: initial_models.clone(),
        },
    )
    .await;

    let codex_home = tempdir().expect("temp dir");
    let auth_manager =
        AuthManager::from_auth_for_testing(CodexAuth::create_dummy_chatgpt_auth_for_testing());
    let provider = provider_for(server.uri());
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );

    manager
        .refresh_available_models(RefreshStrategy::OnlineIfUncached)
        .await
        .expect("initial refresh succeeds");

    // Rewrite cache with an old timestamp so it is treated as stale.
    manager
        .cache_manager
        .read()
        .await
        .manipulate_cache_for_test(|fetched_at| {
            *fetched_at = Utc::now() - chrono::Duration::hours(1);
        })
        .await
        .expect("cache manipulation succeeds");

    let updated_models = vec![remote_model("fresh", "Fresh", 9)];
    server.reset().await;
    let refreshed_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: updated_models.clone(),
        },
    )
    .await;

    manager
        .refresh_available_models(RefreshStrategy::OnlineIfUncached)
        .await
        .expect("second refresh succeeds");
    assert_models_contain(&manager.get_remote_models().await, &updated_models);
    assert_eq!(
        initial_mock.requests().len(),
        1,
        "initial refresh should only hit /models once"
    );
    assert_eq!(
        refreshed_mock.requests().len(),
        1,
        "stale cache refresh should fetch /models once"
    );
}

#[tokio::test]
async fn refresh_available_models_refetches_when_version_mismatch() {
    let server = MockServer::start().await;
    let initial_models = vec![remote_model("old", "Old", 1)];
    let initial_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: initial_models.clone(),
        },
    )
    .await;

    let codex_home = tempdir().expect("temp dir");
    let auth_manager =
        AuthManager::from_auth_for_testing(CodexAuth::create_dummy_chatgpt_auth_for_testing());
    let provider = provider_for(server.uri());
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );

    manager
        .refresh_available_models(RefreshStrategy::OnlineIfUncached)
        .await
        .expect("initial refresh succeeds");

    manager
        .cache_manager
        .read()
        .await
        .mutate_cache_for_test(|cache| {
            let client_version = crate::models_manager::client_version_to_whole();
            cache.client_version = Some(format!("{client_version}-mismatch"));
        })
        .await
        .expect("cache mutation succeeds");

    let updated_models = vec![remote_model("new", "New", 2)];
    server.reset().await;
    let refreshed_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: updated_models.clone(),
        },
    )
    .await;

    manager
        .refresh_available_models(RefreshStrategy::OnlineIfUncached)
        .await
        .expect("second refresh succeeds");
    assert_models_contain(&manager.get_remote_models().await, &updated_models);
    assert_eq!(
        initial_mock.requests().len(),
        1,
        "initial refresh should only hit /models once"
    );
    assert_eq!(
        refreshed_mock.requests().len(),
        1,
        "version mismatch should fetch /models once"
    );
}

#[tokio::test]
async fn refresh_available_models_drops_removed_remote_models() {
    let server = MockServer::start().await;
    let initial_models = vec![remote_model("remote-old", "Remote Old", 1)];
    let initial_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: initial_models,
        },
    )
    .await;

    let codex_home = tempdir().expect("temp dir");
    let auth_manager =
        AuthManager::from_auth_for_testing(CodexAuth::create_dummy_chatgpt_auth_for_testing());
    let provider = provider_for(server.uri());
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );
    manager.cache_manager.write().await.set_ttl(Duration::ZERO);

    manager
        .refresh_available_models(RefreshStrategy::OnlineIfUncached)
        .await
        .expect("initial refresh succeeds");

    server.reset().await;
    let refreshed_models = vec![remote_model("remote-new", "Remote New", 1)];
    let refreshed_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: refreshed_models,
        },
    )
    .await;

    manager
        .refresh_available_models(RefreshStrategy::OnlineIfUncached)
        .await
        .expect("second refresh succeeds");

    let available = manager
        .try_list_models()
        .expect("models should be available");
    assert!(
        available.iter().any(|preset| preset.model == "remote-new"),
        "new remote model should be listed"
    );
    assert!(
        !available.iter().any(|preset| preset.model == "remote-old"),
        "removed remote model should not be listed"
    );
    assert_eq!(
        initial_mock.requests().len(),
        1,
        "initial refresh should only hit /models once"
    );
    assert_eq!(
        refreshed_mock.requests().len(),
        1,
        "second refresh should only hit /models once"
    );
}

#[tokio::test]
async fn refresh_available_models_fetches_remote_models_without_chatgpt_auth() {
    let server = MockServer::start().await;
    let dynamic_slug = "dynamic-model-only-for-test-noauth";
    let models_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: vec![remote_model(dynamic_slug, "No Auth", 1)],
        },
    )
    .await;

    let codex_home = tempdir().expect("temp dir");
    let auth_manager = Arc::new(AuthManager::new(
        codex_home.path().to_path_buf(),
        false,
        AuthCredentialsStoreMode::File,
    ));
    let provider = provider_for(server.uri());
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );

    manager
        .refresh_available_models(RefreshStrategy::Online)
        .await
        .expect("refresh should succeed without chatgpt auth");
    let cached_remote = manager.get_remote_models().await;
    assert!(
        cached_remote
            .iter()
            .any(|candidate| candidate.slug == dynamic_slug),
        "remote refresh should update models without chatgpt auth"
    );
    assert_eq!(
        models_mock.requests().len(),
        1,
        "non-chatgpt providers should still fetch /models"
    );
}

#[tokio::test]
async fn refresh_available_models_fetches_remote_models_when_provider_auth_is_missing() {
    let server = MockServer::start().await;
    let dynamic_slug = "dynamic-model-needs-auth";
    let models_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: vec![remote_model(dynamic_slug, "Needs Auth", 1)],
        },
    )
    .await;

    let codex_home = tempdir().expect("temp dir");
    let auth_manager = Arc::new(AuthManager::new(
        codex_home.path().to_path_buf(),
        false,
        AuthCredentialsStoreMode::File,
    ));
    let mut provider = provider_for(server.uri());
    provider.auth_strategy = ModelProviderAuthStrategy::ApiKey;
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );

    manager
        .refresh_available_models(RefreshStrategy::Online)
        .await
        .expect("missing provider credentials should still allow anonymous refresh");

    let available = manager
        .try_list_models()
        .expect("models should remain available after anonymous refresh");
    assert!(
        available.iter().any(|preset| preset.model == dynamic_slug),
        "anonymous refresh should still surface remote models"
    );
    assert_eq!(
        models_mock.requests().len(),
        1,
        "missing provider credentials should not block /models for anonymous providers"
    );
}

#[tokio::test]
async fn remote_gpt_5_4_derives_one_million_alias_with_context_window() {
    let server = MockServer::start().await;
    let remote_models = vec![remote_model(GPT_5_4_MODEL, "GPT 5.4", 1)];
    let models_mock = mount_models_once(
        &server,
        ModelsResponse {
            models: remote_models.clone(),
        },
    )
    .await;

    let codex_home = tempdir().expect("temp dir");
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("load default test config");
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let provider = provider_for(server.uri());
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );

    let available = manager.list_models(RefreshStrategy::OnlineIfUncached).await;
    assert!(
        available
            .iter()
            .any(|preset| preset.model == GPT_5_4_ONE_MILLION_MODEL),
        "remote gpt-5.4 should derive the 1M alias"
    );

    let alias_info = manager
        .get_model_info(GPT_5_4_ONE_MILLION_MODEL, &config)
        .await;
    assert_eq!(alias_info.context_window, Some(1_050_000));
    assert_eq!(alias_info.api_model_slug(), GPT_5_4_MODEL);
    assert_eq!(
        models_mock.requests().len(),
        1,
        "expected a single /models request for remote alias derivation"
    );
}

#[tokio::test]
async fn standard_openai_models_response_enriches_known_models_and_replaces_membership() {
    let server = MockServer::start().await;
    mount_openai_models_once(&server, &["gpt-5.4", "gpt-5.3-codex"]).await;

    let codex_home = tempdir().expect("temp dir");
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("load default test config");
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let provider = provider_for(server.uri());
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );

    let available = manager.list_models(RefreshStrategy::OnlineIfUncached).await;

    assert!(available.iter().any(|preset| preset.model == "gpt-5.4"));
    assert!(
        available
            .iter()
            .any(|preset| preset.model == "gpt-5.3-codex")
    );
    assert!(
        available
            .iter()
            .any(|preset| preset.model == GPT_5_4_ONE_MILLION_MODEL)
    );
    assert!(
        !available
            .iter()
            .any(|preset| preset.model == "gpt-5.2-codex"),
        "bundled models missing from the remote membership should not stay visible"
    );

    let model_info = manager.get_model_info("gpt-5.3-codex", &config).await;
    assert_eq!(
        model_info.description.as_deref(),
        Some("Latest frontier agentic coding model.")
    );
    assert_eq!(model_info.context_window, Some(272_000));
    assert!(model_info.support_verbosity);

    let requests = server
        .received_requests()
        .await
        .expect("capture /models requests");
    assert_eq!(requests.len(), 1);
}

#[tokio::test]
async fn standard_openai_models_response_surfaces_unknown_models_with_visible_fallback_metadata() {
    let server = MockServer::start().await;
    mount_openai_models_once(&server, &["acme-custom-model"]).await;

    let codex_home = tempdir().expect("temp dir");
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .build()
        .await
        .expect("load default test config");
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let provider = provider_for(server.uri());
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );

    let available = manager.list_models(RefreshStrategy::OnlineIfUncached).await;

    assert!(
        available
            .iter()
            .any(|preset| preset.model == "acme-custom-model"),
        "unknown remote models should still be selectable"
    );

    let model_info = manager.get_model_info("acme-custom-model", &config).await;
    assert_eq!(model_info.visibility, ModelVisibility::List);
    assert_eq!(
        model_info.description.as_deref(),
        Some("Remote model discovered from /v1/models.")
    );
    assert!(model_info.used_fallback_model_metadata);
}

#[tokio::test]
async fn list_models_result_surfaces_remote_refresh_errors() {
    let server = MockServer::start().await;
    let codex_home = tempdir().expect("temp dir");
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let provider = provider_for(server.uri());
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );

    let err = manager
        .list_models_result(RefreshStrategy::OnlineIfUncached)
        .await
        .expect_err("missing /models handler should surface an error");

    assert!(
        !err.to_string().is_empty(),
        "expected refresh error to carry context"
    );
}

#[tokio::test]
async fn replace_provider_uses_provider_specific_cache_and_remote_models() {
    let first_server = MockServer::start().await;
    let second_server = MockServer::start().await;
    let first_slug = "provider-one-model";
    let second_slug = "provider-two-model";

    let first_mock = mount_models_once(
        &first_server,
        ModelsResponse {
            models: vec![remote_model(first_slug, "Provider One", 1)],
        },
    )
    .await;
    let second_mock = mount_models_once(
        &second_server,
        ModelsResponse {
            models: vec![remote_model(second_slug, "Provider Two", 1)],
        },
    )
    .await;

    let codex_home = tempdir().expect("temp dir");
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let manager = ModelsManager::new_with_provider(
        codex_home.path().to_path_buf(),
        auth_manager,
        None,
        CollaborationModesConfig::default(),
        "provider-one".to_string(),
        provider_for(first_server.uri()),
        OAuthCredentialsStoreMode::default(),
    );

    manager
        .refresh_available_models(RefreshStrategy::Online)
        .await
        .expect("first provider refresh should succeed");
    assert!(
        manager
            .try_list_models()
            .expect("list models")
            .iter()
            .any(|preset| preset.model == first_slug)
    );

    manager
        .replace_provider(
            "provider-two".to_string(),
            provider_for(second_server.uri()),
        )
        .await;

    manager
        .refresh_available_models(RefreshStrategy::OnlineIfUncached)
        .await
        .expect("second provider refresh should succeed");

    let models = manager
        .try_list_models()
        .expect("list models after replacement");
    assert!(
        models.iter().any(|preset| preset.model == second_slug),
        "replacement should expose second provider models"
    );
    assert!(
        !models.iter().any(|preset| preset.model == first_slug),
        "replacement should not keep stale first provider cache entries"
    );
    assert_eq!(first_mock.requests().len(), 1);
    assert_eq!(second_mock.requests().len(), 1);
}

#[test]
fn build_available_models_picks_default_after_hiding_hidden_models() {
    let codex_home = tempdir().expect("temp dir");
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let provider = provider_for("http://example.test".to_string());
    let manager = ModelsManager::with_provider_for_tests(
        codex_home.path().to_path_buf(),
        auth_manager,
        provider,
    );

    let hidden_model = remote_model_with_visibility("hidden", "Hidden", 0, "hide");
    let visible_model = remote_model_with_visibility("visible", "Visible", 1, "list");

    let expected_hidden = ModelPreset::from(hidden_model.clone());
    let mut expected_visible = ModelPreset::from(visible_model.clone());
    expected_visible.is_default = true;

    let available = manager.build_available_models(vec![hidden_model, visible_model]);

    assert_eq!(available, vec![expected_hidden, expected_visible]);
}

#[test]
fn bundled_models_json_roundtrips() {
    let file_contents = include_str!("../../models.json");
    let response: ModelsResponse =
        serde_json::from_str(file_contents).expect("bundled models.json should deserialize");

    let serialized =
        serde_json::to_string(&response).expect("bundled models.json should serialize");
    let roundtripped: ModelsResponse =
        serde_json::from_str(&serialized).expect("serialized models.json should deserialize");

    assert_eq!(
        response, roundtripped,
        "bundled models.json should round trip through serde"
    );
    assert!(
        !response.models.is_empty(),
        "bundled models.json should contain at least one model"
    );
}
