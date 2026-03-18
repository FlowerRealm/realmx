#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsSectionMatcher {
    ExactKey(&'static str),
    KeyPrefix(&'static str),
    PathPrefix(&'static str),
}

impl SettingsSectionMatcher {
    pub fn matches(self, key_path: &str) -> bool {
        match self {
            Self::ExactKey(key) => key_path == key,
            Self::KeyPrefix(prefix) => key_path.starts_with(prefix),
            Self::PathPrefix(prefix) => {
                key_path == prefix
                    || key_path
                        .strip_prefix(prefix)
                        .is_some_and(|suffix| suffix.starts_with('.'))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsSectionLabel {
    Keep,
    StripPrefix(&'static str),
    Rename(&'static str),
}

impl SettingsSectionLabel {
    fn format(self, key_path: &str) -> String {
        match self {
            Self::Keep => key_path.to_string(),
            Self::StripPrefix(prefix) => key_path
                .strip_prefix(prefix)
                .unwrap_or(key_path)
                .to_string(),
            Self::Rename(label) => label.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsSectionMember {
    pub matcher: SettingsSectionMatcher,
    pub label: SettingsSectionLabel,
}

impl SettingsSectionMember {
    pub fn matches(self, key_path: &str) -> bool {
        self.matcher.matches(key_path)
    }

    pub fn label_for(self, key_path: &str) -> String {
        self.label.format(key_path)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsSectionDescriptor {
    pub id: &'static str,
    pub section_editor_key: Option<&'static str>,
    pub members: &'static [SettingsSectionMember],
}

impl SettingsSectionDescriptor {
    pub fn matches_key(self, key_path: &str) -> bool {
        self.members
            .iter()
            .copied()
            .any(|member| member.matches(key_path))
    }

    pub fn label_for_key(self, key_path: &str) -> Option<String> {
        self.members
            .iter()
            .copied()
            .find(|member| member.matches(key_path))
            .map(|member| member.label_for(key_path))
    }
}

const fn exact(key: &'static str, label: SettingsSectionLabel) -> SettingsSectionMember {
    SettingsSectionMember {
        matcher: SettingsSectionMatcher::ExactKey(key),
        label,
    }
}

const fn key_prefix(prefix: &'static str, label: SettingsSectionLabel) -> SettingsSectionMember {
    SettingsSectionMember {
        matcher: SettingsSectionMatcher::KeyPrefix(prefix),
        label,
    }
}

const fn path_prefix(prefix: &'static str, label: SettingsSectionLabel) -> SettingsSectionMember {
    SettingsSectionMember {
        matcher: SettingsSectionMatcher::PathPrefix(prefix),
        label,
    }
}

const MODEL_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    exact("model", SettingsSectionLabel::Keep),
    key_prefix("model_", SettingsSectionLabel::StripPrefix("model_")),
    exact("plan_mode_reasoning_effort", SettingsSectionLabel::Keep),
    exact("review_model", SettingsSectionLabel::Keep),
    exact("service_tier", SettingsSectionLabel::Keep),
    exact("personality", SettingsSectionLabel::Keep),
    exact("oss_provider", SettingsSectionLabel::Keep),
    exact("chatgpt_base_url", SettingsSectionLabel::Keep),
    exact("openai_base_url", SettingsSectionLabel::Keep),
];

const AUTH_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    exact(
        "cli_auth_credentials_store",
        SettingsSectionLabel::StripPrefix("cli_auth_"),
    ),
    exact(
        "forced_chatgpt_workspace_id",
        SettingsSectionLabel::StripPrefix("forced_"),
    ),
    exact(
        "forced_login_method",
        SettingsSectionLabel::StripPrefix("forced_"),
    ),
];

const PERMISSIONS_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    exact("approval_policy", SettingsSectionLabel::Keep),
    exact("approvals_reviewer", SettingsSectionLabel::Keep),
    exact("allow_login_shell", SettingsSectionLabel::Keep),
    exact("default_permissions", SettingsSectionLabel::Keep),
    exact("sandbox_mode", SettingsSectionLabel::Keep),
    path_prefix(
        "sandbox_workspace_write",
        SettingsSectionLabel::StripPrefix("sandbox_workspace_write."),
    ),
    path_prefix(
        "shell_environment_policy",
        SettingsSectionLabel::StripPrefix("shell_environment_policy."),
    ),
    path_prefix(
        "permissions",
        SettingsSectionLabel::StripPrefix("permissions."),
    ),
];

const TOOLS_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    exact(
        "background_terminal_max_timeout",
        SettingsSectionLabel::Keep,
    ),
    exact("js_repl_node_module_dirs", SettingsSectionLabel::Keep),
    exact("js_repl_node_path", SettingsSectionLabel::Keep),
    exact("tool_output_token_limit", SettingsSectionLabel::Keep),
    exact(
        "web_search",
        SettingsSectionLabel::Rename("web_search_mode"),
    ),
    exact("tools_view_image", SettingsSectionLabel::Keep),
    exact("zsh_path", SettingsSectionLabel::Keep),
    path_prefix("tools", SettingsSectionLabel::StripPrefix("tools.")),
];

const MCP_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    exact("mcp_servers", SettingsSectionLabel::Rename("servers")),
    path_prefix(
        "mcp_servers",
        SettingsSectionLabel::StripPrefix("mcp_servers."),
    ),
    key_prefix(
        "mcp_oauth_",
        SettingsSectionLabel::StripPrefix("mcp_oauth_"),
    ),
];

const PROJECT_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    key_prefix(
        "project_doc_",
        SettingsSectionLabel::StripPrefix("project_doc_"),
    ),
    exact(
        "project_root_markers",
        SettingsSectionLabel::StripPrefix("project_"),
    ),
    path_prefix("projects", SettingsSectionLabel::StripPrefix("projects.")),
    exact("commit_attribution", SettingsSectionLabel::Keep),
];

const FEATURES_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    path_prefix("features", SettingsSectionLabel::StripPrefix("features.")),
    exact(
        "suppress_unstable_features_warning",
        SettingsSectionLabel::Keep,
    ),
];

const PROMPTING_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    exact("compact_prompt", SettingsSectionLabel::Keep),
    exact("developer_instructions", SettingsSectionLabel::Keep),
    exact(
        "experimental_compact_prompt_file",
        SettingsSectionLabel::StripPrefix("experimental_"),
    ),
    exact("instructions", SettingsSectionLabel::Keep),
];

const REASONING_OUTPUT_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    exact("hide_agent_reasoning", SettingsSectionLabel::Keep),
    exact("show_raw_agent_reasoning", SettingsSectionLabel::Keep),
];

const STORAGE_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    exact("log_dir", SettingsSectionLabel::Keep),
    exact("sqlite_home", SettingsSectionLabel::Keep),
    path_prefix(
        "ghost_snapshot",
        SettingsSectionLabel::StripPrefix("ghost_snapshot."),
    ),
    path_prefix("history", SettingsSectionLabel::StripPrefix("history.")),
];

const TELEMETRY_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    path_prefix("analytics", SettingsSectionLabel::Keep),
    path_prefix("feedback", SettingsSectionLabel::Keep),
    path_prefix("otel", SettingsSectionLabel::Keep),
];

const NOTIFICATIONS_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    exact("check_for_update_on_startup", SettingsSectionLabel::Keep),
    path_prefix("notice", SettingsSectionLabel::StripPrefix("notice.")),
    exact("notify", SettingsSectionLabel::Rename("external_command")),
    exact(
        "tui.notification_method",
        SettingsSectionLabel::Rename("notification_method"),
    ),
    exact(
        "tui.notifications",
        SettingsSectionLabel::Rename("notifications"),
    ),
];

const EXTENSIONS_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    path_prefix("apps", SettingsSectionLabel::Keep),
    exact("plugins", SettingsSectionLabel::Keep),
    path_prefix("skills", SettingsSectionLabel::Keep),
];

const VOICE_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    path_prefix("audio", SettingsSectionLabel::StripPrefix("audio.")),
    key_prefix(
        "experimental_realtime_",
        SettingsSectionLabel::StripPrefix("experimental_"),
    ),
    path_prefix("realtime", SettingsSectionLabel::StripPrefix("realtime.")),
];

const TUI_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    exact("disable_paste_burst", SettingsSectionLabel::Keep),
    exact("file_opener", SettingsSectionLabel::Keep),
    exact("tui", SettingsSectionLabel::Keep),
    exact(
        "tui.alternate_screen",
        SettingsSectionLabel::StripPrefix("tui."),
    ),
    exact("tui.animations", SettingsSectionLabel::StripPrefix("tui.")),
    exact(
        "tui.model_availability_nux",
        SettingsSectionLabel::StripPrefix("tui."),
    ),
    exact(
        "tui.show_tooltips",
        SettingsSectionLabel::StripPrefix("tui."),
    ),
    exact("tui.status_line", SettingsSectionLabel::StripPrefix("tui.")),
    exact("tui.theme", SettingsSectionLabel::StripPrefix("tui.")),
];

const WINDOWS_SECTION_MEMBERS: &[SettingsSectionMember] = &[
    exact(
        "windows_wsl_setup_acknowledged",
        SettingsSectionLabel::Rename("wsl_setup_acknowledged"),
    ),
    path_prefix("windows", SettingsSectionLabel::StripPrefix("windows.")),
];

const SETTINGS_SECTIONS: &[SettingsSectionDescriptor] = &[
    SettingsSectionDescriptor {
        id: "auth",
        section_editor_key: None,
        members: AUTH_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "extensions",
        section_editor_key: None,
        members: EXTENSIONS_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "features",
        section_editor_key: None,
        members: FEATURES_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "mcp",
        section_editor_key: None,
        members: MCP_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "model",
        section_editor_key: None,
        members: MODEL_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "notifications",
        section_editor_key: None,
        members: NOTIFICATIONS_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "permissions",
        section_editor_key: Some("permissions"),
        members: PERMISSIONS_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "project",
        section_editor_key: None,
        members: PROJECT_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "prompting",
        section_editor_key: None,
        members: PROMPTING_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "reasoning_output",
        section_editor_key: None,
        members: REASONING_OUTPUT_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "storage",
        section_editor_key: None,
        members: STORAGE_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "telemetry",
        section_editor_key: None,
        members: TELEMETRY_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "tui",
        section_editor_key: Some("tui"),
        members: TUI_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "tools",
        section_editor_key: Some("tools"),
        members: TOOLS_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "voice",
        section_editor_key: None,
        members: VOICE_SECTION_MEMBERS,
    },
    SettingsSectionDescriptor {
        id: "windows",
        section_editor_key: Some("windows"),
        members: WINDOWS_SECTION_MEMBERS,
    },
];

pub fn settings_sections() -> &'static [SettingsSectionDescriptor] {
    SETTINGS_SECTIONS
}

pub fn settings_section(id: &str) -> Option<&'static SettingsSectionDescriptor> {
    SETTINGS_SECTIONS.iter().find(|section| section.id == id)
}

#[cfg(test)]
mod tests {
    use super::settings_section;

    #[test]
    fn model_section_matches_model_family() {
        let section = settings_section("model").expect("model section");
        assert!(section.matches_key("model"));
        assert!(section.matches_key("model_reasoning_effort"));
        assert!(section.matches_key("plan_mode_reasoning_effort"));
        assert!(section.matches_key("model_providers.openai.name"));
        assert!(section.matches_key("service_tier"));
        assert!(!section.matches_key("approval_policy"));
        assert_eq!(
            section
                .label_for_key("model_reasoning_effort")
                .expect("reasoning effort label"),
            "reasoning_effort"
        );
    }

    #[test]
    fn auth_section_groups_forced_login_controls() {
        let section = settings_section("auth").expect("auth section");
        assert!(section.matches_key("cli_auth_credentials_store"));
        assert!(section.matches_key("forced_chatgpt_workspace_id"));
        assert!(section.matches_key("forced_login_method"));
        assert!(!section.matches_key("mcp_oauth_callback_url"));
        assert_eq!(
            section
                .label_for_key("cli_auth_credentials_store")
                .expect("credentials store label"),
            "credentials_store"
        );
        assert_eq!(
            section
                .label_for_key("forced_login_method")
                .expect("login method label"),
            "login_method"
        );
    }

    #[test]
    fn notifications_section_absorbs_tui_notification_keys() {
        let section = settings_section("notifications").expect("notifications section");
        assert!(section.matches_key("notify"));
        assert!(section.matches_key("notice.hide_full_access_warning"));
        assert!(section.matches_key("tui.notification_method"));
        assert!(section.matches_key("tui.notifications"));
        assert_eq!(
            section
                .label_for_key("notify")
                .expect("external notify label"),
            "external_command"
        );
        assert_eq!(
            section
                .label_for_key("tui.notification_method")
                .expect("notification method label"),
            "notification_method"
        );
    }

    #[test]
    fn features_section_groups_feature_flags_and_warning_controls() {
        let section = settings_section("features").expect("features section");
        assert!(section.matches_key("features.js_repl"));
        assert!(section.matches_key("features.guardian_approval"));
        assert!(section.matches_key("suppress_unstable_features_warning"));
        assert!(!section.matches_key("experimental_use_unified_exec_tool"));
        assert_eq!(
            section
                .label_for_key("features.js_repl")
                .expect("feature label"),
            "js_repl"
        );
    }

    #[test]
    fn tools_section_disambiguates_flat_and_nested_keys() {
        let section = settings_section("tools").expect("tools section");
        assert_eq!(
            section.label_for_key("web_search").expect("mode label"),
            "web_search_mode"
        );
        assert_eq!(
            section
                .label_for_key("tools.web_search.location.city")
                .expect("city label"),
            "web_search.location.city"
        );
        assert_eq!(
            section
                .label_for_key("tools.view_image")
                .expect("view_image label"),
            "view_image"
        );
        assert_eq!(
            section
                .label_for_key("tools_view_image")
                .expect("legacy view_image label"),
            "tools_view_image"
        );
        assert_eq!(
            section
                .label_for_key("js_repl_node_path")
                .expect("js_repl path label"),
            "js_repl_node_path"
        );
        assert_eq!(
            section
                .label_for_key("tool_output_token_limit")
                .expect("tool output limit label"),
            "tool_output_token_limit"
        );
    }

    #[test]
    fn voice_section_claims_experimental_realtime_settings() {
        let section = settings_section("voice").expect("voice section");
        assert!(section.matches_key("audio.microphone"));
        assert!(section.matches_key("realtime.version"));
        assert!(section.matches_key("experimental_realtime_ws_model"));
        assert_eq!(
            section
                .label_for_key("audio.microphone")
                .expect("microphone label"),
            "microphone"
        );
        assert_eq!(
            section
                .label_for_key("experimental_realtime_ws_model")
                .expect("realtime model label"),
            "realtime_ws_model"
        );
    }

    #[test]
    fn prompting_section_absorbs_compact_prompt_file_override() {
        let section = settings_section("prompting").expect("prompting section");
        assert!(section.matches_key("experimental_compact_prompt_file"));
        assert_eq!(
            section
                .label_for_key("experimental_compact_prompt_file")
                .expect("compact prompt file label"),
            "compact_prompt_file"
        );
    }

    #[test]
    fn tui_section_excludes_notification_keys() {
        let section = settings_section("tui").expect("tui section");
        assert!(section.matches_key("tui.theme"));
        assert!(section.matches_key("disable_paste_burst"));
        assert!(section.matches_key("file_opener"));
        assert!(!section.matches_key("tui.notification_method"));
        assert!(!section.matches_key("tui.notifications"));
        assert_eq!(
            section.label_for_key("tui.theme").expect("theme label"),
            "theme"
        );
    }

    #[test]
    fn storage_section_groups_history_and_ghost_snapshot() {
        let section = settings_section("storage").expect("storage section");
        assert_eq!(
            section
                .label_for_key("history.max_bytes")
                .expect("max bytes label"),
            "max_bytes"
        );
        assert_eq!(
            section
                .label_for_key("ghost_snapshot.disable_warnings")
                .expect("disable warnings label"),
            "disable_warnings"
        );
        assert_eq!(
            section.label_for_key("log_dir").expect("log dir label"),
            "log_dir"
        );
    }

    #[test]
    fn reasoning_output_section_groups_display_toggles() {
        let section = settings_section("reasoning_output").expect("reasoning_output section");
        assert!(section.matches_key("hide_agent_reasoning"));
        assert!(section.matches_key("show_raw_agent_reasoning"));
        assert!(!section.matches_key("disable_paste_burst"));
        assert_eq!(
            section
                .label_for_key("show_raw_agent_reasoning")
                .expect("show raw label"),
            "show_raw_agent_reasoning"
        );
    }
}
