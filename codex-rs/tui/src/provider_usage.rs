use codex_app_server_protocol::ConfigLayerSource;
use codex_core::CodexAuth;
use codex_core::ModelProviderInfo;
use codex_core::config::Config;
use codex_core::config_loader::ConfigLayerStackOrdering;
use codex_core::default_client::build_reqwest_client;
use codex_core::git_info::resolve_root_git_project_for_trust;
use codex_core::path_utils::write_atomically;
use reqwest::Method;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderName;
use reqwest::header::HeaderValue;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;
use tokio::task;
use url::Url;

const PROVIDER_USAGE_SCRIPT: &str = "usage.js";
const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
const USAGE_SCRIPT_TIMEOUT: Duration = Duration::from_secs(10);
const USAGE_SCRIPT_RUNNER: &str = r#"const { readFileSync } = require("node:fs");
const vm = require("node:vm");

(async () => {
  const scriptPath = process.env.CODEX_PROVIDER_USAGE_SCRIPT;
  const mode = process.env.CODEX_PROVIDER_USAGE_MODE;
  const payload = JSON.parse(process.env.CODEX_PROVIDER_USAGE_PAYLOAD ?? "null");
  if (!scriptPath) {
    throw new Error("CODEX_PROVIDER_USAGE_SCRIPT is required");
  }

  const source = readFileSync(scriptPath, "utf8");
  const sandbox = {
    module: { exports: {} },
    exports: {},
    console,
    process,
    URL,
    URLSearchParams,
    setTimeout,
    clearTimeout,
  };
  const evaluated = vm.runInNewContext(source, sandbox, { filename: scriptPath });
  const exported =
    sandbox.module.exports?.default ??
    sandbox.module.exports ??
    sandbox.exports?.default ??
    sandbox.exports;
  const api = evaluated ?? exported;

  if (!api || typeof api !== "object") {
    throw new Error("usage script must evaluate to an object");
  }

  let result;
  if (mode === "request") {
    if (api.request && typeof api.request === "object") {
      result = api.request;
    } else {
      throw new Error("usage script must define request");
    }
  } else if (mode === "response") {
    if (typeof api.extractor === "function") {
      result = await api.extractor(payload?.extractorInput);
    } else {
      throw new Error("usage script must define extractor()");
    }
  } else {
    throw new Error(`Unknown usage script mode: ${mode}`);
  }

  process.stdout.write(JSON.stringify(result ?? null));
})().catch((err) => {
  console.error(err && err.stack ? err.stack : String(err));
  process.exit(1);
});
"#;

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ProviderUsageSnapshot {
    pub(crate) plans: Vec<ProviderUsagePlan>,
    pub(crate) error_message: Option<String>,
}

impl ProviderUsageSnapshot {
    pub(crate) fn summary_plan(&self) -> Option<ProviderUsagePlan> {
        match self.plans.as_slice() {
            [] => None,
            [plan] => Some(plan.clone()),
            plans => aggregate_plans(plans),
        }
    }

    pub(crate) fn remote_usage_summary(&self) -> Option<String> {
        if self.error_message.is_some() {
            return None;
        }
        self.summary_plan().and_then(|plan| {
            let mut parts = Vec::new();
            if let Some(remaining) = plan.remaining {
                parts.push(format!(
                    "rem {}",
                    format_usage_amount(remaining, plan.unit.as_deref())
                ));
            }
            if let Some(used) = plan.used {
                parts.push(format!(
                    "used {}",
                    format_usage_amount(used, plan.unit.as_deref())
                ));
            }
            if let Some(total) = plan.total {
                parts.push(format!(
                    "total {}",
                    format_usage_amount(total, plan.unit.as_deref())
                ));
            }
            if let Some(extra) = plan.extra.as_ref().filter(|extra| !extra.trim().is_empty()) {
                parts.push(extra.clone());
            }
            (!parts.is_empty()).then(|| parts.join(" | "))
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ProviderUsageRefreshResult {
    Updated(ProviderUsageSnapshot),
    Skipped,
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ProviderUsagePlan {
    pub(crate) plan_name: Option<String>,
    pub(crate) remaining: Option<f64>,
    pub(crate) used: Option<f64>,
    pub(crate) total: Option<f64>,
    pub(crate) unit: Option<String>,
    pub(crate) extra: Option<String>,
}

impl ProviderUsagePlan {
    fn has_content(&self) -> bool {
        self.plan_name.is_some()
            || self.remaining.is_some()
            || self.used.is_some()
            || self.total.is_some()
            || self.unit.is_some()
            || self.extra.is_some()
    }

    pub(crate) fn status_label(&self, index: usize) -> String {
        self.plan_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                if index == 0 {
                    "Remote usage".to_string()
                } else {
                    format!("Remote usage {}", index + 1)
                }
            })
    }

    pub(crate) fn summary_text(&self) -> Option<String> {
        format_provider_usage_plan(self)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderUsageEditorState {
    pub(crate) provider_id: String,
    pub(crate) provider_name: String,
    pub(crate) script_path: PathBuf,
    pub(crate) initial_contents: String,
    pub(crate) has_existing_script: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptRequestPlan {
    method: Option<String>,
    url: String,
    headers: Option<HashMap<String, String>>,
    #[serde(alias = "body")]
    body_text: Option<String>,
    body_json: Option<JsonValue>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptRequestContext {
    provider: ScriptProviderContext,
    auth: Option<ScriptAuthContext>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptProviderContext {
    id: String,
    name: String,
    base_url: Option<String>,
    api_key: Option<String>,
    env_key: Option<String>,
    experimental_bearer_token: Option<String>,
    query_params: Option<HashMap<String, String>>,
    http_headers: Option<HashMap<String, String>>,
    env_http_headers: Option<HashMap<String, String>>,
    requires_openai_auth: bool,
}

impl ScriptProviderContext {
    fn from_provider(id: &str, provider: &ModelProviderInfo) -> Self {
        Self {
            id: id.to_string(),
            name: provider.name.clone(),
            base_url: provider.base_url.clone(),
            api_key: provider.api_key.clone(),
            env_key: provider.env_key.clone(),
            experimental_bearer_token: provider.experimental_bearer_token.clone(),
            query_params: provider.query_params.clone(),
            http_headers: provider.http_headers.clone(),
            env_http_headers: provider.env_http_headers.clone(),
            requires_openai_auth: provider.requires_openai_auth,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptAuthContext {
    bearer_token: Option<String>,
    account_id: Option<String>,
}

impl ScriptAuthContext {
    fn from_auth(auth: &CodexAuth) -> Self {
        Self {
            bearer_token: auth.get_token().ok(),
            account_id: auth.get_account_id(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptResponseContext {
    status: u16,
    ok: bool,
    headers: HashMap<String, String>,
    body_text: String,
    body_json: Option<JsonValue>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScriptRunnerResponsePayload {
    extractor_input: JsonValue,
    response_context: ScriptResponseContext,
}

#[derive(Debug, Deserialize)]
struct ScriptedUsageRow {
    #[serde(rename = "planName")]
    plan_name: Option<String>,
    remaining: Option<f64>,
    used: Option<f64>,
    total: Option<f64>,
    unit: Option<String>,
    #[serde(rename = "isValid")]
    is_valid: Option<bool>,
    extra: Option<String>,
}

pub(crate) fn can_edit_provider_usage_scripts(config: &Config) -> bool {
    trusted_project_root(config).is_some()
}

pub(crate) fn provider_usage_enabled(config: &Config) -> bool {
    active_provider_usage_script_path(config).is_some()
}

pub(crate) fn provider_usage_poll_interval(config: &Config) -> Option<Duration> {
    active_provider_usage_script_path(config)
        .map(|_| Duration::from_secs(DEFAULT_POLL_INTERVAL_SECS))
}

pub(crate) fn provider_usage_editor_state(
    config: &Config,
    provider_id: &str,
) -> Result<ProviderUsageEditorState, String> {
    let provider = config
        .model_providers
        .get(provider_id)
        .ok_or_else(|| format!("Model provider `{provider_id}` not found"))?;
    let script_path = provider_usage_script_path(config, provider_id)?;
    let initial_contents = match std::fs::read_to_string(&script_path) {
        Ok(contents) => Some(contents),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(format!(
                "failed to read provider usage script {}: {err}",
                script_path.display()
            ));
        }
    };
    let has_existing_script = initial_contents.is_some();
    Ok(ProviderUsageEditorState {
        provider_id: provider_id.to_string(),
        provider_name: provider.name.clone(),
        script_path,
        initial_contents: initial_contents.unwrap_or_default(),
        has_existing_script,
    })
}

pub(crate) async fn save_provider_usage_script(
    config: &Config,
    provider_id: &str,
    script: String,
) -> Result<PathBuf, String> {
    let script_path = provider_usage_script_path(config, provider_id)?;
    if let Some(parent) = script_path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "failed to create provider usage script directory {}: {err}",
                parent.display()
            )
        })?;
    }
    let write_path = script_path.clone();
    task::spawn_blocking(move || write_atomically(&write_path, &script))
        .await
        .map_err(|err| format!("failed to save provider usage script: {err}"))?
        .map_err(|err| format!("failed to save provider usage script: {err}"))?;
    Ok(script_path)
}

pub(crate) async fn delete_provider_usage_script(
    config: &Config,
    provider_id: &str,
) -> Result<PathBuf, String> {
    let script_path = provider_usage_script_path(config, provider_id)?;
    if !script_path.exists() {
        return Err(format!(
            "usage script does not exist at {}",
            script_path.display()
        ));
    }
    let delete_path = script_path.clone();
    task::spawn_blocking(move || std::fs::remove_file(&delete_path))
        .await
        .map_err(|err| format!("failed to delete provider usage script: {err}"))?
        .map_err(|err| format!("failed to delete provider usage script: {err}"))?;
    Ok(script_path)
}

pub(crate) async fn fetch_provider_usage_snapshot(
    config: Config,
    auth: Option<CodexAuth>,
) -> Option<ProviderUsageRefreshResult> {
    if let Some(path) = active_provider_usage_script_path(&config) {
        return fetch_scripted_provider_usage_snapshot(&config, &path, auth.as_ref()).await;
    }

    if crate::provider_usage_compat::is_legacy_su8_provider(&config.model_provider_id) {
        return crate::provider_usage_compat::fetch_legacy_su8_provider_usage_snapshot(
            config.model_provider.clone(),
            auth,
        )
        .await;
    }

    None
}

pub(crate) fn format_usage_amount(value: f64, unit: Option<&str>) -> String {
    match unit.filter(|unit| !unit.trim().is_empty()) {
        Some(unit) => format!("{value:.2} {unit}"),
        None => format!("{value:.2}"),
    }
}

pub(crate) fn format_provider_usage_plan(plan: &ProviderUsagePlan) -> Option<String> {
    let mut parts = Vec::new();

    if let Some(plan_name) = plan
        .plan_name
        .as_ref()
        .filter(|name| !name.trim().is_empty())
    {
        parts.push(plan_name.clone());
    }
    if let Some(remaining) = plan.remaining {
        parts.push(format!(
            "remaining {}",
            format_usage_amount(remaining, plan.unit.as_deref())
        ));
    }
    if let Some(used) = plan.used {
        parts.push(format!(
            "used {}",
            format_usage_amount(used, plan.unit.as_deref())
        ));
    }
    if let Some(total) = plan.total {
        parts.push(format!(
            "total {}",
            format_usage_amount(total, plan.unit.as_deref())
        ));
    }
    if let Some(extra) = plan.extra.as_ref().filter(|extra| !extra.trim().is_empty()) {
        parts.push(extra.clone());
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" | "))
    }
}

fn active_provider_usage_script_path(config: &Config) -> Option<PathBuf> {
    if let Ok(path) = provider_usage_script_path(config, &config.model_provider_id)
        && path.is_file()
    {
        return Some(path);
    }
    None
}

fn trusted_project_root(config: &Config) -> Option<PathBuf> {
    if !config.active_project.is_trusted() {
        return None;
    }
    let cwd = config.cwd.as_path();
    let cwd_project_root =
        resolve_root_git_project_for_trust(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let project_root = config
        .config_layer_stack
        .get_layers(ConfigLayerStackOrdering::LowestPrecedenceFirst, true)
        .iter()
        .find_map(|layer| match &layer.name {
            ConfigLayerSource::Project { dot_codex_folder } => {
                dot_codex_folder.as_path().parent().map(Path::to_path_buf)
            }
            _ => None,
        });
    if let Some(project_root) = project_root
        && (cwd.starts_with(&project_root) || cwd_project_root == project_root)
    {
        return Some(project_root);
    }
    Some(cwd_project_root)
}

fn provider_usage_script_path(config: &Config, provider_id: &str) -> Result<PathBuf, String> {
    if !is_safe_provider_id(provider_id) {
        return Err(format!(
            "Provider ID `{provider_id}` cannot be used for a project usage script"
        ));
    }

    let project_root = trusted_project_root(config)
        .ok_or_else(|| "Usage scripts can only be edited inside a trusted project.".to_string())?;
    Ok(project_root
        .join(".codex")
        .join("providers")
        .join(provider_id)
        .join(PROVIDER_USAGE_SCRIPT))
}

fn is_safe_provider_id(provider_id: &str) -> bool {
    !provider_id.trim().is_empty()
        && provider_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

async fn fetch_scripted_provider_usage_snapshot(
    config: &Config,
    script_path: &Path,
    auth: Option<&CodexAuth>,
) -> Option<ProviderUsageRefreshResult> {
    let request = match run_usage_script_request(config, script_path, auth).await {
        Ok(request) => request,
        Err(err) => return Some(ProviderUsageRefreshResult::Failed(err)),
    };
    let request = apply_request_placeholders(
        request,
        &script_placeholders(&config.model_provider_id, &config.model_provider, auth),
    );
    if let Some(message) = duplicate_provider_base_path_message(
        config.model_provider.base_url.as_deref(),
        &request.url,
    ) {
        return Some(ProviderUsageRefreshResult::Failed(message));
    }
    let method = match Method::from_bytes(request.method.as_deref().unwrap_or("GET").as_bytes()) {
        Ok(method) => method,
        Err(err) => {
            return Some(ProviderUsageRefreshResult::Failed(format!(
                "invalid request method: {err}"
            )));
        }
    };
    let client = build_reqwest_client();
    let mut builder = client.request(method, request.url);

    if let Some(headers) = request.headers.as_ref() {
        let mut header_map =
            provider_header_map(&config.model_provider, &|name| std::env::var(name));
        for (name, value) in headers {
            let name = match HeaderName::try_from(name.as_str()) {
                Ok(name) => name,
                Err(err) => {
                    return Some(ProviderUsageRefreshResult::Failed(format!(
                        "invalid request header `{name}`: {err}"
                    )));
                }
            };
            let value = match HeaderValue::try_from(value.as_str()) {
                Ok(value) => value,
                Err(err) => {
                    return Some(ProviderUsageRefreshResult::Failed(format!(
                        "invalid request header value for `{name}`: {err}"
                    )));
                }
            };
            header_map.insert(name, value);
        }
        if !header_map.is_empty() {
            builder = builder.headers(header_map);
        }
    }

    if let Some(body_text) = request.body_text {
        builder = builder.body(body_text);
    } else if let Some(body_json) = request.body_json {
        builder = builder.json(&body_json);
    }

    let response = match builder.send().await {
        Ok(response) => response,
        Err(err) => {
            return Some(ProviderUsageRefreshResult::Failed(format!(
                "request failed: {err}"
            )));
        }
    };

    let status = response.status();
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (name.to_string(), value.to_string()))
        })
        .collect::<HashMap<_, _>>();
    let body_text = match response.text().await {
        Ok(body_text) => body_text,
        Err(err) => {
            return Some(ProviderUsageRefreshResult::Failed(format!(
                "failed to read response body: {err}"
            )));
        }
    };
    let body_json = serde_json::from_str::<JsonValue>(&body_text).ok();
    if !status.is_success() {
        let body_hint = truncate_usage_error_message(body_text.trim());
        let suffix = if body_hint.is_empty() {
            String::new()
        } else {
            format!(": {body_hint}")
        };
        return Some(ProviderUsageRefreshResult::Failed(format!(
            "request returned HTTP {}{}",
            status.as_u16(),
            suffix
        )));
    }
    let extractor_input = body_json
        .clone()
        .unwrap_or_else(|| JsonValue::String(body_text.clone()));
    let output = match run_usage_script_response(
        config,
        script_path,
        extractor_input.clone(),
        ScriptResponseContext {
            status: status.as_u16(),
            ok: status.is_success(),
            headers,
            body_text,
            body_json,
        },
    )
    .await
    {
        Ok(output) => output,
        Err(err) => return Some(ProviderUsageRefreshResult::Failed(err)),
    };

    normalize_script_output(output)
}

async fn run_usage_script_request(
    config: &Config,
    script_path: &Path,
    auth: Option<&CodexAuth>,
) -> Result<ScriptRequestPlan, String> {
    let payload = serde_json::to_value(ScriptRequestContext {
        provider: ScriptProviderContext::from_provider(
            &config.model_provider_id,
            &config.model_provider,
        ),
        auth: auth.map(ScriptAuthContext::from_auth),
    })
    .map_err(|err| format!("failed to serialize provider usage script request payload: {err}"))?;

    run_usage_script(config, script_path, "request", payload).await
}

async fn run_usage_script_response(
    config: &Config,
    script_path: &Path,
    extractor_input: JsonValue,
    response_context: ScriptResponseContext,
) -> Result<JsonValue, String> {
    let payload = serde_json::to_value(ScriptRunnerResponsePayload {
        extractor_input,
        response_context,
    })
    .map_err(|err| format!("failed to serialize provider usage script response payload: {err}"))?;

    run_usage_script(config, script_path, "response", payload).await
}

async fn run_usage_script<T>(
    config: &Config,
    script_path: &Path,
    mode: &str,
    payload: JsonValue,
) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let node_path = resolve_node_path(config).ok_or_else(|| {
        "Node runtime not found; install Node or set CODEX_JS_REPL_NODE_PATH".to_string()
    })?;
    let mut command = Command::new(node_path);
    command.kill_on_drop(true);
    command.arg("-e").arg(USAGE_SCRIPT_RUNNER);
    command.env("CODEX_PROVIDER_USAGE_SCRIPT", script_path);
    command.env("CODEX_PROVIDER_USAGE_MODE", mode);
    command.env(
        "CODEX_PROVIDER_USAGE_PAYLOAD",
        serde_json::to_string(&payload)
            .map_err(|err| format!("failed to encode provider usage script payload: {err}"))?,
    );

    let output = tokio::time::timeout(USAGE_SCRIPT_TIMEOUT, command.output())
        .await
        .map_err(|_| "provider usage script timed out".to_string())?
        .map_err(|err| format!("failed to run provider usage script: {err}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("provider usage script exited with {}", output.status)
        } else {
            format!("provider usage script failed: {stderr}")
        });
    }

    serde_json::from_slice::<T>(&output.stdout)
        .map_err(|err| format!("failed to parse provider usage script output: {err}"))
}

fn resolve_node_path(config: &Config) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("CODEX_JS_REPL_NODE_PATH") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    if let Some(path) = config.js_repl_node_path.as_ref()
        && path.exists()
    {
        return Some(path.clone());
    }

    find_runtime_in_path(&["node", "nodejs"])
}

fn find_runtime_in_path(candidates: &[&str]) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let executable_extensions = executable_extensions();

    std::env::split_paths(&path).find_map(|dir| {
        candidates
            .iter()
            .find_map(|candidate| find_executable_in_dir(&dir, candidate, &executable_extensions))
    })
}

fn executable_extensions() -> Vec<String> {
    #[cfg(windows)]
    {
        let path_ext = std::env::var("PATHEXT")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".to_string());
        path_ext
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    }

    #[cfg(not(windows))]
    {
        vec![String::new()]
    }
}

fn find_executable_in_dir(
    dir: &Path,
    candidate: &str,
    executable_extensions: &[String],
) -> Option<PathBuf> {
    let candidate_path = Path::new(candidate);
    if candidate_path.extension().is_some() {
        let path = dir.join(candidate);
        return is_executable_file(&path).then_some(path);
    }

    executable_extensions.iter().find_map(|extension| {
        let file_name = if extension.is_empty() {
            candidate.to_string()
        } else {
            format!("{candidate}{extension}")
        };
        let path = dir.join(file_name);
        is_executable_file(&path).then_some(path)
    })
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn script_placeholders(
    provider_id: &str,
    provider: &ModelProviderInfo,
    auth: Option<&CodexAuth>,
) -> Vec<(&'static str, String)> {
    vec![
        (
            "{{baseUrl}}",
            provider
                .base_url
                .clone()
                .unwrap_or_default()
                .trim_end_matches('/')
                .to_string(),
        ),
        (
            "{{apiKey}}",
            provider
                .api_key()
                .ok()
                .flatten()
                .unwrap_or_default()
                .trim()
                .to_string(),
        ),
        ("{{providerId}}", provider_id.to_string()),
        ("{{providerName}}", provider.name.clone()),
        (
            "{{bearerToken}}",
            auth.and_then(|auth| auth.get_token().ok())
                .unwrap_or_default(),
        ),
        (
            "{{accessToken}}",
            auth.and_then(|auth| auth.get_token().ok())
                .unwrap_or_default(),
        ),
        (
            "{{accountId}}",
            auth.and_then(CodexAuth::get_account_id).unwrap_or_default(),
        ),
        (
            "{{userId}}",
            auth.and_then(CodexAuth::get_account_id).unwrap_or_default(),
        ),
    ]
}

fn apply_request_placeholders(
    mut plan: ScriptRequestPlan,
    placeholders: &[(&str, String)],
) -> ScriptRequestPlan {
    plan.url = replace_placeholders(&plan.url, placeholders);
    if let Some(headers) = plan.headers.as_mut() {
        for value in headers.values_mut() {
            *value = replace_placeholders(value, placeholders);
        }
    }
    if let Some(body_text) = plan.body_text.as_mut() {
        *body_text = replace_placeholders(body_text, placeholders);
    }
    if let Some(body_json) = plan.body_json.as_mut() {
        replace_json_placeholders(body_json, placeholders);
    }
    plan
}

fn duplicate_provider_base_path_message(
    provider_base_url: Option<&str>,
    request_url: &str,
) -> Option<String> {
    let provider_url = Url::parse(provider_base_url?).ok()?;
    let request_url = Url::parse(request_url).ok()?;

    if provider_url.scheme() != request_url.scheme()
        || provider_url.host_str() != request_url.host_str()
        || provider_url.port_or_known_default() != request_url.port_or_known_default()
    {
        return None;
    }

    let provider_segments = provider_url
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if provider_segments.is_empty() {
        return None;
    }

    let request_segments = request_url
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let provider_segment_count = provider_segments.len();
    if request_segments.len() < provider_segment_count * 2
        || request_segments[..provider_segment_count] != provider_segments
        || request_segments[provider_segment_count..provider_segment_count * 2] != provider_segments
    {
        return None;
    }

    let provider_path = format!("/{}", provider_segments.join("/"));
    let suffix = request_segments[provider_segment_count * 2..].join("/");
    let suggestion = if suffix.is_empty() {
        "{{baseUrl}}".to_string()
    } else {
        format!("{{{{baseUrl}}}}/{suffix}")
    };

    Some(format!(
        "request.url duplicates provider base path; provider `base_url` already ends with `{provider_path}`. Use `{suggestion}` instead."
    ))
}

fn replace_placeholders(value: &str, placeholders: &[(&str, String)]) -> String {
    let mut rendered = value.to_string();
    for (placeholder, replacement) in placeholders {
        rendered = rendered.replace(placeholder, replacement);
    }
    rendered
}

fn replace_json_placeholders(value: &mut JsonValue, placeholders: &[(&str, String)]) {
    match value {
        JsonValue::String(string) => {
            *string = replace_placeholders(string, placeholders);
        }
        JsonValue::Array(array) => {
            for item in array {
                replace_json_placeholders(item, placeholders);
            }
        }
        JsonValue::Object(object) => {
            for item in object.values_mut() {
                replace_json_placeholders(item, placeholders);
            }
        }
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Number(_) => {}
    }
}

fn normalize_script_output(output: JsonValue) -> Option<ProviderUsageRefreshResult> {
    if output.is_null() {
        return Some(ProviderUsageRefreshResult::Skipped);
    }

    if let Some(error_message) = parse_script_error_message(&output) {
        return Some(ProviderUsageRefreshResult::Failed(error_message));
    }

    let rows = match serde_json::from_value::<Vec<ScriptedUsageRow>>(output) {
        Ok(rows) => rows,
        Err(err) => {
            return Some(ProviderUsageRefreshResult::Failed(format!(
                "extractor returned an invalid payload: {err}"
            )));
        }
    };
    let plans = rows
        .into_iter()
        .filter(|row| row.is_valid != Some(false))
        .filter_map(|row| {
            let plan = ProviderUsagePlan {
                plan_name: row.plan_name,
                remaining: row.remaining,
                used: row.used,
                total: row.total,
                unit: row.unit,
                extra: row.extra,
            };
            plan.has_content().then_some(plan)
        })
        .collect::<Vec<_>>();
    if plans.is_empty() {
        return Some(ProviderUsageRefreshResult::Failed(
            "extractor returned no usable usage rows".to_string(),
        ));
    }

    Some(ProviderUsageRefreshResult::Updated(ProviderUsageSnapshot {
        plans,
        error_message: None,
    }))
}

fn parse_script_error_message(output: &JsonValue) -> Option<String> {
    let object = output.as_object()?;
    if object.get("isValid").and_then(JsonValue::as_bool) != Some(false) {
        return None;
    }

    let invalid_message = object
        .get("invalidMessage")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(ToString::to_string);
    let invalid_code = object
        .get("invalidCode")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(ToString::to_string);

    Some(match (invalid_message, invalid_code) {
        (Some(message), Some(code)) => format!("{message} ({code})"),
        (Some(message), None) => message,
        (None, Some(code)) => format!("remote usage is invalid ({code})"),
        (None, None) => "remote usage is invalid".to_string(),
    })
}

fn truncate_usage_error_message(message: &str) -> String {
    const MAX_CHARS: usize = 2048;

    let trimmed = message.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let truncated = trimmed.chars().take(MAX_CHARS).collect::<String>();
    if trimmed.chars().count() > MAX_CHARS {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn provider_header_map<F>(provider: &ModelProviderInfo, env_lookup: &F) -> HeaderMap
where
    F: Fn(&str) -> Result<String, std::env::VarError>,
{
    let capacity = provider.http_headers.as_ref().map_or(0, HashMap::len)
        + provider.env_http_headers.as_ref().map_or(0, HashMap::len);
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
            let Ok(value) = env_lookup(env_var) else {
                continue;
            };
            if value.trim().is_empty() {
                continue;
            }
            if let (Ok(name), Ok(value)) =
                (HeaderName::try_from(name), HeaderValue::try_from(value))
            {
                headers.insert(name, value);
            }
        }
    }

    headers
}

fn aggregate_plans(plans: &[ProviderUsagePlan]) -> Option<ProviderUsagePlan> {
    let units: BTreeSet<String> = plans
        .iter()
        .filter_map(|plan| {
            plan.unit
                .as_deref()
                .map(str::trim)
                .filter(|unit| !unit.is_empty())
                .map(ToString::to_string)
        })
        .collect();
    if units.len() > 1 {
        return None;
    }

    let plan = ProviderUsagePlan {
        plan_name: Some(format!("{} plans", plans.len())),
        remaining: sum_optional(plans.iter().map(|plan| plan.remaining)),
        used: sum_optional(plans.iter().map(|plan| plan.used)),
        total: sum_optional(plans.iter().map(|plan| plan.total)),
        unit: units.into_iter().next(),
        extra: None,
    };

    plan.has_content().then_some(plan)
}

fn sum_optional<I>(values: I) -> Option<f64>
where
    I: Iterator<Item = Option<f64>>,
{
    let mut total = 0.0;
    let mut found = false;
    for value in values.flatten() {
        found = true;
        total += value;
    }
    found.then_some(total)
}

#[cfg(test)]
mod tests;
