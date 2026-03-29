//! Root of the `codex-core` library.

// Prevent accidental direct writes to stdout/stderr in library code. All
// user-visible output must go through the appropriate abstraction (e.g.,
// the TUI or the tracing stack).
#![deny(clippy::print_stdout, clippy::print_stderr)]
#![recursion_limit = "256"]

pub mod api_bridge;
mod apply_patch;
mod apps;
mod arc_monitor;
pub use codex_login as auth;
mod auth_env_telemetry;
mod client;
mod client_common;
pub mod codex;
mod realtime_context;
mod realtime_conversation;
pub use codex::SteerInputError;
mod codex_thread;
mod compact_remote;
pub use codex_thread::CodexThread;
pub use codex_thread::ThreadConfigSnapshot;
mod agent;
mod codex_delegate;
mod command_canonicalization;
mod commit_attribution;
pub mod config;
pub mod config_loader;
pub mod connectors;
mod context_manager;
mod contextual_user_message;
pub mod custom_prompts;
pub use codex_utils_path::env;
mod environment_context;
pub mod error;
pub mod exec;
pub mod exec_env;
mod exec_policy;
mod execute_plan_dispatch;
mod execute_plan_guard;
pub mod external_agent_config;
pub mod file_watcher;
mod flags;
pub(crate) mod features {
    pub(crate) use codex_features::FEATURES;
    pub(crate) use codex_features::Feature;
}
#[cfg(test)]
mod git_info_tests;
pub(crate) mod git_info {

    pub(crate) use codex_git_utils::resolve_root_git_project_for_trust;
}
mod guardian;
mod hook_runtime;
pub mod instructions;
pub mod landlock;
pub mod mcp;
pub(crate) mod terminal {}
mod mcp_connection_manager;
mod mcp_tool_approval_templates;
pub mod models_manager;
mod network_policy_decision;
pub mod network_proxy_loader;
mod original_image_detail;
mod plan_csv;
mod plan_display;
mod plan_review;
pub mod plan_workspace;
mod provider_credentials;
mod provider_id;
mod provider_login_capabilities;
pub use mcp_connection_manager::MCP_SANDBOX_STATE_CAPABILITY;
pub use mcp_connection_manager::MCP_SANDBOX_STATE_METHOD;
pub use mcp_connection_manager::SandboxState;
pub use text_encoding::bytes_to_string_smart;
mod mcp_tool_call;
mod memories;
pub mod mention_syntax;
pub mod message_history;
mod model_provider_info;
pub mod utils;
pub use utils::path_utils;
pub mod personality_migration;
pub mod plugins;
pub(crate) mod mentions {
    pub(crate) use crate::plugins::build_connector_slug_counts;
    pub(crate) use crate::plugins::build_skill_name_counts;
    pub(crate) use crate::plugins::collect_explicit_app_ids;
    pub(crate) use crate::plugins::collect_explicit_plugin_mentions;
    pub(crate) use crate::plugins::collect_tool_mentions_from_messages;
}
mod sandbox_tags;
pub mod sandboxing;
mod session_prefix;
mod session_startup_prewarm;
mod shell_detect;
pub mod skills;
pub(crate) use skills::SkillError;
pub(crate) use skills::SkillInjections;
pub(crate) use skills::SkillLoadOutcome;
pub(crate) use skills::SkillMetadata;
pub(crate) use skills::SkillsLoadInput;
pub(crate) use skills::SkillsManager;
pub(crate) use skills::build_skill_injections;
pub(crate) use skills::build_skill_name_counts;
pub(crate) use skills::collect_env_var_dependencies;
pub(crate) use skills::collect_explicit_skill_mentions;
pub(crate) use skills::config_rules;
pub(crate) use skills::injection;
pub(crate) use skills::loader;
pub(crate) use skills::manager;
pub(crate) use skills::maybe_emit_implicit_skill_invocation;
pub(crate) use skills::model;
pub(crate) use skills::render_skills_section;
pub(crate) use skills::resolve_skill_dependencies_for_turn;
pub(crate) use skills::skills_load_input_from_config;
mod skills_watcher;
mod stream_events_utils;
pub mod test_support;
mod text_encoding;
pub(crate) mod truncate {}
pub use codex_login::token_data;
mod unified_exec;
pub mod windows_sandbox;
pub use client::X_RESPONSESAPI_INCLUDE_TIMING_METRICS_HEADER;
pub use codex_rmcp_client::OAuthCredentialsStoreMode;
pub use codex_rmcp_client::OauthLoginHandle;
pub use codex_rmcp_client::perform_oauth_login_return_url;
pub use model_provider_info::DEFAULT_LMSTUDIO_PORT;
pub use model_provider_info::DEFAULT_OLLAMA_PORT;
pub use model_provider_info::LMSTUDIO_OSS_PROVIDER_ID;
pub use model_provider_info::ModelProviderAuthStrategy;
pub use model_provider_info::ModelProviderInfo;
pub use model_provider_info::ModelProviderOAuthConfig;
pub use model_provider_info::OLLAMA_OSS_PROVIDER_ID;
pub use model_provider_info::OPENAI_PROVIDER_ID;
pub use model_provider_info::WireApi;
pub use model_provider_info::built_in_model_providers;
pub use model_provider_info::create_oss_provider_with_base_url;
pub use provider_credentials::ProviderCredentialMode;
pub use provider_credentials::ResolvedProviderCredential;
pub use provider_credentials::activate_provider_api_key;
pub use provider_credentials::clear_provider_api_key;
pub use provider_credentials::clear_provider_credentials;
pub use provider_credentials::clear_provider_oauth_tokens;
pub use provider_credentials::detect_provider_credential_mode;
pub use provider_credentials::has_provider_oauth_tokens;
pub use provider_credentials::read_provider_api_key;
pub use provider_credentials::rename_provider_api_key;
pub use provider_credentials::resolve_provider_credential;
pub use provider_credentials::store_provider_api_key;
pub use provider_id::model_provider_id_requirements;
pub use provider_id::validate_model_provider_id;
pub use provider_id::validate_model_provider_reference;
pub use provider_login_capabilities::ProviderLoginCapabilities;
pub use provider_login_capabilities::provider_login_capabilities;
pub use provider_login_capabilities::provider_oauth_url;
mod event_mapping;
pub use plan_csv::CanonicalPlanCsv;
pub use plan_csv::canonical_plan_csv_from_update_plan_args;
pub use plan_display::PlanDisplayLayer;
pub use plan_display::PlanDisplayMode;
pub use plan_display::PlanDisplayProjection;
pub use plan_display::PlanDisplayRow;
pub use plan_display::project_plan_items;
pub use plan_display::render_plan_markdown;
mod response_debug_context;
pub mod review_format;
pub mod review_prompts;
mod thread_manager;
pub mod web_search;
pub mod windows_sandbox_read_grants;
pub use thread_manager::ForkSnapshot;
pub use thread_manager::NewThread;
pub use thread_manager::ThreadManager;
#[deprecated(note = "use ThreadManager")]
pub type ConversationManager = ThreadManager;
#[deprecated(note = "use NewThread")]
pub type NewConversation = NewThread;
#[deprecated(note = "use CodexThread")]
pub type CodexConversation = CodexThread;
// Re-export common auth types for workspace consumers
pub use auth::AuthManager;
pub use auth::AuthScope;
pub use auth::CodexAuth;
pub use codex_analytics::AnalyticsEventsClient;
mod default_client_forwarding;

/// Default Codex HTTP client headers and reqwest construction.
///
/// Implemented in [`codex_login::default_client`]; this module re-exports that API for crates
/// that import `codex_core::default_client`.
pub mod default_client {
    pub use super::default_client_forwarding::*;
}
pub mod project_doc;
mod rollout;
pub(crate) mod safety;
pub mod seatbelt;
mod session_rollout_init_error;
pub mod shell;
pub mod shell_snapshot;
pub mod spawn;
pub mod state_db_bridge;
pub use codex_rollout::state_db;
mod thread_rollout_truncation;
mod tools;
pub mod turn_diff_tracker;
mod turn_metadata;
mod turn_timing;
pub use rollout::ARCHIVED_SESSIONS_SUBDIR;
pub use rollout::INTERACTIVE_SESSION_SOURCES;
pub use rollout::RolloutRecorder;
pub use rollout::RolloutRecorderParams;
pub use rollout::SESSIONS_SUBDIR;
pub use rollout::SessionMeta;
pub use rollout::append_thread_name;
pub use rollout::find_archived_thread_path_by_id_str;
#[deprecated(note = "use find_thread_path_by_id_str")]
pub use rollout::find_conversation_path_by_id_str;
pub use rollout::find_thread_name_by_id;
pub use rollout::find_thread_path_by_id_str;
pub use rollout::find_thread_path_by_name_str;
pub use rollout::list::Cursor;
pub use rollout::list::ThreadItem;
pub use rollout::list::ThreadSortKey;
pub use rollout::list::ThreadsPage;
pub use rollout::list::parse_cursor;
pub use rollout::list::read_head_for_summary;
pub use rollout::list::read_session_meta_line;
pub use rollout::policy::EventPersistenceMode;
pub use rollout::rollout_date_parts;
pub use rollout::session_index::find_thread_names_by_ids;
mod function_tool;
mod state;
mod tasks;
mod user_shell_command;
pub mod util;
pub(crate) use codex_protocol::protocol;
pub(crate) use codex_shell_command::bash;
pub(crate) use codex_shell_command::is_dangerous_command;
pub(crate) use codex_shell_command::is_safe_command;
pub(crate) use codex_shell_command::parse_command;
pub(crate) use codex_shell_command::powershell;

pub use client::ModelClient;
pub use client::ModelClientSession;
pub use client::X_CODEX_TURN_METADATA_HEADER;
pub use client_common::Prompt;
pub use client_common::REVIEW_PROMPT;
pub use client_common::ResponseEvent;
pub use client_common::ResponseStream;
pub use codex_sandboxing::get_platform_sandbox;
pub use compact::content_items_to_text;
pub use event_mapping::parse_turn_item;
pub use exec_policy::ExecPolicyError;
pub use exec_policy::check_execpolicy_for_warnings;
pub use exec_policy::format_exec_policy_error_with_source;
pub use exec_policy::load_exec_policy;
pub use file_watcher::FileWatcherEvent;
pub use tools::spec::parse_tool_input_schema;
pub use turn_metadata::build_turn_metadata_header;
pub mod compact;
pub mod memory_trace;
pub mod otel_init;
