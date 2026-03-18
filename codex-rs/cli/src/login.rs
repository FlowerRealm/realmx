//! CLI login commands and their direct-user observability surfaces.
//!
//! The TUI path already installs a broader tracing stack with feedback, OpenTelemetry, and other
//! interactive-session layers. Direct `codex login` intentionally does less: it preserves the
//! existing stderr/browser UX and adds only a small file-backed tracing layer for login-specific
//! targets. Keeping that setup local avoids pulling the TUI's session-oriented logging machinery
//! into a one-shot CLI command while still producing a durable `codex-login.log` artifact that
//! support can request from users.

use crate::branding::command_example;
use codex_core::CodexAuth;
use codex_core::ModelProviderInfo;
use codex_core::ProviderCredentialMode;
use codex_core::activate_provider_api_key;
use codex_core::auth::AuthCredentialsStoreMode;
use codex_core::auth::CLIENT_ID;
use codex_core::auth::login_with_api_key;
use codex_core::auth::logout;
use codex_core::clear_provider_credentials;
use codex_core::config::Config;
use codex_core::detect_provider_credential_mode;
use codex_core::provider_login_capabilities;
use codex_core::provider_oauth_url;
use codex_core::resolve_provider_credential;
use codex_login::ServerOptions;
use codex_login::run_device_code_login;
use codex_login::run_login_server;
use codex_protocol::config_types::ForcedLoginMethod;
use codex_rmcp_client::perform_oauth_login;
use codex_utils_cli::CliConfigOverrides;
use std::fs::OpenOptions;
use std::io::IsTerminal;
use std::io::Read;
use std::path::PathBuf;
use tracing_appender::non_blocking;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const CHATGPT_LOGIN_DISABLED_MESSAGE: &str =
    "ChatGPT login is disabled. Use API key login instead.";
const API_KEY_LOGIN_DISABLED_MESSAGE: &str =
    "API key login is disabled. Use ChatGPT login instead.";
const LOGIN_SUCCESS_MESSAGE: &str = "Successfully logged in";

/// Installs a small file-backed tracing layer for direct `codex login` flows.
///
/// This deliberately duplicates a narrow slice of the TUI logging setup instead of reusing it
/// wholesale. The TUI stack includes session-oriented layers that are valuable for interactive
/// runs but unnecessary for a one-shot login command. Keeping the direct CLI path local lets this
/// command produce a durable `codex-login.log` artifact without coupling it to the TUI's broader
/// telemetry and feedback initialization.
fn init_login_file_logging(config: &Config) -> Option<WorkerGuard> {
    let log_dir = match codex_core::config::log_dir(config) {
        Ok(log_dir) => log_dir,
        Err(err) => {
            eprintln!("Warning: failed to resolve login log directory: {err}");
            return None;
        }
    };

    if let Err(err) = std::fs::create_dir_all(&log_dir) {
        eprintln!(
            "Warning: failed to create login log directory {}: {err}",
            log_dir.display()
        );
        return None;
    }

    let mut log_file_opts = OpenOptions::new();
    log_file_opts.create(true).append(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        log_file_opts.mode(0o600);
    }

    let log_path = log_dir.join("codex-login.log");
    let log_file = match log_file_opts.open(&log_path) {
        Ok(log_file) => log_file,
        Err(err) => {
            eprintln!(
                "Warning: failed to open login log file {}: {err}",
                log_path.display()
            );
            return None;
        }
    };

    let (non_blocking, guard) = non_blocking(log_file);
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("codex_cli=info,codex_core=info,codex_login=info"));
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_target(true)
        .with_ansi(false)
        .with_filter(env_filter);

    // Direct `codex login` otherwise relies on ephemeral stderr and browser output.
    // Persist the same login targets to a file so support can inspect auth failures
    // without reproducing them through TUI or app-server.
    if let Err(err) = tracing_subscriber::registry().with(file_layer).try_init() {
        eprintln!(
            "Warning: failed to initialize login log file {}: {err}",
            log_path.display()
        );
        return None;
    }

    Some(guard)
}

fn print_login_server_start(actual_port: u16, auth_url: &str) {
    let device_auth_command = command_example("login --device-auth");
    eprintln!(
        "Starting local login server on http://localhost:{actual_port}.\nIf your browser did not open, navigate to this URL to authenticate:\n\n{auth_url}\n\nOn a remote or headless machine? Use `{device_auth_command}` instead."
    );
}

fn resolve_login_provider(
    config: &Config,
    requested_provider_id: Option<&str>,
) -> Result<(String, ModelProviderInfo), String> {
    let provider_id = requested_provider_id.unwrap_or(&config.model_provider_id);
    config
        .model_providers
        .get(provider_id)
        .cloned()
        .map(|provider| (provider_id.to_string(), provider))
        .ok_or_else(|| format!("Unknown provider `{provider_id}`"))
}

fn login_success_message(provider_id: &str) -> String {
    format!("Successfully logged in to provider `{provider_id}`")
}

pub async fn login_with_chatgpt(
    codex_home: PathBuf,
    forced_chatgpt_workspace_id: Option<String>,
    cli_auth_credentials_store_mode: AuthCredentialsStoreMode,
) -> std::io::Result<()> {
    let opts = ServerOptions::new(
        codex_home,
        CLIENT_ID.to_string(),
        forced_chatgpt_workspace_id,
        cli_auth_credentials_store_mode,
    );
    let server = run_login_server(opts)?;

    print_login_server_start(server.actual_port, &server.auth_url);

    server.block_until_done().await
}

pub async fn run_login_with_chatgpt(
    cli_config_overrides: CliConfigOverrides,
    requested_provider_id: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let _login_log_guard = init_login_file_logging(&config);
    tracing::info!("starting browser login flow");

    let (provider_id, provider) =
        match resolve_login_provider(&config, requested_provider_id.as_deref()) {
            Ok(provider) => provider,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
    let capabilities = provider_login_capabilities(&provider_id, &provider);

    if capabilities.chatgpt {
        if matches!(config.forced_login_method, Some(ForcedLoginMethod::Api)) {
            eprintln!("{CHATGPT_LOGIN_DISABLED_MESSAGE}");
            std::process::exit(1);
        }

        match login_with_chatgpt(
            config.codex_home,
            config.forced_chatgpt_workspace_id.clone(),
            config.cli_auth_credentials_store_mode,
        )
        .await
        {
            Ok(_) => {
                eprintln!("{LOGIN_SUCCESS_MESSAGE}");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("Error logging in: {e}");
                std::process::exit(1);
            }
        }
    }

    if !capabilities.oauth {
        if capabilities.api_key {
            eprintln!(
                "Provider `{provider_id}` expects an API key. Use `codex login --provider {provider_id} --with-api-key`."
            );
            std::process::exit(1);
        }
        eprintln!("Provider `{provider_id}` does not require login.");
        std::process::exit(0);
    }

    let oauth_scopes = provider
        .oauth
        .as_ref()
        .and_then(|oauth| oauth.scopes.clone())
        .unwrap_or_default();
    let oauth_resource = provider
        .oauth
        .as_ref()
        .and_then(|oauth| oauth.oauth_resource.as_deref());

    match perform_oauth_login(
        &format!("model-provider:{provider_id}"),
        provider_oauth_url(&provider_id, &provider).unwrap_or_default(),
        config.mcp_oauth_credentials_store_mode,
        provider.http_headers.clone(),
        provider.env_http_headers.clone(),
        &oauth_scopes,
        oauth_resource,
        config.mcp_oauth_callback_port,
        config.mcp_oauth_callback_url.as_deref(),
    )
    .await
    {
        Ok(()) => {
            eprintln!("{}", login_success_message(&provider_id));
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in: {e}");
            std::process::exit(1);
        }
    }
}

pub async fn run_login_with_api_key(
    cli_config_overrides: CliConfigOverrides,
    requested_provider_id: Option<String>,
    api_key: String,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let _login_log_guard = init_login_file_logging(&config);
    tracing::info!("starting api key login flow");

    let (provider_id, provider) =
        match resolve_login_provider(&config, requested_provider_id.as_deref()) {
            Ok(provider) => provider,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
    let capabilities = provider_login_capabilities(&provider_id, &provider);

    if capabilities.uses_openai_auth() {
        if matches!(config.forced_login_method, Some(ForcedLoginMethod::Chatgpt)) {
            eprintln!("{API_KEY_LOGIN_DISABLED_MESSAGE}");
            std::process::exit(1);
        }

        match login_with_api_key(
            &config.codex_home,
            &api_key,
            config.cli_auth_credentials_store_mode,
        ) {
            Ok(_) => {
                eprintln!("{LOGIN_SUCCESS_MESSAGE}");
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("Error logging in: {e}");
                std::process::exit(1);
            }
        }
    }

    if !capabilities.api_key {
        eprintln!("Provider `{provider_id}` does not support API key login.");
        std::process::exit(1);
    }

    match activate_provider_api_key(
        &config.codex_home,
        &provider_id,
        &provider,
        config.mcp_oauth_credentials_store_mode,
        &api_key,
    ) {
        Ok(()) => {
            eprintln!("{}", login_success_message(&provider_id));
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in: {e}");
            std::process::exit(1);
        }
    }
}

pub fn read_api_key_from_stdin() -> String {
    let mut stdin = std::io::stdin();

    if stdin.is_terminal() {
        let login_command = command_example("login --with-api-key");
        eprintln!(
            "--with-api-key expects the API key on stdin. Try piping it, e.g. `printenv SOME_API_KEY | {login_command}`."
        );
        std::process::exit(1);
    }

    eprintln!("Reading API key from stdin...");

    let mut buffer = String::new();
    if let Err(err) = stdin.read_to_string(&mut buffer) {
        eprintln!("Failed to read API key from stdin: {err}");
        std::process::exit(1);
    }

    let api_key = buffer.trim().to_string();
    if api_key.is_empty() {
        eprintln!("No API key provided via stdin.");
        std::process::exit(1);
    }

    api_key
}

/// Login using the OAuth device code flow.
pub async fn run_login_with_device_code(
    cli_config_overrides: CliConfigOverrides,
    requested_provider_id: Option<String>,
    issuer_base_url: Option<String>,
    client_id: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let _login_log_guard = init_login_file_logging(&config);
    tracing::info!("starting device code login flow");
    let (provider_id, provider) =
        match resolve_login_provider(&config, requested_provider_id.as_deref()) {
            Ok(provider) => provider,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
    if !provider_login_capabilities(&provider_id, &provider).device_code {
        eprintln!("Provider `{provider_id}` does not support ChatGPT device-code login.");
        std::process::exit(1);
    }
    if matches!(config.forced_login_method, Some(ForcedLoginMethod::Api)) {
        eprintln!("{CHATGPT_LOGIN_DISABLED_MESSAGE}");
        std::process::exit(1);
    }
    let forced_chatgpt_workspace_id = config.forced_chatgpt_workspace_id.clone();
    let mut opts = ServerOptions::new(
        config.codex_home,
        client_id.unwrap_or(CLIENT_ID.to_string()),
        forced_chatgpt_workspace_id,
        config.cli_auth_credentials_store_mode,
    );
    if let Some(iss) = issuer_base_url {
        opts.issuer = iss;
    }
    match run_device_code_login(opts).await {
        Ok(()) => {
            eprintln!("{LOGIN_SUCCESS_MESSAGE}");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging in with device code: {e}");
            std::process::exit(1);
        }
    }
}

/// Prefers device-code login (with `open_browser = false`) when headless environment is detected, but keeps
/// `codex login` working in environments where device-code may be disabled/feature-gated.
/// If `run_device_code_login` returns `ErrorKind::NotFound` ("device-code unsupported"), this
/// falls back to starting the local browser login server.
pub async fn run_login_with_device_code_fallback_to_browser(
    cli_config_overrides: CliConfigOverrides,
    issuer_base_url: Option<String>,
    client_id: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;
    let _login_log_guard = init_login_file_logging(&config);
    tracing::info!("starting login flow with device code fallback");
    if matches!(config.forced_login_method, Some(ForcedLoginMethod::Api)) {
        eprintln!("{CHATGPT_LOGIN_DISABLED_MESSAGE}");
        std::process::exit(1);
    }

    let forced_chatgpt_workspace_id = config.forced_chatgpt_workspace_id.clone();
    let mut opts = ServerOptions::new(
        config.codex_home,
        client_id.unwrap_or(CLIENT_ID.to_string()),
        forced_chatgpt_workspace_id,
        config.cli_auth_credentials_store_mode,
    );
    if let Some(iss) = issuer_base_url {
        opts.issuer = iss;
    }
    opts.open_browser = false;

    match run_device_code_login(opts.clone()).await {
        Ok(()) => {
            eprintln!("{LOGIN_SUCCESS_MESSAGE}");
            std::process::exit(0);
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                eprintln!("Device code login is not enabled; falling back to browser login.");
                match run_login_server(opts) {
                    Ok(server) => {
                        print_login_server_start(server.actual_port, &server.auth_url);
                        match server.block_until_done().await {
                            Ok(()) => {
                                eprintln!("{LOGIN_SUCCESS_MESSAGE}");
                                std::process::exit(0);
                            }
                            Err(e) => {
                                eprintln!("Error logging in: {e}");
                                std::process::exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Error logging in: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                eprintln!("Error logging in with device code: {e}");
                std::process::exit(1);
            }
        }
    }
}

pub async fn run_login_status(
    cli_config_overrides: CliConfigOverrides,
    requested_provider_id: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;

    let (provider_id, provider) =
        match resolve_login_provider(&config, requested_provider_id.as_deref()) {
            Ok(provider) => provider,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
    let capabilities = provider_login_capabilities(&provider_id, &provider);

    if !capabilities.requires_auth() {
        eprintln!("Provider `{provider_id}` does not require login.");
        std::process::exit(0);
    }

    let openai_auth = match CodexAuth::from_auth_storage(
        &config.codex_home,
        config.cli_auth_credentials_store_mode,
    ) {
        Ok(auth) => auth,
        Err(e) => {
            eprintln!("Error checking login status: {e}");
            std::process::exit(1);
        }
    };

    let status = match detect_provider_credential_mode(
        &config.codex_home,
        &provider_id,
        &provider,
        openai_auth.as_ref(),
        config.mcp_oauth_credentials_store_mode,
    ) {
        Ok(status) => status,
        Err(e) => {
            eprintln!("Error checking login status: {e}");
            std::process::exit(1);
        }
    };

    match status {
        Some(ProviderCredentialMode::ApiKey) => {
            let credential = match resolve_provider_credential(
                &config.codex_home,
                &provider_id,
                &provider,
                openai_auth,
                config.mcp_oauth_credentials_store_mode,
            )
            .await
            {
                Ok(credential) => credential,
                Err(e) => {
                    eprintln!("Error checking login status: {e}");
                    std::process::exit(1);
                }
            };
            if let Some(api_key) = credential.token {
                eprintln!(
                    "Provider `{provider_id}` is using an API key - {}",
                    safe_format_key(&api_key)
                );
                std::process::exit(0);
            }
            eprintln!("Provider `{provider_id}` is configured for API key auth.");
            std::process::exit(0);
        }
        Some(ProviderCredentialMode::Chatgpt) => {
            eprintln!("Provider `{provider_id}` is using ChatGPT login.");
            std::process::exit(0);
        }
        Some(ProviderCredentialMode::OAuth) => {
            eprintln!("Provider `{provider_id}` is using OAuth.");
            std::process::exit(0);
        }
        None => {
            eprintln!("Provider `{provider_id}` is not logged in.");
            std::process::exit(1);
        }
    }
}

pub async fn run_logout(
    cli_config_overrides: CliConfigOverrides,
    requested_provider_id: Option<String>,
) -> ! {
    let config = load_config_or_exit(cli_config_overrides).await;

    let (provider_id, provider) =
        match resolve_login_provider(&config, requested_provider_id.as_deref()) {
            Ok(provider) => provider,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
    let capabilities = provider_login_capabilities(&provider_id, &provider);

    let result = if capabilities.uses_openai_auth() {
        logout(&config.codex_home, config.cli_auth_credentials_store_mode)
    } else {
        clear_provider_credentials(
            &config.codex_home,
            &provider_id,
            &provider,
            config.mcp_oauth_credentials_store_mode,
        )
        .map_err(std::io::Error::other)
    };

    match result {
        Ok(true) => {
            eprintln!("Successfully logged out from provider `{provider_id}`");
            std::process::exit(0);
        }
        Ok(false) => {
            eprintln!("Provider `{provider_id}` was not logged in");
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("Error logging out: {e}");
            std::process::exit(1);
        }
    }
}

async fn load_config_or_exit(cli_config_overrides: CliConfigOverrides) -> Config {
    let cli_overrides = match cli_config_overrides.parse_overrides() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Error parsing -c overrides: {e}");
            std::process::exit(1);
        }
    };

    match Config::load_with_cli_overrides(cli_overrides).await {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Error loading configuration: {e}");
            std::process::exit(1);
        }
    }
}

fn safe_format_key(key: &str) -> String {
    if key.len() <= 13 {
        return "***".to_string();
    }
    let prefix = &key[..8];
    let suffix = &key[key.len() - 5..];
    format!("{prefix}***{suffix}")
}

#[cfg(test)]
mod tests {
    use super::safe_format_key;

    #[test]
    fn formats_long_key() {
        let key = "sk-proj-1234567890ABCDE";
        assert_eq!(safe_format_key(key), "sk-proj-***ABCDE");
    }

    #[test]
    fn short_key_returns_stars() {
        let key = "sk-proj-12345";
        assert_eq!(safe_format_key(key), "***");
    }
}
