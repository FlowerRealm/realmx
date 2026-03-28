use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use strum_macros::Display;
use strum_macros::EnumIter;
use ts_rs::TS;

use crate::openai_models::ReasoningEffort;

/// A summary of the reasoning performed by the model. This can be useful for
/// debugging and understanding the model's reasoning process.
/// See https://platform.openai.com/docs/guides/reasoning?api-mode=responses#reasoning-summaries
#[derive(
    Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Display, JsonSchema, TS,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ReasoningSummary {
    #[default]
    Auto,
    Concise,
    Detailed,
    /// Option to disable reasoning summaries.
    None,
}

/// Controls output length/detail on GPT-5 models via the Responses API.
/// Serialized with lowercase values to match the OpenAI API.
#[derive(
    Hash,
    Debug,
    Serialize,
    Deserialize,
    Default,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Display,
    JsonSchema,
    TS,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Verbosity {
    Low,
    #[default]
    Medium,
    High,
}

#[derive(
    Deserialize, Debug, Clone, Copy, PartialEq, Default, Serialize, Display, JsonSchema, TS,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum SandboxMode {
    #[serde(rename = "read-only")]
    #[default]
    ReadOnly,

    #[serde(rename = "workspace-write")]
    WorkspaceWrite,

    #[serde(rename = "danger-full-access")]
    DangerFullAccess,
}

#[derive(
    Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Display, JsonSchema, TS,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
/// Configures who approval requests are routed to for review. Examples
/// include sandbox escapes, blocked network access, MCP approval prompts, and
/// ARC escalations. Defaults to `user`. `guardian_subagent` uses a carefully
/// prompted subagent to gather relevant context and apply a risk-based
/// decision framework before approving or denying the request.
pub enum ApprovalsReviewer {
    #[default]
    User,
    GuardianSubagent,
}

#[derive(
    Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Display, JsonSchema, TS,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum WindowsSandboxLevel {
    #[default]
    Disabled,
    RestrictedToken,
    Elevated,
}

#[derive(
    Debug,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Display,
    JsonSchema,
    TS,
    PartialOrd,
    Ord,
    EnumIter,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum Personality {
    None,
    Friendly,
    Pragmatic,
}

#[derive(
    Debug,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Display,
    JsonSchema,
    TS,
    Default,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum WebSearchMode {
    Disabled,
    #[default]
    Cached,
    Live,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Display, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum WebSearchContextSize {
    Low,
    Medium,
    High,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
#[schemars(deny_unknown_fields)]
pub struct WebSearchLocation {
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub timezone: Option<String>,
}

impl WebSearchLocation {
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            country: other.country.clone().or_else(|| self.country.clone()),
            region: other.region.clone().or_else(|| self.region.clone()),
            city: other.city.clone().or_else(|| self.city.clone()),
            timezone: other.timezone.clone().or_else(|| self.timezone.clone()),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
#[schemars(deny_unknown_fields)]
pub struct WebSearchToolConfig {
    pub context_size: Option<WebSearchContextSize>,
    pub allowed_domains: Option<Vec<String>>,
    pub location: Option<WebSearchLocation>,
}

impl WebSearchToolConfig {
    pub fn merge(&self, other: &Self) -> Self {
        Self {
            context_size: other.context_size.or(self.context_size),
            allowed_domains: other
                .allowed_domains
                .clone()
                .or_else(|| self.allowed_domains.clone()),
            location: match (&self.location, &other.location) {
                (Some(location), Some(other_location)) => Some(location.merge(other_location)),
                (Some(location), None) => Some(location.clone()),
                (None, Some(other_location)) => Some(other_location.clone()),
                (None, None) => None,
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
#[schemars(deny_unknown_fields)]
pub struct WebSearchFilters {
    pub allowed_domains: Option<Vec<String>>,
}

#[derive(
    Debug, Serialize, Deserialize, Clone, Copy, Default, PartialEq, Eq, Display, JsonSchema, TS,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum WebSearchUserLocationType {
    #[default]
    Approximate,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
#[schemars(deny_unknown_fields)]
pub struct WebSearchUserLocation {
    #[serde(default)]
    pub r#type: WebSearchUserLocationType,
    pub country: Option<String>,
    pub region: Option<String>,
    pub city: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq, JsonSchema, TS)]
#[schemars(deny_unknown_fields)]
pub struct WebSearchConfig {
    pub filters: Option<WebSearchFilters>,
    pub user_location: Option<WebSearchUserLocation>,
    pub search_context_size: Option<WebSearchContextSize>,
}

impl From<WebSearchLocation> for WebSearchUserLocation {
    fn from(location: WebSearchLocation) -> Self {
        Self {
            r#type: WebSearchUserLocationType::Approximate,
            country: location.country,
            region: location.region,
            city: location.city,
            timezone: location.timezone,
        }
    }
}

impl From<WebSearchToolConfig> for WebSearchConfig {
    fn from(config: WebSearchToolConfig) -> Self {
        Self {
            filters: config
                .allowed_domains
                .map(|allowed_domains| WebSearchFilters {
                    allowed_domains: Some(allowed_domains),
                }),
            user_location: config.location.map(Into::into),
            search_context_size: config.context_size,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Display, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ServiceTier {
    Fast,
    Flex,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Display, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum ForcedLoginMethod {
    Chatgpt,
    Api,
}

/// Represents the trust level for a project directory.
/// This determines the approval policy and sandbox mode applied.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Display, JsonSchema, TS)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum TrustLevel {
    Trusted,
    Untrusted,
}

/// Controls whether the TUI uses the terminal's alternate screen buffer.
///
/// **Background:** The alternate screen buffer provides a cleaner fullscreen experience
/// without polluting the terminal's scrollback history. However, it conflicts with terminal
/// multiplexers like Zellij that strictly follow the xterm specification, which defines
/// that alternate screen buffers should not have scrollback.
///
/// **Zellij's behavior:** Zellij intentionally disables scrollback in alternate screen mode
/// (see https://github.com/zellij-org/zellij/pull/1032) to comply with the xterm spec. This
/// is by design and not configurable in Zellij—there is no option to enable scrollback in
/// alternate screen mode.
///
/// **Solution:** This setting provides a pragmatic workaround:
/// - `auto` (default): Automatically detect the terminal multiplexer. If running in Zellij,
///   disable alternate screen to preserve scrollback. Enable it everywhere else.
/// - `always`: Always use alternate screen mode (original behavior before this fix).
/// - `never`: Never use alternate screen mode. Runs in inline mode, preserving scrollback
///   in all multiplexers.
///
/// The CLI flag `--no-alt-screen` can override this setting at runtime.
#[derive(
    Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Display, JsonSchema, TS,
)]
#[serde(rename_all = "lowercase")]
#[strum(serialize_all = "lowercase")]
pub enum AltScreenMode {
    /// Auto-detect: disable alternate screen in Zellij, enable elsewhere.
    #[default]
    Auto,
    /// Always use alternate screen (original behavior).
    Always,
    /// Never use alternate screen (inline mode only).
    Never,
}

/// Initial collaboration mode to use when the TUI starts.
#[derive(
    Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, JsonSchema, TS, Default,
)]
#[serde(rename_all = "snake_case")]
pub enum ModeKind {
    Plan,
    #[serde(alias = "auto_plan", alias = "execute")]
    UltraWork,
    #[default]
    #[serde(alias = "code", alias = "pair_programming", alias = "custom")]
    Default,
    #[doc(hidden)]
    #[serde(skip_serializing, skip_deserializing)]
    #[schemars(skip)]
    #[ts(skip)]
    PairProgramming,
    #[doc(hidden)]
    #[serde(skip_serializing, skip_deserializing)]
    #[schemars(skip)]
    #[ts(skip)]
    Execute,
}

pub const TUI_VISIBLE_COLLABORATION_MODES: [ModeKind; 3] =
    [ModeKind::Default, ModeKind::Plan, ModeKind::UltraWork];

#[derive(
    Debug,
    Serialize,
    Deserialize,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Display,
    JsonSchema,
    TS,
    Default,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum PlanModePhase {
    #[default]
    Planning,
    Executing,
}

impl ModeKind {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Plan => "Plan",
            Self::UltraWork => "Ultra Work",
            Self::Default => "Default",
            Self::PairProgramming => "Pair Programming",
            Self::Execute => "Execute",
        }
    }

    pub const fn is_tui_visible(self) -> bool {
        matches!(self, Self::Plan | Self::UltraWork | Self::Default)
    }

    pub const fn is_plan_mode(self) -> bool {
        matches!(self, Self::Plan)
    }

    pub const fn is_ultra_work_mode(self) -> bool {
        matches!(self, Self::UltraWork | Self::Execute)
    }

    pub const fn is_ultra_work_planning_mode(self) -> bool {
        matches!(self, Self::UltraWork)
    }

    pub const fn is_ultra_work_execution_mode(self) -> bool {
        matches!(self, Self::Execute)
    }

    pub const fn allows_request_user_input(self) -> bool {
        matches!(self, Self::Plan | Self::UltraWork)
    }
}

/// Collaboration mode for a Codex session.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "lowercase", from = "CollaborationModeDe")]
pub struct CollaborationMode {
    pub mode: ModeKind,
    #[ts(optional = false)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_phase: Option<PlanModePhase>,
    pub settings: Settings,
}

#[derive(Deserialize)]
struct CollaborationModeDe {
    mode: CollaborationModeDeKind,
    plan_phase: Option<PlanModePhase>,
    settings: Settings,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CollaborationModeDeKind {
    Plan,
    #[serde(alias = "auto_plan")]
    UltraWork,
    #[serde(alias = "execute")]
    Execute,
    #[serde(alias = "code", alias = "pair_programming", alias = "custom")]
    Default,
}

impl From<CollaborationModeDe> for CollaborationMode {
    fn from(value: CollaborationModeDe) -> Self {
        match value.mode {
            CollaborationModeDeKind::Plan => Self {
                mode: ModeKind::Plan,
                plan_phase: None,
                settings: value.settings,
            },
            CollaborationModeDeKind::UltraWork => Self {
                mode: ModeKind::UltraWork,
                plan_phase: Some(value.plan_phase.unwrap_or(PlanModePhase::Planning)),
                settings: value.settings,
            },
            CollaborationModeDeKind::Execute => Self {
                mode: ModeKind::UltraWork,
                plan_phase: Some(PlanModePhase::Executing),
                settings: value.settings,
            },
            CollaborationModeDeKind::Default => Self {
                mode: ModeKind::Default,
                plan_phase: None,
                settings: value.settings,
            },
        }
    }
}

impl CollaborationMode {
    fn normalize_mode(mode: ModeKind) -> ModeKind {
        match mode {
            ModeKind::Execute => ModeKind::UltraWork,
            other => other,
        }
    }

    fn normalize_plan_phase(
        mode: ModeKind,
        plan_phase: Option<PlanModePhase>,
    ) -> Option<PlanModePhase> {
        match mode {
            ModeKind::Execute => Some(PlanModePhase::Executing),
            ModeKind::UltraWork => Some(plan_phase.unwrap_or(PlanModePhase::Planning)),
            ModeKind::Plan | ModeKind::Default | ModeKind::PairProgramming => None,
        }
    }

    /// Returns a reference to the settings.
    fn settings_ref(&self) -> &Settings {
        &self.settings
    }

    pub fn normalized(&self) -> Self {
        let mode = Self::normalize_mode(self.mode);
        let plan_phase = Self::normalize_plan_phase(self.mode, self.plan_phase);
        Self {
            mode,
            plan_phase,
            settings: self.settings.clone(),
        }
    }

    pub fn model(&self) -> &str {
        self.settings_ref().model.as_str()
    }

    pub fn reasoning_effort(&self) -> Option<ReasoningEffort> {
        self.settings_ref().reasoning_effort
    }

    pub fn plan_phase(&self) -> Option<PlanModePhase> {
        Self::normalize_plan_phase(self.mode, self.plan_phase)
    }

    pub fn is_plan_mode(&self) -> bool {
        Self::normalize_mode(self.mode).is_plan_mode()
    }

    pub fn is_ultra_work_mode(&self) -> bool {
        Self::normalize_mode(self.mode).is_ultra_work_mode()
    }

    pub fn is_plan_output_mode(&self) -> bool {
        self.is_plan_mode()
    }

    pub fn is_ultra_work_planning_mode(&self) -> bool {
        matches!(self.plan_phase(), Some(PlanModePhase::Planning))
    }

    pub fn is_plan_execution_mode(&self) -> bool {
        self.is_ultra_work_execution_mode()
    }

    pub fn is_ultra_work_execution_mode(&self) -> bool {
        matches!(self.plan_phase(), Some(PlanModePhase::Executing))
    }

    /// Updates the collaboration mode with new model and/or effort values.
    ///
    /// - `model`: `Some(s)` to update the model, `None` to keep the current model
    /// - `effort`: `Some(Some(e))` to set effort to `e`, `Some(None)` to clear effort, `None` to keep current effort
    /// - `developer_instructions`: `Some(Some(s))` to set instructions, `Some(None)` to clear them, `None` to keep current
    ///
    /// Returns a new `CollaborationMode` with updated values, preserving the mode.
    pub fn with_updates(
        &self,
        model: Option<String>,
        effort: Option<Option<ReasoningEffort>>,
        developer_instructions: Option<Option<String>>,
    ) -> Self {
        let settings = self.settings_ref();
        let updated_settings = Settings {
            model: model.unwrap_or_else(|| settings.model.clone()),
            reasoning_effort: effort.unwrap_or(settings.reasoning_effort),
            developer_instructions: developer_instructions
                .unwrap_or_else(|| settings.developer_instructions.clone()),
        };

        let normalized = self.normalized();
        CollaborationMode {
            mode: normalized.mode,
            plan_phase: normalized.plan_phase,
            settings: updated_settings,
        }
    }

    /// Applies a mask to this collaboration mode, returning a new collaboration mode
    /// with the mask values applied. Fields in the mask that are `Some` will override
    /// the corresponding fields, while `None` values will preserve the original values.
    ///
    /// The `name` field in the mask is ignored as it's metadata for the mask itself.
    pub fn apply_mask(&self, mask: &CollaborationModeMask) -> Self {
        let settings = self.settings_ref();
        let mode = mask.mode.unwrap_or(self.mode);
        CollaborationMode {
            mode: Self::normalize_mode(mode),
            plan_phase: Self::normalize_plan_phase(mode, mask.plan_phase.or(self.plan_phase())),
            settings: Settings {
                model: mask.model.clone().unwrap_or_else(|| settings.model.clone()),
                reasoning_effort: mask.reasoning_effort.unwrap_or(settings.reasoning_effort),
                developer_instructions: mask
                    .developer_instructions
                    .clone()
                    .unwrap_or_else(|| settings.developer_instructions.clone()),
            },
        }
    }
}

/// Settings for a collaboration mode.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct Settings {
    pub model: String,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub developer_instructions: Option<String>,
}

/// A mask for collaboration mode settings, allowing partial updates.
/// All fields except `name` are optional, enabling selective updates.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema, TS)]
pub struct CollaborationModeMask {
    pub name: String,
    pub mode: Option<ModeKind>,
    pub plan_phase: Option<PlanModePhase>,
    pub model: Option<String>,
    pub reasoning_effort: Option<Option<ReasoningEffort>>,
    pub developer_instructions: Option<Option<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn apply_mask_can_clear_optional_fields() {
        let mode = CollaborationMode {
            mode: ModeKind::Default,
            plan_phase: None,
            settings: Settings {
                model: "gpt-5.2-codex".to_string(),
                reasoning_effort: Some(ReasoningEffort::High),
                developer_instructions: Some("stay focused".to_string()),
            },
        };
        let mask = CollaborationModeMask {
            name: "Clear".to_string(),
            mode: None,
            plan_phase: None,
            model: None,
            reasoning_effort: Some(None),
            developer_instructions: Some(None),
        };

        let expected = CollaborationMode {
            mode: ModeKind::Default,
            plan_phase: None,
            settings: Settings {
                model: "gpt-5.2-codex".to_string(),
                reasoning_effort: None,
                developer_instructions: None,
            },
        };
        assert_eq!(expected, mode.apply_mask(&mask));
    }

    #[test]
    fn legacy_execute_mode_normalizes_to_ultra_work_executing() {
        let mode = CollaborationMode {
            mode: ModeKind::Execute,
            plan_phase: None,
            settings: Settings {
                model: "gpt-5.2-codex".to_string(),
                reasoning_effort: None,
                developer_instructions: None,
            },
        };

        let expected = CollaborationMode {
            mode: ModeKind::UltraWork,
            plan_phase: Some(PlanModePhase::Executing),
            settings: Settings {
                model: "gpt-5.2-codex".to_string(),
                reasoning_effort: None,
                developer_instructions: None,
            },
        };
        assert_eq!(expected, mode.normalized());
    }

    #[test]
    fn mode_kind_deserializes_alias_values_to_default() {
        for alias in ["code", "pair_programming", "custom"] {
            let json = format!("\"{alias}\"");
            let mode: ModeKind = serde_json::from_str(&json).expect("deserialize mode");
            assert_eq!(ModeKind::Default, mode);
        }
    }

    #[test]
    fn mode_kind_deserializes_ultra_work_aliases_for_backward_compatibility() {
        for alias in ["auto_plan", "execute"] {
            let json = format!("\"{alias}\"");
            let mode: ModeKind = serde_json::from_str(&json).expect("deserialize mode");
            assert_eq!(ModeKind::UltraWork, mode);
        }
    }

    #[test]
    fn collaboration_mode_deserializes_legacy_execute_payload_as_ultra_work_execution() {
        let mode: CollaborationMode = serde_json::from_str(
            r#"{
                "mode":"execute",
                "plan_phase":null,
                "settings":{
                    "model":"gpt-5.2-codex",
                    "reasoning_effort":null,
                    "developer_instructions":null
                }
            }"#,
        )
        .expect("deserialize collaboration mode");

        assert_eq!(ModeKind::UltraWork, mode.mode);
        assert_eq!(Some(PlanModePhase::Executing), mode.plan_phase());
    }

    #[test]
    fn tui_visible_collaboration_modes_match_mode_kind_visibility() {
        let expected = [ModeKind::Default, ModeKind::Plan, ModeKind::UltraWork];
        assert_eq!(expected, TUI_VISIBLE_COLLABORATION_MODES);

        for mode in TUI_VISIBLE_COLLABORATION_MODES {
            assert!(mode.is_tui_visible());
        }

        assert!(!ModeKind::PairProgramming.is_tui_visible());
        assert!(!ModeKind::Execute.is_tui_visible());
    }

    #[test]
    fn web_search_location_merge_prefers_overlay_values() {
        let base = WebSearchLocation {
            country: Some("US".to_string()),
            region: Some("CA".to_string()),
            city: None,
            timezone: Some("America/Los_Angeles".to_string()),
        };
        let overlay = WebSearchLocation {
            country: None,
            region: Some("WA".to_string()),
            city: Some("Seattle".to_string()),
            timezone: None,
        };

        let expected = WebSearchLocation {
            country: Some("US".to_string()),
            region: Some("WA".to_string()),
            city: Some("Seattle".to_string()),
            timezone: Some("America/Los_Angeles".to_string()),
        };

        assert_eq!(expected, base.merge(&overlay));
    }

    #[test]
    fn web_search_tool_config_merge_prefers_overlay_values() {
        let base = WebSearchToolConfig {
            context_size: Some(WebSearchContextSize::Low),
            allowed_domains: Some(vec!["openai.com".to_string()]),
            location: Some(WebSearchLocation {
                country: Some("US".to_string()),
                region: Some("CA".to_string()),
                city: None,
                timezone: Some("America/Los_Angeles".to_string()),
            }),
        };
        let overlay = WebSearchToolConfig {
            context_size: Some(WebSearchContextSize::High),
            allowed_domains: None,
            location: Some(WebSearchLocation {
                country: None,
                region: Some("WA".to_string()),
                city: Some("Seattle".to_string()),
                timezone: None,
            }),
        };

        let expected = WebSearchToolConfig {
            context_size: Some(WebSearchContextSize::High),
            allowed_domains: Some(vec!["openai.com".to_string()]),
            location: Some(WebSearchLocation {
                country: Some("US".to_string()),
                region: Some("WA".to_string()),
                city: Some("Seattle".to_string()),
                timezone: Some("America/Los_Angeles".to_string()),
            }),
        };

        assert_eq!(expected, base.merge(&overlay));
    }
}
