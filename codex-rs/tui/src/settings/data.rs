use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;

use codex_core::config::settings_catalog::SettingScopeSupport;
use codex_core::features::FEATURES;
use codex_core::features::FeatureSpec;
use codex_core::features::Features;
use codex_core::features::Stage;
use serde_json::Value as JsonValue;

use crate::settings::schema::SchemaNode;
use crate::settings::schema::SchemaNodeKind;
use crate::settings::schema::SettingsSchema;
use crate::settings::schema::SettingsSectionDescriptor;
use crate::settings::schema::SettingsSectionMatcher;
use crate::settings::schema::load_settings_sections;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsScope {
    Global,
    ActiveProfile,
}

impl SettingsScope {
    pub(crate) fn default_for(active_profile: Option<&str>) -> Self {
        if active_profile.is_some() {
            Self::ActiveProfile
        } else {
            Self::Global
        }
    }

    pub(crate) fn normalized(self, active_profile: Option<&str>) -> Self {
        match (self, active_profile) {
            (Self::ActiveProfile, None) => Self::Global,
            _ => self,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingsScreen {
    Root,
    Section { section_key: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingsItemAction {
    EditScalar,
    EditToml,
    OpenModel,
    OpenProvider,
    OpenPersonality,
    OpenPermissions,
    OpenTheme,
    OpenStatusLine,
    OpenAudio,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SettingItemData {
    pub(crate) label: String,
    pub(crate) node: SchemaNode,
    pub(crate) display_value: String,
    pub(crate) editor_value: String,
    pub(crate) search_value: String,
    pub(crate) action: SettingsItemAction,
    pub(crate) disabled_reason: Option<String>,
    pub(crate) category_tag: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) selected_description: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SettingsRootItemKind {
    Section { section_key: String },
    Setting(Box<SettingItemData>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SettingsRootItemData {
    pub(crate) item_key: String,
    pub(crate) label: String,
    pub(crate) description: Option<String>,
    pub(crate) selected_description: Option<String>,
    pub(crate) search_value: String,
    pub(crate) disabled_reason: Option<String>,
    pub(crate) kind: SettingsRootItemKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SettingsSectionItemData {
    pub(crate) item_key: String,
    pub(crate) setting: SettingItemData,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SettingsSectionViewData {
    pub(crate) section_key: String,
    pub(crate) section_item: Option<SettingsSectionItemData>,
    pub(crate) items: Vec<SettingsSectionItemData>,
}

const FEATURES_SECTION_ID: &str = "features";

#[cfg(test)]
pub(crate) fn build_setting_items(
    schema: &SettingsSchema,
    effective_config: &toml::Value,
    origins: &HashMap<String, codex_app_server_protocol::ConfigLayerMetadata>,
    active_profile: Option<&str>,
    scope: SettingsScope,
) -> Vec<SettingItemData> {
    build_setting_items_with_features(
        schema,
        effective_config,
        origins,
        None,
        active_profile,
        scope,
    )
}

pub(crate) fn build_setting_items_with_features(
    schema: &SettingsSchema,
    effective_config: &toml::Value,
    origins: &HashMap<String, codex_app_server_protocol::ConfigLayerMetadata>,
    effective_features: Option<&Features>,
    active_profile: Option<&str>,
    scope: SettingsScope,
) -> Vec<SettingItemData> {
    let scope = scope.normalized(active_profile);
    let mut items = schema
        .nodes
        .iter()
        .filter(|node| !node.key_path.contains("[item]"))
        .filter(|node| !is_hidden_schema_setting(node.key_path.as_str()))
        .filter(|node| supports_scope(node, scope))
        .map(|node| build_setting_item(node, effective_config, origins, active_profile, scope))
        .collect::<Vec<_>>();
    if effective_features.is_some() || origins.keys().any(|key| key.starts_with("features.")) {
        items.extend(build_feature_setting_items(
            effective_config,
            origins,
            effective_features,
            active_profile,
            scope,
        ));
    }
    items
}

#[cfg(test)]
pub(crate) fn build_settings_root_items(
    schema: &SettingsSchema,
    effective_config: &toml::Value,
    origins: &HashMap<String, codex_app_server_protocol::ConfigLayerMetadata>,
    active_profile: Option<&str>,
    scope: SettingsScope,
) -> Vec<SettingsRootItemData> {
    build_settings_root_items_with_features(
        schema,
        effective_config,
        origins,
        None,
        active_profile,
        scope,
    )
}

pub(crate) fn build_settings_root_items_with_features(
    schema: &SettingsSchema,
    effective_config: &toml::Value,
    origins: &HashMap<String, codex_app_server_protocol::ConfigLayerMetadata>,
    effective_features: Option<&Features>,
    active_profile: Option<&str>,
    scope: SettingsScope,
) -> Vec<SettingsRootItemData> {
    let setting_items = build_setting_items_with_features(
        schema,
        effective_config,
        origins,
        effective_features,
        active_profile,
        scope,
    );
    let mut consumed = HashSet::<String>::new();
    let mut root_items = BTreeMap::<String, SettingsRootItemData>::new();

    for section in load_settings_sections() {
        let matched_items = setting_items
            .iter()
            .filter(|item| {
                !consumed.contains(item.node.key_path.as_str())
                    && section.matches_key(item.node.key_path.as_str())
            })
            .collect::<Vec<_>>();
        if matched_items.is_empty() {
            continue;
        }

        consumed.extend(matched_items.iter().map(|item| item.node.key_path.clone()));
        let root_item = build_manual_section_root_item(section, &matched_items);
        root_items.insert(root_item.label.clone(), root_item);
    }

    #[derive(Default)]
    struct RootBucket {
        direct_item: Option<SettingItemData>,
        child_items: Vec<SettingItemData>,
    }

    let mut buckets = BTreeMap::<String, RootBucket>::new();
    for item in setting_items {
        if consumed.contains(item.node.key_path.as_str()) {
            continue;
        }

        let root_key = root_key(item.node.key_path.as_str()).to_string();
        let bucket = buckets.entry(root_key.clone()).or_default();
        if item.node.key_path == root_key {
            bucket.direct_item = Some(item);
        } else {
            bucket.child_items.push(item);
        }
    }

    root_items.extend(buckets.into_iter().filter_map(|(root_key, bucket)| {
        if bucket.child_items.is_empty() {
            return bucket.direct_item.map(|mut item| {
                item.label = root_key.clone();
                (
                    item.label.clone(),
                    SettingsRootItemData {
                        item_key: item.node.key_path.clone(),
                        label: item.label.clone(),
                        description: item.description.clone(),
                        selected_description: item.selected_description.clone(),
                        search_value: item.search_value.clone(),
                        disabled_reason: item.disabled_reason.clone(),
                        kind: SettingsRootItemKind::Setting(Box::new(item)),
                    },
                )
            });
        }

        let item_count = bucket.child_items.len() + usize::from(bucket.direct_item.is_some());
        let count_label = match item_count {
            1 => "1 setting".to_string(),
            _ => format!("{item_count} settings"),
        };
        let section_description = bucket
            .direct_item
            .as_ref()
            .and_then(|item| item.node.description.clone())
            .unwrap_or_else(|| count_label.clone());
        let selected_description = bucket
            .direct_item
            .as_ref()
            .and_then(|item| item.node.description.as_deref())
            .map(|description| format!("{description} Open `{root_key}` to browse {count_label}."))
            .unwrap_or_else(|| format!("Open `{root_key}` to browse {count_label}."));
        let child_search = bucket
            .child_items
            .iter()
            .map(|item| item.node.key_path.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let search_value = format!("{root_key} {section_description} {child_search}");

        Some((
            root_key.clone(),
            SettingsRootItemData {
                item_key: root_key.clone(),
                label: root_key.clone(),
                description: Some(section_description),
                selected_description: Some(selected_description),
                search_value,
                disabled_reason: None,
                kind: SettingsRootItemKind::Section {
                    section_key: root_key,
                },
            },
        ))
    }));

    root_items.into_values().collect()
}

#[cfg(test)]
pub(crate) fn build_settings_section_view_data(
    schema: &SettingsSchema,
    effective_config: &toml::Value,
    origins: &HashMap<String, codex_app_server_protocol::ConfigLayerMetadata>,
    active_profile: Option<&str>,
    scope: SettingsScope,
    section_key: &str,
) -> SettingsSectionViewData {
    build_settings_section_view_data_with_features(
        schema,
        effective_config,
        origins,
        None,
        active_profile,
        scope,
        section_key,
    )
}

pub(crate) fn build_settings_section_view_data_with_features(
    schema: &SettingsSchema,
    effective_config: &toml::Value,
    origins: &HashMap<String, codex_app_server_protocol::ConfigLayerMetadata>,
    effective_features: Option<&Features>,
    active_profile: Option<&str>,
    scope: SettingsScope,
    section_key: &str,
) -> SettingsSectionViewData {
    if let Some(section) = manual_section(section_key) {
        return build_manual_settings_section_view_data(
            section,
            schema,
            effective_config,
            origins,
            effective_features,
            active_profile,
            scope,
        );
    }

    let mut section_item = None;
    let mut items = Vec::new();

    for item in build_setting_items_with_features(
        schema,
        effective_config,
        origins,
        effective_features,
        active_profile,
        scope,
    ) {
        if item.node.key_path == section_key {
            section_item = Some(SettingsSectionItemData {
                item_key: item.node.key_path.clone(),
                setting: SettingItemData {
                    label: "Edit this section".to_string(),
                    ..item
                },
            });
            continue;
        }

        let Some(label) = section_label(item.node.key_path.as_str(), section_key) else {
            continue;
        };
        items.push(SettingsSectionItemData {
            item_key: item.node.key_path.clone(),
            setting: SettingItemData { label, ..item },
        });
    }

    SettingsSectionViewData {
        section_key: section_key.to_string(),
        section_item,
        items,
    }
}

fn build_manual_section_root_item(
    section: &SettingsSectionDescriptor,
    matched_items: &[&SettingItemData],
) -> SettingsRootItemData {
    let item_count = matched_items.len();
    let count_label = match item_count {
        1 => "1 setting".to_string(),
        _ => format!("{item_count} settings"),
    };
    let summary_item = manual_section_summary_item(section, matched_items);
    let section_description = summary_item
        .and_then(|item| item.node.description.clone())
        .unwrap_or_else(|| count_label.clone());
    let selected_description = summary_item
        .and_then(|item| item.node.description.as_deref())
        .map(|description| {
            format!(
                "{description} Open `{}` to browse {count_label}.",
                section.id
            )
        })
        .unwrap_or_else(|| format!("Open `{}` to browse {count_label}.", section.id));
    let search_value = matched_items
        .iter()
        .map(|item| item.search_value.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    SettingsRootItemData {
        item_key: section.id.to_string(),
        label: section.id.to_string(),
        description: Some(section_description),
        selected_description: Some(selected_description),
        search_value,
        disabled_reason: None,
        kind: SettingsRootItemKind::Section {
            section_key: section.id.to_string(),
        },
    }
}

fn build_manual_settings_section_view_data(
    section: &SettingsSectionDescriptor,
    schema: &SettingsSchema,
    effective_config: &toml::Value,
    origins: &HashMap<String, codex_app_server_protocol::ConfigLayerMetadata>,
    effective_features: Option<&Features>,
    active_profile: Option<&str>,
    scope: SettingsScope,
) -> SettingsSectionViewData {
    let mut section_item = None;
    let mut items = Vec::new();

    for item in build_setting_items_with_features(
        schema,
        effective_config,
        origins,
        effective_features,
        active_profile,
        scope,
    ) {
        if !section.matches_key(item.node.key_path.as_str()) {
            continue;
        }

        if section
            .section_editor_key
            .is_some_and(|section_editor_key| item.node.key_path == section_editor_key)
        {
            section_item = Some(SettingsSectionItemData {
                item_key: item.node.key_path.clone(),
                setting: SettingItemData {
                    label: "Edit this section".to_string(),
                    ..item
                },
            });
            continue;
        }

        let label = section_item_label(section, &item);
        items.push(SettingsSectionItemData {
            item_key: item.node.key_path.clone(),
            setting: SettingItemData { label, ..item },
        });
    }

    if section.id != FEATURES_SECTION_ID {
        items.sort_by(|left, right| {
            left.setting
                .label
                .cmp(&right.setting.label)
                .then_with(|| left.item_key.cmp(&right.item_key))
        });
    }

    SettingsSectionViewData {
        section_key: section.id.to_string(),
        section_item,
        items,
    }
}

fn manual_section(section_id: &str) -> Option<&'static SettingsSectionDescriptor> {
    load_settings_sections()
        .iter()
        .find(|section| section.id == section_id)
}

fn section_item_label(section: &SettingsSectionDescriptor, item: &SettingItemData) -> String {
    if section.id == FEATURES_SECTION_ID
        && let Some(label) = feature_section_label(item.node.key_path.as_str())
    {
        return label;
    }

    section
        .label_for_key(item.node.key_path.as_str())
        .unwrap_or_else(|| item.node.key_path.clone())
}

fn manual_section_summary_item<'a>(
    section: &SettingsSectionDescriptor,
    matched_items: &'a [&SettingItemData],
) -> Option<&'a SettingItemData> {
    if let Some(section_editor_key) = section.section_editor_key
        && let Some(item) = matched_items
            .iter()
            .copied()
            .find(|item| item.node.key_path == section_editor_key)
    {
        return Some(item);
    }

    if let Some(item) = matched_items
        .iter()
        .copied()
        .find(|item| item.node.key_path == section.id)
    {
        return Some(item);
    }

    for member in section.members.iter().copied() {
        let summary_key = match member.matcher {
            SettingsSectionMatcher::ExactKey(key) | SettingsSectionMatcher::PathPrefix(key) => key,
            SettingsSectionMatcher::KeyPrefix(_) => continue,
        };
        if let Some(item) = matched_items
            .iter()
            .copied()
            .find(|item| item.node.key_path == summary_key)
        {
            return Some(item);
        }
    }

    None
}

fn build_setting_item(
    node: &SchemaNode,
    effective_config: &toml::Value,
    origins: &HashMap<String, codex_app_server_protocol::ConfigLayerMetadata>,
    active_profile: Option<&str>,
    scope: SettingsScope,
) -> SettingItemData {
    let scope = scope.normalized(active_profile);
    let scoped_key_path = scoped_key_path(node.key_path.as_str(), active_profile, scope);
    let scoped_value = value_for_path(effective_config, scoped_key_path.as_deref());
    let global_value = value_for_path(effective_config, Some(node.key_path.as_str()));
    let value = match scope {
        SettingsScope::Global => global_value,
        SettingsScope::ActiveProfile => scoped_value.or(global_value),
    };
    let origin_key = match scope {
        SettingsScope::Global => node.key_path.clone(),
        SettingsScope::ActiveProfile => scoped_key_path
            .clone()
            .filter(|key| origins.contains_key(key))
            .unwrap_or_else(|| node.key_path.clone()),
    };
    let origin = origins.get(&origin_key);
    let editor_value = value.map(format_value).unwrap_or_default();
    let display_value = value
        .map(format_inline_value)
        .unwrap_or_else(|| "<unset>".to_string());
    let disabled_reason = None;
    let action = action_for_key(node.key_path.as_str(), node.kind);
    let category_tag = origin.map(format_origin);
    let description = Some(display_value.clone());
    let selected_description = build_description(
        node,
        display_value.as_str(),
        category_tag.as_deref(),
        matches!(scope, SettingsScope::ActiveProfile)
            && scoped_value.is_none()
            && global_value.is_some(),
    );
    let search_value = [
        node.key_path.as_str(),
        node.title.as_str(),
        display_value.as_str(),
        node.description.as_deref().unwrap_or_default(),
        category_tag.as_deref().unwrap_or_default(),
    ]
    .join(" ");

    SettingItemData {
        label: node.key_path.clone(),
        node: node.clone(),
        display_value,
        editor_value,
        search_value,
        action,
        disabled_reason,
        category_tag,
        description,
        selected_description,
    }
}

fn build_feature_setting_items(
    effective_config: &toml::Value,
    origins: &HashMap<String, codex_app_server_protocol::ConfigLayerMetadata>,
    effective_features: Option<&Features>,
    active_profile: Option<&str>,
    scope: SettingsScope,
) -> Vec<SettingItemData> {
    FEATURES
        .iter()
        .filter(|spec| !matches!(spec.stage, Stage::Removed))
        .map(|spec| {
            build_feature_setting_item(
                spec,
                effective_config,
                origins,
                effective_features,
                active_profile,
                scope,
            )
        })
        .collect()
}

fn build_feature_setting_item(
    spec: &FeatureSpec,
    effective_config: &toml::Value,
    origins: &HashMap<String, codex_app_server_protocol::ConfigLayerMetadata>,
    effective_features: Option<&Features>,
    active_profile: Option<&str>,
    scope: SettingsScope,
) -> SettingItemData {
    let key_path = format!("features.{}", spec.key);
    let scoped_key_path = scoped_key_path(key_path.as_str(), active_profile, scope);
    let scoped_value = value_for_path(effective_config, scoped_key_path.as_deref());
    let global_value = value_for_path(effective_config, Some(key_path.as_str()));
    let is_inherited = matches!(scope, SettingsScope::ActiveProfile)
        && scoped_value.is_none()
        && global_value.is_some();
    let effective_enabled = effective_features
        .map(|features| features.enabled(spec.id))
        .unwrap_or_else(|| {
            match scope.normalized(active_profile) {
                SettingsScope::Global => global_value,
                SettingsScope::ActiveProfile => scoped_value.or(global_value),
            }
            .and_then(toml::Value::as_bool)
            .unwrap_or(spec.default_enabled)
        });
    let node = SchemaNode {
        key_path: key_path.clone(),
        title: feature_title(spec).to_string(),
        description: Some(feature_summary(spec)),
        kind: SchemaNodeKind::Boolean,
        enum_values: Vec::new(),
        default_value: Some(JsonValue::Bool(spec.default_enabled)),
        scopes: SettingScopeSupport {
            global: true,
            profile: true,
        },
    };
    let origin_key = match scope.normalized(active_profile) {
        SettingsScope::Global => key_path.clone(),
        SettingsScope::ActiveProfile => scoped_key_path
            .clone()
            .filter(|key| origins.contains_key(key))
            .unwrap_or_else(|| key_path.clone()),
    };
    let category_tag = origins.get(&origin_key).map(format_origin);
    let display_value = effective_enabled.to_string();
    let selected_description = build_description(
        &node,
        display_value.as_str(),
        category_tag.as_deref(),
        is_inherited,
    );
    let search_value = [
        key_path.as_str(),
        spec.key,
        node.title.as_str(),
        display_value.as_str(),
        node.description.as_deref().unwrap_or_default(),
        category_tag.as_deref().unwrap_or_default(),
    ]
    .join(" ");

    SettingItemData {
        label: key_path,
        node,
        display_value: display_value.clone(),
        editor_value: display_value,
        search_value,
        action: SettingsItemAction::EditScalar,
        disabled_reason: None,
        category_tag,
        description: Some(feature_summary(spec)),
        selected_description,
    }
}

fn feature_section_label(key_path: &str) -> Option<String> {
    feature_spec_for_settings_key_path(key_path).map(|spec| feature_title(spec).to_string())
}

fn feature_spec_for_settings_key_path(key_path: &str) -> Option<&'static FeatureSpec> {
    key_path
        .strip_prefix("features.")
        .and_then(|feature_key| FEATURES.iter().find(|spec| spec.key == feature_key))
}

fn feature_title(spec: &FeatureSpec) -> &'static str {
    match spec.stage {
        Stage::Experimental { name, .. } => name,
        Stage::UnderDevelopment | Stage::Stable | Stage::Deprecated => spec.key,
        Stage::Removed => spec.key,
    }
}

fn feature_summary(spec: &FeatureSpec) -> String {
    let default_state = if spec.default_enabled { "on" } else { "off" };
    match spec.stage {
        Stage::Experimental {
            menu_description, ..
        } => format!("{menu_description} Default: {default_state}."),
        Stage::UnderDevelopment => format!(
            "Under development. Incomplete and may behave unpredictably. Default: {default_state}."
        ),
        Stage::Stable => format!("Stable feature flag. Default: {default_state}."),
        Stage::Deprecated => format!(
            "Deprecated feature flag kept for backward compatibility. Default: {default_state}."
        ),
        Stage::Removed => format!("Removed feature flag. Default: {default_state}."),
    }
}

fn root_key(key_path: &str) -> &str {
    key_path.split_once('.').map_or(key_path, |(head, _)| head)
}

fn section_label(key_path: &str, section_key: &str) -> Option<String> {
    key_path
        .strip_prefix(section_key)
        .and_then(|suffix| suffix.strip_prefix('.'))
        .map(ToOwned::to_owned)
}

fn supports_scope(node: &SchemaNode, scope: SettingsScope) -> bool {
    match scope {
        SettingsScope::Global => node.scopes.global,
        SettingsScope::ActiveProfile => node.scopes.profile,
    }
}

fn build_description(
    node: &SchemaNode,
    display_value: &str,
    source_tag: Option<&str>,
    is_inherited: bool,
) -> Option<String> {
    let mut parts = vec![format!("Current: {display_value}.")];
    if let Some(source_tag) = source_tag {
        parts.push(format!("Source: {source_tag}."));
    }
    if is_inherited {
        parts.push("Inherited from global config.".to_string());
    }
    if let Some(description) = node.description.as_deref() {
        parts.push(description.to_string());
    }

    let kind = match node.kind {
        SchemaNodeKind::Boolean => "bool",
        SchemaNodeKind::Integer => "integer",
        SchemaNodeKind::Number => "number",
        SchemaNodeKind::String => "string",
        SchemaNodeKind::Array => "array",
        SchemaNodeKind::Object => "object",
        SchemaNodeKind::Unknown => "unknown",
    };
    parts.push(format!("Type: {kind}."));
    if !node.enum_values.is_empty() {
        parts.push(format!("Options: {}.", node.enum_values.join(", ")));
    }
    if let Some(default_value) = &node.default_value {
        parts.push(format!("Default: {default_value}."));
    }

    Some(parts.join(" "))
}

fn action_for_key(key_path: &str, kind: SchemaNodeKind) -> SettingsItemAction {
    match key_path {
        "model" | "model_reasoning_effort" | "model_reasoning_summary" | "model_verbosity" => {
            SettingsItemAction::OpenModel
        }
        "model_provider" => SettingsItemAction::OpenProvider,
        "personality" => SettingsItemAction::OpenPersonality,
        "approval_policy" | "sandbox_mode" | "sandbox_workspace_write" | "approvals_reviewer" => {
            SettingsItemAction::OpenPermissions
        }
        "tui.theme" => SettingsItemAction::OpenTheme,
        "tui.status_line" => SettingsItemAction::OpenStatusLine,
        "audio" | "audio.microphone" | "audio.speaker" => SettingsItemAction::OpenAudio,
        _ => match kind {
            SchemaNodeKind::Boolean
            | SchemaNodeKind::Integer
            | SchemaNodeKind::Number
            | SchemaNodeKind::String => SettingsItemAction::EditScalar,
            SchemaNodeKind::Array | SchemaNodeKind::Object | SchemaNodeKind::Unknown => {
                SettingsItemAction::EditToml
            }
        },
    }
}

fn is_hidden_schema_setting(key_path: &str) -> bool {
    matches!(
        key_path,
        "features"
            | "include_apply_patch_tool"
            | "experimental_use_freeform_apply_patch"
            | "experimental_use_unified_exec_tool"
    ) || key_path.starts_with("features.")
}

fn scoped_key_path(
    key_path: &str,
    active_profile: Option<&str>,
    scope: SettingsScope,
) -> Option<String> {
    match scope {
        SettingsScope::Global => Some(key_path.to_string()),
        SettingsScope::ActiveProfile => {
            active_profile.map(|profile| format!("profiles.{profile}.{key_path}"))
        }
    }
}

fn value_for_path<'a>(value: &'a toml::Value, key_path: Option<&str>) -> Option<&'a toml::Value> {
    let key_path = key_path?;
    let mut current = value;
    for segment in key_path.split('.') {
        current = current.as_table()?.get(segment)?;
    }
    Some(current)
}

fn format_value(value: &toml::Value) -> String {
    match value {
        toml::Value::String(value) => value.clone(),
        toml::Value::Integer(value) => value.to_string(),
        toml::Value::Float(value) => value.to_string(),
        toml::Value::Boolean(value) => value.to_string(),
        toml::Value::Array(value) => toml::to_string_pretty(&toml::Value::Array(value.clone()))
            .unwrap_or_else(|_| format!("{value:?}"))
            .trim()
            .to_string(),
        toml::Value::Table(value) => toml::to_string_pretty(value)
            .unwrap_or_else(|_| format!("{value:?}"))
            .trim()
            .to_string(),
        toml::Value::Datetime(value) => value.to_string(),
    }
}

fn format_inline_value(value: &toml::Value) -> String {
    format_value(value)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn format_origin(origin: &codex_app_server_protocol::ConfigLayerMetadata) -> String {
    use codex_app_server_protocol::ConfigLayerSource;

    match &origin.name {
        ConfigLayerSource::User { .. } => "user".to_string(),
        ConfigLayerSource::Project { .. } => "project".to_string(),
        ConfigLayerSource::System { .. } => "system".to_string(),
        ConfigLayerSource::SessionFlags => "session".to_string(),
        ConfigLayerSource::Mdm { .. } => "mdm".to_string(),
        ConfigLayerSource::LegacyManagedConfigTomlFromFile { .. } => "managed".to_string(),
        ConfigLayerSource::LegacyManagedConfigTomlFromMdm => "managed-mdm".to_string(),
    }
}

pub(crate) fn parse_scalar_input(kind: SchemaNodeKind, input: &str) -> Result<toml::Value, String> {
    let trimmed = input.trim();
    match kind {
        SchemaNodeKind::Boolean => trimmed
            .parse::<bool>()
            .map(toml::Value::Boolean)
            .map_err(|_| "Expected `true` or `false`.".to_string()),
        SchemaNodeKind::Integer => trimmed
            .parse::<i64>()
            .map(toml::Value::Integer)
            .map_err(|_| "Expected an integer.".to_string()),
        SchemaNodeKind::Number => trimmed
            .parse::<f64>()
            .map(toml::Value::Float)
            .map_err(|_| "Expected a number.".to_string()),
        SchemaNodeKind::String => {
            if (trimmed.starts_with('"') && trimmed.ends_with('"'))
                || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
            {
                match parse_toml_fragment(trimmed)? {
                    toml::Value::String(value) => Ok(toml::Value::String(value)),
                    _ => Err("Expected a string.".to_string()),
                }
            } else {
                Ok(toml::Value::String(trimmed.to_string()))
            }
        }
        SchemaNodeKind::Array | SchemaNodeKind::Object | SchemaNodeKind::Unknown => {
            Err("This setting requires TOML editing.".to_string())
        }
    }
}

pub(crate) fn parse_toml_fragment(input: &str) -> Result<toml::Value, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Value cannot be empty.".to_string());
    }

    if let Ok(value) = toml::from_str::<toml::Value>(trimmed) {
        return Ok(value);
    }

    toml::from_str::<toml::Value>(&format!("value = {trimmed}"))
        .map_err(|err| format!("Invalid TOML: {err}"))
        .and_then(|value| {
            value
                .get("value")
                .cloned()
                .ok_or_else(|| "Invalid TOML value.".to_string())
        })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use codex_app_server_protocol::ConfigLayerMetadata;
    use codex_app_server_protocol::ConfigLayerSource;
    use pretty_assertions::assert_eq;

    use crate::settings::schema::SchemaNode;
    use crate::settings::schema::SchemaNodeKind;
    use crate::settings::schema::SettingsSchema;
    use codex_core::config::settings_catalog::SettingScopeSupport;
    use codex_core::features::Feature;
    use codex_core::features::Features;

    use super::SettingsItemAction;
    use super::SettingsRootItemKind;
    use super::SettingsScope;
    use super::build_setting_items;
    use super::build_setting_items_with_features;
    use super::build_settings_root_items;
    use super::build_settings_section_view_data;
    use super::build_settings_section_view_data_with_features;
    use super::parse_scalar_input;
    use super::parse_toml_fragment;

    fn metadata() -> ConfigLayerMetadata {
        ConfigLayerMetadata {
            name: ConfigLayerSource::User {
                file: codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(
                    "/tmp/config.toml",
                )
                .expect("absolute path"),
            },
            version: "sha256:test".to_string(),
        }
    }

    fn find_setting_item<'a>(
        items: &'a [super::SettingItemData],
        key_path: &str,
    ) -> &'a super::SettingItemData {
        items
            .iter()
            .find(|item| item.node.key_path == key_path)
            .unwrap_or_else(|| panic!("missing setting item `{key_path}`"))
    }

    #[test]
    fn builds_setting_items_with_profile_scope_rules() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "model".to_string(),
                    title: "model".to_string(),
                    description: Some("Model".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "audio.microphone".to_string(),
                    title: "microphone".to_string(),
                    description: Some("Mic".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
model = "gpt-5"
[audio]
microphone = "Desk Mic"
[profiles.dev]
model = "o3"
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("model".to_string(), metadata());
        origins.insert("audio.microphone".to_string(), metadata());
        origins.insert("profiles.dev.model".to_string(), metadata());

        let items = build_setting_items(
            &schema,
            &config,
            &origins,
            Some("dev"),
            SettingsScope::ActiveProfile,
        );

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].display_value, "o3");
        assert_eq!(items[0].editor_value, "o3");
        assert_eq!(items[0].action, SettingsItemAction::OpenModel);
    }

    #[test]
    fn builds_root_items_without_repeating_section_children() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "audio".to_string(),
                    title: "audio".to_string(),
                    description: Some("Audio device settings".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "audio.microphone".to_string(),
                    title: "microphone".to_string(),
                    description: Some("Mic".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "model".to_string(),
                    title: "model".to_string(),
                    description: Some("Model".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
model = "gpt-5"
[audio]
microphone = "Desk Mic"
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("audio".to_string(), metadata());
        origins.insert("audio.microphone".to_string(), metadata());
        origins.insert("model".to_string(), metadata());

        let items =
            build_settings_root_items(&schema, &config, &origins, None, SettingsScope::Global);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].item_key, "model");
        assert_eq!(items[1].item_key, "voice");
        assert_eq!(items[1].label, "voice");
        assert!(matches!(
            &items[1].kind,
            SettingsRootItemKind::Section { .. }
        ));
    }

    #[test]
    fn builds_section_items_with_stripped_labels_and_section_editor() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "audio".to_string(),
                    title: "audio".to_string(),
                    description: Some("Audio device settings".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "audio.microphone".to_string(),
                    title: "microphone".to_string(),
                    description: Some("Mic".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "audio.speaker".to_string(),
                    title: "speaker".to_string(),
                    description: Some("Speaker".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
[audio]
microphone = "Desk Mic"
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("audio".to_string(), metadata());
        origins.insert("audio.microphone".to_string(), metadata());
        origins.insert("audio.speaker".to_string(), metadata());

        let section_view = build_settings_section_view_data(
            &schema,
            &config,
            &origins,
            None,
            SettingsScope::Global,
            "audio",
        );

        assert_eq!(section_view.section_key, "audio");
        assert_eq!(
            section_view
                .section_item
                .as_ref()
                .expect("section item")
                .setting
                .label,
            "Edit this section"
        );
        assert_eq!(section_view.items.len(), 2);
        assert_eq!(section_view.items[0].setting.label, "microphone");
        assert_eq!(section_view.items[1].setting.label, "speaker");
    }

    #[test]
    fn manual_model_section_absorbs_snake_case_family() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "model".to_string(),
                    title: "model".to_string(),
                    description: Some("Model".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "model_reasoning_effort".to_string(),
                    title: "model_reasoning_effort".to_string(),
                    description: Some("Effort".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "plan_mode_reasoning_effort".to_string(),
                    title: "plan_mode_reasoning_effort".to_string(),
                    description: Some("Plan effort".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "service_tier".to_string(),
                    title: "service_tier".to_string(),
                    description: Some("Tier".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
model = "gpt-5"
model_reasoning_effort = "high"
plan_mode_reasoning_effort = "medium"
service_tier = "flex"
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("model".to_string(), metadata());
        origins.insert("model_reasoning_effort".to_string(), metadata());
        origins.insert("plan_mode_reasoning_effort".to_string(), metadata());
        origins.insert("service_tier".to_string(), metadata());

        let root_items =
            build_settings_root_items(&schema, &config, &origins, None, SettingsScope::Global);

        assert_eq!(root_items.len(), 1);
        assert_eq!(root_items[0].item_key, "model");
        assert!(matches!(
            &root_items[0].kind,
            SettingsRootItemKind::Section { section_key } if section_key == "model"
        ));

        let section_view = build_settings_section_view_data(
            &schema,
            &config,
            &origins,
            None,
            SettingsScope::Global,
            "model",
        );

        assert!(section_view.section_item.is_none());
        assert_eq!(
            section_view
                .items
                .iter()
                .map(|item| item.setting.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "model",
                "plan_mode_reasoning_effort",
                "reasoning_effort",
                "service_tier",
            ]
        );
    }

    #[test]
    fn manual_tools_section_disambiguates_flat_and_nested_keys() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "background_terminal_max_timeout".to_string(),
                    title: "background_terminal_max_timeout".to_string(),
                    description: Some("Terminal timeout".to_string()),
                    kind: SchemaNodeKind::Integer,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "js_repl_node_module_dirs".to_string(),
                    title: "js_repl_node_module_dirs".to_string(),
                    description: Some("Node module dirs".to_string()),
                    kind: SchemaNodeKind::Array,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "js_repl_node_path".to_string(),
                    title: "js_repl_node_path".to_string(),
                    description: Some("Node path".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "tools".to_string(),
                    title: "tools".to_string(),
                    description: Some("Tools".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "tools.view_image".to_string(),
                    title: "view_image".to_string(),
                    description: Some("Tool view_image".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "tools.web_search".to_string(),
                    title: "web_search".to_string(),
                    description: Some("Tool web search".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "tools.web_search.location.city".to_string(),
                    title: "city".to_string(),
                    description: Some("City".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "tool_output_token_limit".to_string(),
                    title: "tool_output_token_limit".to_string(),
                    description: Some("Tool output limit".to_string()),
                    kind: SchemaNodeKind::Integer,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "tools_view_image".to_string(),
                    title: "tools_view_image".to_string(),
                    description: Some("Legacy view image".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: false,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "web_search".to_string(),
                    title: "web_search".to_string(),
                    description: Some("Search mode".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "zsh_path".to_string(),
                    title: "zsh_path".to_string(),
                    description: Some("Zsh path".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
web_search = "live"
[profiles.dev]
js_repl_node_path = "/usr/bin/node"
js_repl_node_module_dirs = ["/tmp/node_modules"]
tools_view_image = true
zsh_path = "/bin/zsh"
[tools]
view_image = true
[tools.web_search.location]
city = "Tokyo"
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("web_search".to_string(), metadata());
        origins.insert("tools".to_string(), metadata());
        origins.insert("tools.view_image".to_string(), metadata());
        origins.insert("tools.web_search".to_string(), metadata());
        origins.insert("tools.web_search.location.city".to_string(), metadata());
        origins.insert(
            "profiles.dev.js_repl_node_module_dirs".to_string(),
            metadata(),
        );
        origins.insert("profiles.dev.js_repl_node_path".to_string(), metadata());
        origins.insert("profiles.dev.tools_view_image".to_string(), metadata());
        origins.insert("profiles.dev.zsh_path".to_string(), metadata());

        let section_view = build_settings_section_view_data(
            &schema,
            &config,
            &origins,
            Some("dev"),
            SettingsScope::ActiveProfile,
            "tools",
        );

        assert_eq!(
            section_view
                .section_item
                .as_ref()
                .expect("tools section editor")
                .setting
                .label,
            "Edit this section"
        );
        assert_eq!(
            section_view
                .items
                .iter()
                .map(|item| item.setting.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "js_repl_node_module_dirs",
                "js_repl_node_path",
                "tools_view_image",
                "view_image",
                "web_search",
                "web_search.location.city",
                "web_search_mode",
                "zsh_path",
            ]
        );
    }

    #[test]
    fn manual_auth_section_absorbs_forced_login_fields() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "cli_auth_credentials_store".to_string(),
                    title: "cli_auth_credentials_store".to_string(),
                    description: Some("Credentials backend".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "forced_chatgpt_workspace_id".to_string(),
                    title: "forced_chatgpt_workspace_id".to_string(),
                    description: Some("Workspace".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "forced_login_method".to_string(),
                    title: "forced_login_method".to_string(),
                    description: Some("Login method".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
cli_auth_credentials_store = "keyring"
forced_chatgpt_workspace_id = "ws_123"
forced_login_method = "chatgpt"
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("cli_auth_credentials_store".to_string(), metadata());
        origins.insert("forced_chatgpt_workspace_id".to_string(), metadata());
        origins.insert("forced_login_method".to_string(), metadata());

        let root_items =
            build_settings_root_items(&schema, &config, &origins, None, SettingsScope::Global);

        assert_eq!(root_items.len(), 1);
        assert_eq!(root_items[0].item_key, "auth");
        assert!(matches!(
            &root_items[0].kind,
            SettingsRootItemKind::Section { section_key } if section_key == "auth"
        ));

        let section_view = build_settings_section_view_data(
            &schema,
            &config,
            &origins,
            None,
            SettingsScope::Global,
            "auth",
        );

        assert!(section_view.section_item.is_none());
        assert_eq!(
            section_view
                .items
                .iter()
                .map(|item| item.setting.label.as_str())
                .collect::<Vec<_>>(),
            vec!["chatgpt_workspace_id", "credentials_store", "login_method"]
        );
    }

    #[test]
    fn manual_reasoning_output_section_groups_display_toggles() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "hide_agent_reasoning".to_string(),
                    title: "hide_agent_reasoning".to_string(),
                    description: Some("Hide reasoning".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "show_raw_agent_reasoning".to_string(),
                    title: "show_raw_agent_reasoning".to_string(),
                    description: Some("Show raw reasoning".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "disable_paste_burst".to_string(),
                    title: "disable_paste_burst".to_string(),
                    description: Some("Disable paste burst".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
hide_agent_reasoning = true
show_raw_agent_reasoning = false
disable_paste_burst = true
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("hide_agent_reasoning".to_string(), metadata());
        origins.insert("show_raw_agent_reasoning".to_string(), metadata());
        origins.insert("disable_paste_burst".to_string(), metadata());

        let root_items =
            build_settings_root_items(&schema, &config, &origins, None, SettingsScope::Global);

        assert_eq!(root_items.len(), 2);
        assert_eq!(root_items[0].item_key, "reasoning_output");
        assert_eq!(root_items[1].item_key, "tui");

        let section_view = build_settings_section_view_data(
            &schema,
            &config,
            &origins,
            None,
            SettingsScope::Global,
            "reasoning_output",
        );

        assert!(section_view.section_item.is_none());
        assert_eq!(
            section_view
                .items
                .iter()
                .map(|item| item.setting.label.as_str())
                .collect::<Vec<_>>(),
            vec!["hide_agent_reasoning", "show_raw_agent_reasoning"]
        );
    }

    #[test]
    fn manual_storage_section_absorbs_history_and_ghost_snapshot_settings() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "ghost_snapshot".to_string(),
                    title: "ghost_snapshot".to_string(),
                    description: Some("Ghost snapshot".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "ghost_snapshot.disable_warnings".to_string(),
                    title: "disable_warnings".to_string(),
                    description: Some("Disable warnings".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "history".to_string(),
                    title: "history".to_string(),
                    description: Some("History".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "history.max_bytes".to_string(),
                    title: "max_bytes".to_string(),
                    description: Some("Max bytes".to_string()),
                    kind: SchemaNodeKind::Integer,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "log_dir".to_string(),
                    title: "log_dir".to_string(),
                    description: Some("Log dir".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "sqlite_home".to_string(),
                    title: "sqlite_home".to_string(),
                    description: Some("SQLite home".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
log_dir = "/tmp/log"
sqlite_home = "/tmp/sqlite"
[history]
max_bytes = 4096
[ghost_snapshot]
disable_warnings = true
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("ghost_snapshot".to_string(), metadata());
        origins.insert("ghost_snapshot.disable_warnings".to_string(), metadata());
        origins.insert("history".to_string(), metadata());
        origins.insert("history.max_bytes".to_string(), metadata());
        origins.insert("log_dir".to_string(), metadata());
        origins.insert("sqlite_home".to_string(), metadata());

        let root_items =
            build_settings_root_items(&schema, &config, &origins, None, SettingsScope::Global);

        assert_eq!(root_items.len(), 1);
        assert_eq!(root_items[0].item_key, "storage");

        let section_view = build_settings_section_view_data(
            &schema,
            &config,
            &origins,
            None,
            SettingsScope::Global,
            "storage",
        );

        assert!(section_view.section_item.is_none());
        assert_eq!(
            section_view
                .items
                .iter()
                .map(|item| item.setting.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "disable_warnings",
                "ghost_snapshot",
                "history",
                "log_dir",
                "max_bytes",
                "sqlite_home",
            ]
        );
    }

    #[test]
    fn manual_notifications_section_absorbs_notice_and_tui_notification_controls() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "check_for_update_on_startup".to_string(),
                    title: "check_for_update_on_startup".to_string(),
                    description: Some("Check for update".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "notice".to_string(),
                    title: "notice".to_string(),
                    description: Some("Notice settings".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "notice.hide_full_access_warning".to_string(),
                    title: "hide_full_access_warning".to_string(),
                    description: Some("Hide warning".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "notify".to_string(),
                    title: "notify".to_string(),
                    description: Some("Notify command".to_string()),
                    kind: SchemaNodeKind::Array,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "tui.notification_method".to_string(),
                    title: "notification_method".to_string(),
                    description: Some("Notification method".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "tui.notifications".to_string(),
                    title: "notifications".to_string(),
                    description: Some("Notifications".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
check_for_update_on_startup = true
notify = ["terminal-notifier"]
[notice]
hide_full_access_warning = true
[tui]
notification_method = "bell"
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("check_for_update_on_startup".to_string(), metadata());
        origins.insert("notice".to_string(), metadata());
        origins.insert("notice.hide_full_access_warning".to_string(), metadata());
        origins.insert("notify".to_string(), metadata());
        origins.insert("tui.notification_method".to_string(), metadata());
        origins.insert("tui.notifications".to_string(), metadata());

        let root_items =
            build_settings_root_items(&schema, &config, &origins, None, SettingsScope::Global);

        assert_eq!(root_items.len(), 1);
        assert_eq!(root_items[0].item_key, "notifications");

        let section_view = build_settings_section_view_data(
            &schema,
            &config,
            &origins,
            None,
            SettingsScope::Global,
            "notifications",
        );

        assert!(section_view.section_item.is_none());
        assert_eq!(
            section_view
                .items
                .iter()
                .map(|item| item.setting.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "check_for_update_on_startup",
                "external_command",
                "hide_full_access_warning",
                "notice",
                "notification_method",
                "notifications",
            ]
        );
    }

    #[test]
    fn manual_tui_section_excludes_notification_controls() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "disable_paste_burst".to_string(),
                    title: "disable_paste_burst".to_string(),
                    description: Some("Disable paste burst".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "file_opener".to_string(),
                    title: "file_opener".to_string(),
                    description: Some("File opener".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "tui".to_string(),
                    title: "tui".to_string(),
                    description: Some("TUI settings".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "tui.notification_method".to_string(),
                    title: "notification_method".to_string(),
                    description: Some("Notification method".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "tui.notifications".to_string(),
                    title: "notifications".to_string(),
                    description: Some("Notifications".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "tui.theme".to_string(),
                    title: "theme".to_string(),
                    description: Some("Theme".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
disable_paste_burst = true
file_opener = "vscode"
[tui]
theme = "sunrise"
notification_method = "bell"
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("disable_paste_burst".to_string(), metadata());
        origins.insert("file_opener".to_string(), metadata());
        origins.insert("tui".to_string(), metadata());
        origins.insert("tui.notification_method".to_string(), metadata());
        origins.insert("tui.notifications".to_string(), metadata());
        origins.insert("tui.theme".to_string(), metadata());

        let root_items =
            build_settings_root_items(&schema, &config, &origins, None, SettingsScope::Global);

        assert_eq!(
            root_items
                .iter()
                .map(|item| item.item_key.as_str())
                .collect::<Vec<_>>(),
            vec!["notifications", "tui"]
        );

        let section_view = build_settings_section_view_data(
            &schema,
            &config,
            &origins,
            None,
            SettingsScope::Global,
            "tui",
        );

        assert_eq!(
            section_view
                .section_item
                .as_ref()
                .expect("tui section editor")
                .setting
                .label,
            "Edit this section"
        );
        assert_eq!(
            section_view
                .items
                .iter()
                .map(|item| item.setting.label.as_str())
                .collect::<Vec<_>>(),
            vec!["disable_paste_burst", "file_opener", "theme"]
        );
    }

    #[test]
    fn manual_voice_section_absorbs_audio_and_realtime_settings() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "audio".to_string(),
                    title: "audio".to_string(),
                    description: Some("Audio settings".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "audio.microphone".to_string(),
                    title: "microphone".to_string(),
                    description: Some("Microphone".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "experimental_realtime_ws_model".to_string(),
                    title: "experimental_realtime_ws_model".to_string(),
                    description: Some("Realtime model".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "realtime".to_string(),
                    title: "realtime".to_string(),
                    description: Some("Realtime".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
                SchemaNode {
                    key_path: "realtime.version".to_string(),
                    title: "version".to_string(),
                    description: Some("Version".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: false,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
experimental_realtime_ws_model = "gpt-realtime"
[audio]
microphone = "Desk Mic"
[realtime]
version = "v2"
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("audio".to_string(), metadata());
        origins.insert("audio.microphone".to_string(), metadata());
        origins.insert("experimental_realtime_ws_model".to_string(), metadata());
        origins.insert("realtime".to_string(), metadata());
        origins.insert("realtime.version".to_string(), metadata());

        let root_items =
            build_settings_root_items(&schema, &config, &origins, None, SettingsScope::Global);

        assert_eq!(root_items.len(), 1);
        assert_eq!(root_items[0].item_key, "voice");

        let section_view = build_settings_section_view_data(
            &schema,
            &config,
            &origins,
            None,
            SettingsScope::Global,
            "voice",
        );

        assert!(section_view.section_item.is_none());
        assert_eq!(
            section_view
                .items
                .iter()
                .map(|item| item.setting.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "audio",
                "microphone",
                "realtime",
                "realtime_ws_model",
                "version"
            ]
        );
    }

    #[test]
    fn manual_telemetry_section_keeps_full_labels_to_avoid_enabled_collisions() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "analytics".to_string(),
                    title: "analytics".to_string(),
                    description: Some("Analytics".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "analytics.enabled".to_string(),
                    title: "enabled".to_string(),
                    description: Some("Analytics enabled".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "feedback".to_string(),
                    title: "feedback".to_string(),
                    description: Some("Feedback".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "feedback.enabled".to_string(),
                    title: "enabled".to_string(),
                    description: Some("Feedback enabled".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "otel".to_string(),
                    title: "otel".to_string(),
                    description: Some("OTEL".to_string()),
                    kind: SchemaNodeKind::Object,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "otel.environment".to_string(),
                    title: "environment".to_string(),
                    description: Some("Environment".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
[analytics]
enabled = true
[feedback]
enabled = false
[otel]
environment = "dev"
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("analytics".to_string(), metadata());
        origins.insert("analytics.enabled".to_string(), metadata());
        origins.insert("feedback".to_string(), metadata());
        origins.insert("feedback.enabled".to_string(), metadata());
        origins.insert("otel".to_string(), metadata());
        origins.insert("otel.environment".to_string(), metadata());

        let root_items =
            build_settings_root_items(&schema, &config, &origins, None, SettingsScope::Global);

        assert_eq!(root_items.len(), 1);
        assert_eq!(root_items[0].item_key, "telemetry");

        let section_view = build_settings_section_view_data(
            &schema,
            &config,
            &origins,
            None,
            SettingsScope::Global,
            "telemetry",
        );

        assert!(section_view.section_item.is_none());
        assert_eq!(
            section_view
                .items
                .iter()
                .map(|item| item.setting.label.as_str())
                .collect::<Vec<_>>(),
            vec![
                "analytics",
                "analytics.enabled",
                "feedback",
                "feedback.enabled",
                "otel",
                "otel.environment",
            ]
        );
    }

    #[test]
    fn parses_scalar_and_toml_values() {
        assert_eq!(
            parse_scalar_input(SchemaNodeKind::Boolean, "true").expect("bool"),
            toml::Value::Boolean(true)
        );
        assert_eq!(
            parse_scalar_input(SchemaNodeKind::String, "\"__clear__\"").expect("quoted string"),
            toml::Value::String("__clear__".to_string())
        );
        assert_eq!(
            parse_toml_fragment("{ enabled = true }").expect("table"),
            toml::Value::Table(
                [("enabled".to_string(), toml::Value::Boolean(true))]
                    .into_iter()
                    .collect()
            )
        );
    }

    #[test]
    fn formats_array_values_for_editor_and_display() {
        let schema = SettingsSchema {
            nodes: vec![SchemaNode {
                key_path: "allowed_tools".to_string(),
                title: "allowed_tools".to_string(),
                description: Some("Allowed tool list".to_string()),
                kind: SchemaNodeKind::Array,
                enum_values: Vec::new(),
                default_value: None,
                scopes: SettingScopeSupport {
                    global: true,
                    profile: false,
                },
            }],
        };
        let config: toml::Value =
            toml::from_str("allowed_tools = [\"alpha\", \"beta\"]").expect("config");
        let mut origins = HashMap::new();
        origins.insert("allowed_tools".to_string(), metadata());

        let items = build_setting_items(&schema, &config, &origins, None, SettingsScope::Global);

        assert!(items[0].editor_value.starts_with('['));
        assert!(items[0].editor_value.contains("\"alpha\""));
        assert!(items[0].editor_value.contains("\"beta\""));
        assert!(items[0].display_value.starts_with('['));
        assert!(items[0].display_value.contains("\"alpha\""));
        assert!(items[0].display_value.contains("\"beta\""));
    }

    #[test]
    fn feature_registry_items_replace_legacy_aliases() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "include_apply_patch_tool".to_string(),
                    title: "include_apply_patch_tool".to_string(),
                    description: Some("Legacy apply_patch toggle".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "experimental_use_freeform_apply_patch".to_string(),
                    title: "experimental_use_freeform_apply_patch".to_string(),
                    description: Some("Legacy freeform apply_patch toggle".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "experimental_use_unified_exec_tool".to_string(),
                    title: "experimental_use_unified_exec_tool".to_string(),
                    description: Some("Legacy unified exec toggle".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
include_apply_patch_tool = true
experimental_use_freeform_apply_patch = true
experimental_use_unified_exec_tool = true
"#,
        )
        .expect("config");
        let origins = HashMap::from([
            ("include_apply_patch_tool".to_string(), metadata()),
            (
                "experimental_use_freeform_apply_patch".to_string(),
                metadata(),
            ),
            ("experimental_use_unified_exec_tool".to_string(), metadata()),
        ]);
        let mut effective_features = Features::with_defaults();
        effective_features
            .set_enabled(Feature::ApplyPatchFreeform, true)
            .set_enabled(Feature::UnifiedExec, true);

        let items = build_setting_items_with_features(
            &schema,
            &config,
            &origins,
            Some(&effective_features),
            None,
            SettingsScope::Global,
        );
        let key_paths = items
            .iter()
            .map(|item| item.node.key_path.as_str())
            .collect::<Vec<_>>();

        assert!(key_paths.contains(&"features.apply_patch_freeform"));
        assert!(key_paths.contains(&"features.unified_exec"));
        assert!(!key_paths.contains(&"include_apply_patch_tool"));
        assert!(!key_paths.contains(&"experimental_use_freeform_apply_patch"));
        assert!(!key_paths.contains(&"experimental_use_unified_exec_tool"));
    }

    #[test]
    fn hides_profile_only_settings_from_global_scope() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "include_apply_patch_tool".to_string(),
                    title: "include_apply_patch_tool".to_string(),
                    description: Some("Include apply_patch".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: false,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "model".to_string(),
                    title: "model".to_string(),
                    description: Some("Model".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
model = "gpt-5"
[profiles.dev]
include_apply_patch_tool = true
"#,
        )
        .expect("config");
        let mut origins = HashMap::new();
        origins.insert("model".to_string(), metadata());
        origins.insert(
            "profiles.dev.include_apply_patch_tool".to_string(),
            metadata(),
        );

        let items = build_setting_items(&schema, &config, &origins, None, SettingsScope::Global);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].node.key_path, "model");
    }

    #[test]
    fn features_section_appears_in_root_view() {
        let schema = SettingsSchema {
            nodes: vec![
                SchemaNode {
                    key_path: "model".to_string(),
                    title: "model".to_string(),
                    description: Some("Model".to_string()),
                    kind: SchemaNodeKind::String,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
                SchemaNode {
                    key_path: "suppress_unstable_features_warning".to_string(),
                    title: "suppress_unstable_features_warning".to_string(),
                    description: Some("Hide unstable feature warnings".to_string()),
                    kind: SchemaNodeKind::Boolean,
                    enum_values: Vec::new(),
                    default_value: None,
                    scopes: SettingScopeSupport {
                        global: true,
                        profile: true,
                    },
                },
            ],
        };
        let config: toml::Value = toml::from_str(
            r#"
model = "gpt-5"
suppress_unstable_features_warning = true
"#,
        )
        .expect("config");
        let origins = HashMap::from([
            ("model".to_string(), metadata()),
            ("suppress_unstable_features_warning".to_string(), metadata()),
        ]);

        let items =
            build_settings_root_items(&schema, &config, &origins, None, SettingsScope::Global);
        let features = items
            .iter()
            .find(|item| item.item_key == "features")
            .expect("missing features section");

        assert_eq!(features.label, "features");
        assert!(matches!(
            &features.kind,
            SettingsRootItemKind::Section { section_key } if section_key == "features"
        ));
    }

    #[test]
    fn features_section_uses_friendly_labels_for_experimental_flags() {
        let schema = SettingsSchema {
            nodes: vec![SchemaNode {
                key_path: "suppress_unstable_features_warning".to_string(),
                title: "suppress_unstable_features_warning".to_string(),
                description: Some("Hide unstable feature warnings".to_string()),
                kind: SchemaNodeKind::Boolean,
                enum_values: Vec::new(),
                default_value: None,
                scopes: SettingScopeSupport {
                    global: true,
                    profile: true,
                },
            }],
        };
        let config: toml::Value = toml::from_str(
            r#"
[features]
guardian_approval = true
"#,
        )
        .expect("config");
        let origins = HashMap::from([("features.guardian_approval".to_string(), metadata())]);
        let mut effective_features = Features::with_defaults();
        effective_features.set_enabled(Feature::GuardianApproval, true);

        let section_view = build_settings_section_view_data_with_features(
            &schema,
            &config,
            &origins,
            Some(&effective_features),
            None,
            SettingsScope::Global,
            "features",
        );
        let guardian_approval = section_view
            .items
            .iter()
            .find(|item| item.item_key == "features.guardian_approval")
            .expect("missing Guardian Approvals feature");

        assert!(section_view.section_item.is_none());
        assert_eq!(guardian_approval.setting.label, "Guardian Approvals");
        assert_eq!(guardian_approval.setting.display_value, "true");
        assert!(
            guardian_approval
                .setting
                .description
                .as_deref()
                .is_some_and(|description| description.contains("higher-risk actions"))
        );
    }

    #[test]
    fn profile_scope_feature_items_show_inherited_effective_values() {
        let schema = SettingsSchema { nodes: vec![] };
        let config: toml::Value = toml::from_str(
            r#"
[features]
guardian_approval = true
"#,
        )
        .expect("config");
        let origins = HashMap::from([("features.guardian_approval".to_string(), metadata())]);
        let mut effective_features = Features::with_defaults();
        effective_features.set_enabled(Feature::GuardianApproval, true);

        let items = build_setting_items_with_features(
            &schema,
            &config,
            &origins,
            Some(&effective_features),
            Some("dev"),
            SettingsScope::ActiveProfile,
        );
        let guardian_approval = find_setting_item(&items, "features.guardian_approval");

        assert_eq!(guardian_approval.display_value, "true");
        assert_eq!(guardian_approval.category_tag.as_deref(), Some("user"));
        assert!(
            guardian_approval
                .selected_description
                .as_deref()
                .is_some_and(|description| description.contains("Inherited from global config."))
        );
    }
}
