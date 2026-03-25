use anyhow::Result;
use codex_core::config::Config;
use crossterm::event::KeyCode;
use ratatui::style::Stylize;
use ratatui::text::Line;

use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::SelectionAction;
use crate::bottom_pane::SelectionItem;
use crate::bottom_pane::SelectionShortcutAction;
use crate::bottom_pane::SelectionViewParams;
use crate::bottom_pane::custom_prompt_view::CustomPromptView;
use crate::bottom_pane::popup_consts::standard_popup_hint_line;
use crate::key_hint;
use crate::provider_edit::ProviderFieldValue;
use crate::provider_edit::provider_field_groups;
use crate::provider_edit::provider_field_placeholder as provider_edit_placeholder;
use crate::provider_edit::provider_field_value;
use crate::provider_flow::ProviderDetailRuntimeState;
use crate::provider_flow::ProviderDraft;
use crate::provider_flow::ProviderField;
use crate::provider_flow::ProviderFlowData;
use crate::provider_flow::ProviderFlowLocation;
use crate::provider_flow::ProviderFlowNavigation;
use crate::provider_flow::ProviderFlowRow;
use crate::provider_flow::ProviderFlowSource;
use crate::provider_flow::ProviderScreen;
use crate::settings::data::SettingsScope;

pub(crate) const PROVIDER_ROOT_VIEW_ID: &str = "provider.root";
pub(crate) const PROVIDER_DETAIL_VIEW_ID: &str = "provider.detail";
pub(crate) const PROVIDER_SCOPE_VIEW_ID: &str = "provider.scope";
const CLEAR_SENTINEL: &str = "CLEAR";

pub(crate) fn provider_clear_sentinel() -> &'static str {
    CLEAR_SENTINEL
}

pub(crate) fn build_provider_view_params(
    config: &Config,
    create_draft: &crate::provider_flow::ProviderDraft,
    source: ProviderFlowSource,
    scope: SettingsScope,
    screen: &ProviderScreen,
) -> SelectionViewParams {
    let normalized_scope = scope.normalized(config.active_profile.as_deref());
    let mut data = ProviderFlowData::from_config(config, normalized_scope);
    data.create_draft = create_draft.clone();
    match screen {
        ProviderScreen::Root => build_provider_root_view_params(
            &data,
            source,
            normalized_scope,
            config.active_profile.as_deref(),
        ),
        ProviderScreen::Detail { provider_id } => {
            let runtime_state = data.row(provider_id).map(|row| {
                ProviderDetailRuntimeState::from_config(config, provider_id, &row.provider)
            });
            build_provider_detail_view_params(
                &data,
                runtime_state,
                source,
                normalized_scope,
                provider_id,
                config.active_profile.as_deref(),
            )
        }
        ProviderScreen::Create => build_provider_create_view_params(
            &data,
            source,
            normalized_scope,
            config.active_profile.as_deref(),
        ),
    }
}

pub(crate) fn build_provider_scope_picker_params(
    source: ProviderFlowSource,
    current_scope: SettingsScope,
    current_screen: ProviderScreen,
    active_profile: Option<&str>,
) -> SelectionViewParams {
    let current_scope = current_scope.normalized(active_profile);
    let global_screen = current_screen.clone();
    let global_actions: Vec<SelectionAction> = vec![Box::new(move |tx: &AppEventSender| {
        tx.send(AppEvent::OpenProviderFlow {
            source,
            scope: SettingsScope::Global,
            screen: global_screen.clone(),
        });
    })];

    let mut items = vec![SelectionItem {
        name: "User config".to_string(),
        description: Some("Set the default provider in your top-level config.toml.".to_string()),
        selected_description: Some(
            "Writes model_provider to user config.toml. Provider definitions still live in global [model_providers]."
                .to_string(),
        ),
        is_current: current_scope == SettingsScope::Global,
        actions: global_actions,
        dismiss_on_select: true,
        search_value: Some("user config global provider".to_string()),
        ..Default::default()
    }];

    let profile_name = active_profile.map(ToOwned::to_owned);
    let profile_screen = current_screen;
    let profile_actions: Vec<SelectionAction> = vec![Box::new(move |tx: &AppEventSender| {
        tx.send(AppEvent::OpenProviderFlow {
            source,
            scope: SettingsScope::ActiveProfile,
            screen: profile_screen.clone(),
        });
    })];
    items.push(SelectionItem {
        name: "Active profile".to_string(),
        description: Some(match active_profile {
            Some(profile) => format!("Set the default provider under [profiles.{profile}]."),
            None => "No active profile is selected.".to_string(),
        }),
        selected_description: profile_name.as_ref().map(|profile| {
            format!(
                "Writes model_provider under [profiles.{profile}]. Provider definitions still stay global."
            )
        }),
        is_current: current_scope == SettingsScope::ActiveProfile,
        is_disabled: active_profile.is_none(),
        actions: profile_actions,
        dismiss_on_select: true,
        search_value: profile_name.map(|profile| format!("profile {profile}")),
        disabled_reason: active_profile
            .is_none()
            .then_some("No active profile is available.".to_string()),
        ..Default::default()
    });

    SelectionViewParams {
        view_id: Some(PROVIDER_SCOPE_VIEW_ID),
        title: Some("Default Provider Scope".to_string()),
        subtitle: Some("Choose where /provider writes the default provider selection.".to_string()),
        footer_hint: Some(standard_popup_hint_line()),
        items,
        ..Default::default()
    }
}

pub(crate) fn build_provider_field_editor(
    config: &Config,
    create_draft: &ProviderDraft,
    location: ProviderFlowLocation,
    provider_id: Option<String>,
    field: ProviderField,
    app_event_tx: AppEventSender,
) -> Result<CustomPromptView> {
    let title = match &provider_id {
        Some(provider_id) => format!("Edit {} for {provider_id}", field.label()),
        None => format!("Set {}", field.label()),
    };
    let context_label = Some(provider_editor_context_label(
        &location,
        &provider_id,
        config.active_profile.as_deref(),
    ));
    let initial_text =
        provider_field_initial_text(config, create_draft, provider_id.as_deref(), field);
    let submit_tx = app_event_tx;

    Ok(CustomPromptView::new(
        title,
        provider_field_placeholder(field),
        context_label,
        Box::new(move |input: String| match provider_id.as_deref() {
            Some(provider_id) => {
                submit_tx.send(AppEvent::SaveProviderFieldEdit {
                    location: location.clone(),
                    provider_id: provider_id.to_string(),
                    field,
                    value: input.trim().to_string(),
                });
                Ok(())
            }
            None => {
                submit_tx.send(AppEvent::UpdateProviderCreateDraft {
                    field,
                    value: input.trim().to_string(),
                });
                submit_tx.send(AppEvent::OpenProviderFlow {
                    source: location.source,
                    scope: location.scope,
                    screen: ProviderScreen::Create,
                });
                Ok(())
            }
        }),
    )
    .with_initial_text(initial_text))
}

fn build_provider_root_view_params(
    data: &ProviderFlowData,
    source: ProviderFlowSource,
    scope: SettingsScope,
    active_profile: Option<&str>,
) -> SelectionViewParams {
    let mut items = vec![provider_scope_selection_item(
        source,
        scope,
        ProviderScreen::Root,
        active_profile,
    )];
    items.extend(
        data.rows
            .iter()
            .map(|row| provider_row_selection_item(row, source, scope)),
    );
    items.push(create_provider_selection_item(source, scope));

    SelectionViewParams {
        view_id: Some(PROVIDER_ROOT_VIEW_ID),
        title: Some("Provider".to_string()),
        subtitle: Some(provider_root_subtitle(scope, active_profile)),
        footer_note: Some(
            Line::from(
                "Enter sets the current provider. Press e to open details. Provider definitions save globally; only the current provider selection follows the chosen scope."
                    .to_string(),
            )
            .dim(),
        ),
        footer_hint: Some(standard_popup_hint_line()),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to filter providers".to_string()),
        ..Default::default()
    }
}

fn build_provider_detail_view_params(
    data: &ProviderFlowData,
    runtime_state: Option<ProviderDetailRuntimeState>,
    source: ProviderFlowSource,
    scope: SettingsScope,
    provider_id: &str,
    active_profile: Option<&str>,
) -> SelectionViewParams {
    let Some(row) = data.row(provider_id) else {
        return SelectionViewParams {
            view_id: Some(PROVIDER_DETAIL_VIEW_ID),
            title: Some("Provider".to_string()),
            subtitle: Some(format!("Provider `{provider_id}` was not found.")),
            footer_hint: Some(standard_popup_hint_line()),
            items: vec![SelectionItem {
                name: "Back to provider list".to_string(),
                actions: vec![Box::new(move |tx| {
                    tx.send(AppEvent::OpenProviderFlow {
                        source,
                        scope,
                        screen: ProviderScreen::Root,
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            }],
            ..Default::default()
        };
    };
    let runtime_state = runtime_state.unwrap_or_default();

    let location = ProviderFlowLocation {
        source,
        scope,
        screen: ProviderScreen::Detail {
            provider_id: provider_id.to_string(),
        },
    };

    let mut items = vec![provider_scope_selection_item(
        source,
        scope,
        location.screen.clone(),
        active_profile,
    )];
    items.push(default_provider_action_item(row, source, scope));
    for group in provider_field_groups() {
        for field in *group {
            items.push(provider_field_item(
                row,
                &location,
                *field,
                runtime_state.has_secure_api_key,
            ));
        }
    }
    items.push(provider_usage_item(
        row,
        &location,
        runtime_state.can_edit_usage_scripts,
    ));
    items.push(provider_delete_item(row, source, scope));

    SelectionViewParams {
        view_id: Some(PROVIDER_DETAIL_VIEW_ID),
        title: Some(format!("Provider / {provider_id}")),
        subtitle: Some(provider_detail_subtitle(row, scope, active_profile)),
        footer_note: Some(
            Line::from(
                "Usage scripts are project-local. API keys stay in secure storage outside config.toml."
                    .to_string(),
            )
            .dim(),
        ),
        footer_hint: Some(standard_popup_hint_line()),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to filter provider actions".to_string()),
        ..Default::default()
    }
}

fn build_provider_create_view_params(
    data: &ProviderFlowData,
    source: ProviderFlowSource,
    scope: SettingsScope,
    active_profile: Option<&str>,
) -> SelectionViewParams {
    let location = ProviderFlowLocation {
        source,
        scope,
        screen: ProviderScreen::Create,
    };

    let mut items = vec![provider_scope_selection_item(
        source,
        scope,
        ProviderScreen::Create,
        active_profile,
    )];
    for group in provider_field_groups() {
        for field in *group {
            items.push(create_draft_field_item(data, &location, *field));
        }
    }
    items.push(save_create_item(source, scope));

    SelectionViewParams {
        view_id: Some(PROVIDER_DETAIL_VIEW_ID),
        title: Some("Provider / create".to_string()),
        subtitle: Some(
            "Create a custom provider. Definitions save globally; default selection still follows the chosen scope."
                .to_string(),
        ),
        footer_note: Some(
            Line::from(
                "New providers default to wire_api = \"responses\" and requires_openai_auth = true. Hidden values open blank for safety, and typing CLEAR removes an existing secret."
                    .to_string(),
            )
            .dim(),
        ),
        footer_hint: Some(standard_popup_hint_line()),
        items,
        is_searchable: true,
        search_placeholder: Some("Type to filter provider fields".to_string()),
        ..Default::default()
    }
}

fn provider_scope_selection_item(
    source: ProviderFlowSource,
    scope: SettingsScope,
    current_screen: ProviderScreen,
    active_profile: Option<&str>,
) -> SelectionItem {
    SelectionItem {
        name: "Default provider scope".to_string(),
        description: Some(match scope {
            SettingsScope::Global => "Currently writing model_provider to user config.toml.".to_string(),
            SettingsScope::ActiveProfile => match active_profile {
                Some(profile) => format!("Currently writing model_provider under [profiles.{profile}]."),
                None => "No active profile is available; using user config.toml.".to_string(),
            },
        }),
        selected_description: Some(
            "Switch where /provider saves the default provider selection. Provider definitions stay global."
                .to_string(),
        ),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenProviderScopePicker {
                source,
                current_scope: scope,
                current_screen: current_screen.clone(),
            });
        })],
        dismiss_on_select: false,
        search_value: Some("provider scope global profile".to_string()),
        ..Default::default()
    }
}

fn provider_row_selection_item(
    row: &ProviderFlowRow,
    source: ProviderFlowSource,
    scope: SettingsScope,
) -> SelectionItem {
    let provider_id = row.id.clone();
    let detail_screen = ProviderScreen::Detail {
        provider_id: provider_id.clone(),
    };
    let description = provider_row_description(row);
    SelectionItem {
        name: row.id.clone(),
        description: Some(description),
        selected_description: Some(match row.is_default {
            true => format!(
                "{} Current provider for this scope.",
                provider_detail_summary(row)
            ),
            false => format!(
                "{} Available to set as the current provider for this scope.",
                provider_detail_summary(row)
            ),
        }),
        is_current: row.is_default,
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::PersistDefaultModelProvider {
                id: provider_id.clone(),
                scope,
                navigation: ProviderFlowNavigation::ExitFlow,
            });
        })],
        shortcut_action: Some(SelectionShortcutAction {
            binding: key_hint::plain(KeyCode::Char('e')),
            actions: vec![Box::new(move |tx| {
                tx.send(AppEvent::OpenProviderFlow {
                    source,
                    scope,
                    screen: detail_screen.clone(),
                });
            })],
            dismiss_on_select: false,
        }),
        dismiss_on_select: false,
        search_value: Some(format!(
            "{} {} {}",
            row.id,
            row.provider.name,
            row.provider.base_url.as_deref().unwrap_or_default()
        )),
        ..Default::default()
    }
}

fn create_provider_selection_item(
    source: ProviderFlowSource,
    scope: SettingsScope,
) -> SelectionItem {
    SelectionItem {
        name: "Create custom provider".to_string(),
        description: Some("Add a new global [model_providers.<id>] entry.".to_string()),
        selected_description: Some(
            "Create a custom provider definition, then optionally set it as the default for this scope."
                .to_string(),
        ),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenProviderFlow {
                source,
                scope,
                screen: ProviderScreen::Create,
            });
        })],
        dismiss_on_select: false,
        search_value: Some("create custom provider new add".to_string()),
        ..Default::default()
    }
}

fn default_provider_action_item(
    row: &ProviderFlowRow,
    source: ProviderFlowSource,
    scope: SettingsScope,
) -> SelectionItem {
    let provider_id = row.id.clone();
    SelectionItem {
        name: "Set current provider".to_string(),
        description: Some(match row.is_default {
            true => "Already the current provider for this scope.".to_string(),
            false => "Set this as the current provider for the chosen scope.".to_string(),
        }),
        is_current: row.is_default,
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::PersistDefaultModelProvider {
                id: provider_id.clone(),
                scope,
                navigation: ProviderFlowNavigation::ReturnToRoot { source, scope },
            });
        })],
        dismiss_on_select: false,
        search_value: Some("current switch active provider".to_string()),
        ..Default::default()
    }
}

fn provider_field_item(
    row: &ProviderFlowRow,
    location: &ProviderFlowLocation,
    field: ProviderField,
    has_secure_api_key: bool,
) -> SelectionItem {
    let provider_id = row.id.clone();
    let search_provider_id = provider_id.clone();
    let (description, disabled_reason) = provider_field_description(row, field, has_secure_api_key);
    let is_disabled = disabled_reason.is_some();
    let location = location.clone();
    SelectionItem {
        name: field.label().to_string(),
        description: Some(description),
        selected_description: selected_provider_field_description(field, has_secure_api_key),
        is_disabled,
        disabled_reason,
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenProviderFieldEditor {
                location: location.clone(),
                provider_id: Some(provider_id.clone()),
                field,
            });
        })],
        dismiss_on_select: !is_disabled,
        search_value: Some(format!("{} {}", search_provider_id, field.label())),
        ..Default::default()
    }
}

fn provider_usage_item(
    row: &ProviderFlowRow,
    location: &ProviderFlowLocation,
    can_edit_usage_scripts: bool,
) -> SelectionItem {
    let provider_id = row.id.clone();
    let search_provider_id = provider_id.clone();
    let location = location.clone();
    SelectionItem {
        name: "Usage script".to_string(),
        description: Some(if can_edit_usage_scripts {
            "Edit project-local remote usage polling for this provider.".to_string()
        } else {
            "Usage scripts can only be edited inside a trusted project.".to_string()
        }),
        selected_description: Some(
            "Usage scripts live under .codex/providers/<provider-id>/usage.js in the trusted project."
                .to_string(),
        ),
        is_disabled: !can_edit_usage_scripts,
        disabled_reason: (!can_edit_usage_scripts)
            .then_some("Open /provider inside a trusted project to edit usage scripts.".to_string()),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenProviderUsageScriptEditor {
                id: provider_id.clone(),
                return_to: Some(location.clone()),
            });
        })],
        dismiss_on_select: can_edit_usage_scripts,
        search_value: Some(format!("{search_provider_id} usage script remote usage")),
        ..Default::default()
    }
}

fn provider_delete_item(
    row: &ProviderFlowRow,
    source: ProviderFlowSource,
    scope: SettingsScope,
) -> SelectionItem {
    let disabled_reason = if row.is_builtin {
        Some("Built-in providers cannot be deleted.".to_string())
    } else if row.is_default {
        Some("Switch away from the current provider before deleting it.".to_string())
    } else {
        None
    };
    let is_disabled = disabled_reason.is_some();
    let provider_id = row.id.clone();
    let search_provider_id = provider_id.clone();
    SelectionItem {
        name: "Delete provider".to_string(),
        description: Some("Remove this custom provider definition.".to_string()),
        is_disabled,
        disabled_reason,
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::RemoveModelProvider {
                id: provider_id.clone(),
            });
            tx.send(AppEvent::OpenProviderFlow {
                source,
                scope,
                screen: ProviderScreen::Root,
            });
        })],
        dismiss_on_select: !is_disabled,
        search_value: Some(format!("{search_provider_id} delete remove provider")),
        ..Default::default()
    }
}

fn create_draft_field_item(
    data: &ProviderFlowData,
    location: &ProviderFlowLocation,
    field: ProviderField,
) -> SelectionItem {
    let location = location.clone();
    SelectionItem {
        name: field.label().to_string(),
        description: Some(create_field_description(data, field)),
        selected_description: Some(create_field_selected_description(field)),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenProviderFieldEditor {
                location: location.clone(),
                provider_id: None,
                field,
            });
        })],
        dismiss_on_select: true,
        search_value: Some(format!("create provider {}", field.label())),
        ..Default::default()
    }
}

fn save_create_item(source: ProviderFlowSource, scope: SettingsScope) -> SelectionItem {
    SelectionItem {
        name: "Save provider".to_string(),
        description: Some("Validate and persist this custom provider.".to_string()),
        selected_description: Some(
            "Creates a new global provider definition, stores any provided API key securely, and returns to the new provider detail page."
                .to_string(),
        ),
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::SaveProviderCreateDraft { source, scope });
        })],
        dismiss_on_select: true,
        search_value: Some("save provider create persist".to_string()),
        ..Default::default()
    }
}

fn provider_root_subtitle(scope: SettingsScope, active_profile: Option<&str>) -> String {
    match scope {
        SettingsScope::Global => {
            "Browse provider definitions and set the default provider for user config.".to_string()
        }
        SettingsScope::ActiveProfile => match active_profile {
            Some(profile) => format!(
                "Browse provider definitions and set the default provider for active profile `{profile}`."
            ),
            None => "Browse provider definitions and set the default provider for user config."
                .to_string(),
        },
    }
}

fn provider_detail_subtitle(
    row: &ProviderFlowRow,
    scope: SettingsScope,
    active_profile: Option<&str>,
) -> String {
    let scope_text = match scope {
        SettingsScope::Global => "user config".to_string(),
        SettingsScope::ActiveProfile => active_profile
            .map(|profile| format!("profile `{profile}`"))
            .unwrap_or_else(|| "user config".to_string()),
    };
    format!(
        "{} Default selection writes to {scope_text}. Definition edits stay global.",
        provider_detail_summary(row)
    )
}

fn provider_row_description(row: &ProviderFlowRow) -> String {
    let kind = if row.is_builtin { "built-in" } else { "custom" };
    let current = if row.is_default { ", current" } else { "" };
    let base_url = row.provider.base_url.as_deref().unwrap_or("<unset>");
    format!("{kind}{current} · {} · {base_url}", row.provider.name)
}

fn provider_detail_summary(row: &ProviderFlowRow) -> String {
    let mut parts = vec![row.provider.name.clone()];
    if let Some(base_url) = row.provider.base_url.as_ref() {
        parts.push(base_url.clone());
    }
    if row.is_builtin {
        parts.push("built-in".to_string());
    } else {
        parts.push("custom".to_string());
    }
    parts.join(" · ")
}

fn provider_field_description(
    row: &ProviderFlowRow,
    field: ProviderField,
    has_secure_api_key: bool,
) -> (String, Option<String>) {
    let value = provider_field_value(&row.id, &row.provider, field, has_secure_api_key);
    let description = match value {
        ProviderFieldValue::Visible(value) => {
            if value.trim().is_empty() {
                "Current: <unset>".to_string()
            } else {
                format!("Current: {value}")
            }
        }
        ProviderFieldValue::Hidden { current_status, .. } => current_status,
    };
    let disabled_reason = row.is_builtin.then(|| match field {
        ProviderField::ApiKey => {
            "Built-in providers use their built-in auth flow and do not support editing API keys here.".to_string()
        }
        _ => "Built-in providers cannot be edited. Create a custom provider instead.".to_string(),
    });
    (description, disabled_reason)
}

fn selected_provider_field_description(
    field: ProviderField,
    has_secure_api_key: bool,
) -> Option<String> {
    Some(match field {
        ProviderField::Id => {
            "Edit the provider ID. Renaming will migrate secure credentials and update saved default-provider references."
                .to_string()
        }
        ProviderField::Name => "Edit the human-readable provider name stored in config.toml.".to_string(),
        ProviderField::BaseUrl => "Edit the provider base URL stored in config.toml.".to_string(),
        ProviderField::ApiKey => {
            let mut text =
                "Leave the field blank to keep the existing secure value. Type CLEAR to remove it.".to_string();
            if has_secure_api_key {
                text.push_str(" A secure key is already present.");
            }
            text
        }
        ProviderField::WireApi => "Edit the wire_api value stored in config.toml.".to_string(),
        ProviderField::RequiresOpenAiAuth => {
            "Edit whether this provider requires OpenAI auth to be available.".to_string()
        }
        ProviderField::AuthStrategy => "Edit the explicit auth_strategy stored in config.toml.".to_string(),
        ProviderField::OAuth => "Edit the oauth TOML object stored in config.toml.".to_string(),
        ProviderField::EnvKey => "Edit the environment variable name used to load an API key.".to_string(),
        ProviderField::EnvKeyInstructions => {
            "Edit the help text shown when the environment variable is missing.".to_string()
        }
        ProviderField::ExperimentalBearerToken => {
            "Edit the inline bearer token stored in config.toml. Leave blank to keep it, or type CLEAR to remove it.".to_string()
        }
        ProviderField::QueryParams => "Edit the query_params TOML map.".to_string(),
        ProviderField::HttpHeaders => "Edit the http_headers TOML map.".to_string(),
        ProviderField::EnvHttpHeaders => "Edit the env_http_headers TOML map.".to_string(),
        ProviderField::RequestMaxRetries => "Edit the maximum number of request retries.".to_string(),
        ProviderField::StreamMaxRetries => "Edit the maximum number of stream retries.".to_string(),
        ProviderField::StreamIdleTimeoutMs => "Edit the stream idle timeout in milliseconds.".to_string(),
        ProviderField::SupportsWebsockets => {
            "Edit whether this provider can use the Responses API websocket transport.".to_string()
        }
    })
}

fn create_field_description(data: &ProviderFlowData, field: ProviderField) -> String {
    let value = data.create_field_value(field).trim();
    match field {
        ProviderField::ApiKey | ProviderField::ExperimentalBearerToken => {
            if value.is_empty() {
                format!("No {} will be stored yet.", field.label())
            } else {
                format!("A value will be stored for {}.", field.label())
            }
        }
        _ => {
            if value.is_empty() {
                "<unset>".to_string()
            } else {
                format!("Current draft: {value}")
            }
        }
    }
}

fn create_field_selected_description(field: ProviderField) -> String {
    match field {
        ProviderField::Id => "Set the new provider ID. It must be globally unique.".to_string(),
        ProviderField::Name => "Set the display name shown in the UI.".to_string(),
        ProviderField::BaseUrl => "Set the provider base URL used for requests.".to_string(),
        ProviderField::ApiKey => {
            "Optionally store a secure API key outside config.toml while creating this provider."
                .to_string()
        }
        ProviderField::WireApi => {
            "Defaults to responses. Leave it as-is unless you have a provider-specific reason to change it."
                .to_string()
        }
        ProviderField::RequiresOpenAiAuth => {
            "Defaults to true for new providers. Enter false if this provider should not use OpenAI auth."
                .to_string()
        }
        ProviderField::AuthStrategy => {
            "Optional explicit auth strategy. Leave blank to keep the provider definition minimal."
                .to_string()
        }
        ProviderField::OAuth => "Optional TOML object for oauth configuration.".to_string(),
        ProviderField::EnvKey => "Optional environment variable name for API key lookup.".to_string(),
        ProviderField::EnvKeyInstructions => {
            "Optional user-facing help for obtaining or setting the env var.".to_string()
        }
        ProviderField::ExperimentalBearerToken => {
            "Optional inline bearer token. This is stored in config.toml, so leave it blank unless you explicitly want that."
                .to_string()
        }
        ProviderField::QueryParams => "Optional TOML map of query parameters.".to_string(),
        ProviderField::HttpHeaders => "Optional TOML map of static HTTP headers.".to_string(),
        ProviderField::EnvHttpHeaders => {
            "Optional TOML map of header names to environment variable names.".to_string()
        }
        ProviderField::RequestMaxRetries => "Optional request retry count.".to_string(),
        ProviderField::StreamMaxRetries => "Optional stream retry count.".to_string(),
        ProviderField::StreamIdleTimeoutMs => "Optional stream idle timeout in milliseconds.".to_string(),
        ProviderField::SupportsWebsockets => "Optional websocket support flag.".to_string(),
    }
}

fn provider_field_initial_text(
    config: &Config,
    create_draft: &ProviderDraft,
    provider_id: Option<&str>,
    field: ProviderField,
) -> String {
    match provider_id {
        Some(provider_id) => config
            .model_providers
            .get(provider_id)
            .map(|provider| {
                provider_field_value(
                    provider_id,
                    provider,
                    field,
                    provider.inline_api_key().is_some(),
                )
                .initial_text()
            })
            .unwrap_or_else(|| {
                if field == ProviderField::Id {
                    provider_id.to_string()
                } else {
                    String::new()
                }
            }),
        None => create_draft.field_value(field).to_string(),
    }
}

fn provider_field_placeholder(field: ProviderField) -> String {
    let value = match field {
        ProviderField::ApiKey => Some(provider_field_value(
            "",
            &crate::provider_edit::default_create_provider(),
            field,
            false,
        )),
        ProviderField::ExperimentalBearerToken => Some(provider_field_value(
            "",
            &crate::provider_edit::default_create_provider(),
            field,
            false,
        )),
        _ => None,
    };

    if let Some(ProviderFieldValue::Hidden { placeholder, .. }) = value {
        placeholder
    } else {
        provider_edit_placeholder(field)
    }
}

fn provider_editor_context_label(
    location: &ProviderFlowLocation,
    provider_id: &Option<String>,
    active_profile: Option<&str>,
) -> String {
    let scope_label = match location.scope {
        SettingsScope::Global => "Default scope: user config".to_string(),
        SettingsScope::ActiveProfile => match active_profile {
            Some(profile) => format!("Default scope: profile `{profile}`"),
            None => "Default scope: user config".to_string(),
        },
    };
    let target = provider_id
        .as_ref()
        .map(|provider_id| format!("Provider: `{provider_id}`"))
        .unwrap_or_else(|| "Create custom provider".to_string());
    format!("{target} | {scope_label}")
}

#[cfg(test)]
mod tests {
    use super::build_provider_scope_picker_params;
    use super::build_provider_view_params;
    use super::provider_field_initial_text;
    use crate::app_event::AppEvent;
    use crate::app_event_sender::AppEventSender;
    use crate::provider_flow::ProviderDraft;
    use crate::provider_flow::ProviderField;
    use crate::provider_flow::ProviderFlowNavigation;
    use crate::provider_flow::ProviderFlowSource;
    use crate::provider_flow::ProviderScreen;
    use crate::settings::data::SettingsScope;
    use codex_core::config::Config;
    use codex_core::config::ConfigBuilder;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use insta::assert_snapshot;
    use pretty_assertions::assert_eq;
    use tempfile::tempdir;
    use tokio::sync::mpsc::unbounded_channel;

    fn scope_picker_summary(params: &crate::bottom_pane::SelectionViewParams) -> String {
        let mut lines = vec![
            format!("title: {}", params.title.as_deref().unwrap_or_default()),
            format!(
                "subtitle: {}",
                params.subtitle.as_deref().unwrap_or_default()
            ),
        ];
        for item in &params.items {
            lines.push(format!("item: {}", item.name));
            lines.push(format!(
                "  description: {}",
                item.description.as_deref().unwrap_or_default()
            ));
            lines.push(format!("  current: {}", item.is_current));
            lines.push(format!("  disabled: {}", item.is_disabled));
            lines.push(format!(
                "  disabled_reason: {}",
                item.disabled_reason.as_deref().unwrap_or_default()
            ));
        }
        lines.join("\n")
    }

    fn provider_view_focus_summary(params: &crate::bottom_pane::SelectionViewParams) -> String {
        let mut lines = vec![
            format!("title: {}", params.title.as_deref().unwrap_or_default()),
            format!(
                "subtitle: {}",
                params.subtitle.as_deref().unwrap_or_default()
            ),
        ];
        for item in &params.items {
            lines.push(format!("item: {}", item.name));
            lines.push(format!("  current: {}", item.is_current));
            lines.push(format!("  disabled: {}", item.is_disabled));
            lines.push(format!(
                "  selected_description: {}",
                item.selected_description.as_deref().unwrap_or_default()
            ));
            lines.push(format!(
                "  shortcut: {}",
                item.shortcut_action
                    .as_ref()
                    .map(|shortcut| format!("{:?}", shortcut.binding))
                    .unwrap_or_default()
            ));
            lines.push(format!(
                "  description: {}",
                item.description.as_deref().unwrap_or_default()
            ));
            lines.push(format!(
                "  disabled_reason: {}",
                item.disabled_reason.as_deref().unwrap_or_default()
            ));
        }
        lines.join("\n")
    }

    async fn provider_test_config(active_profile: Option<&str>) -> Config {
        let codex_home = std::env::temp_dir();
        let mut config = ConfigBuilder::default()
            .codex_home(codex_home)
            .build()
            .await
            .expect("config");
        let mut acme = config.model_provider.clone();
        acme.name = "Acme".to_string();
        acme.base_url = Some("https://acme.example/v1".to_string());
        acme.requires_openai_auth = false;
        config.model_providers.insert("acme".to_string(), acme);
        let temp = tempdir().expect("tempdir");
        let config_toml_path =
            AbsolutePathBuf::try_from(temp.path().join("config.toml")).expect("absolute path");
        let user_config = toml::from_str(
            r#"
model_provider = "openai"

[model_providers.acme]
name = "Acme"
base_url = "https://acme.example/v1"
wire_api = "responses"

[profiles.dev]
model_provider = "acme"
"#,
        )
        .expect("config");
        config.config_layer_stack = config
            .config_layer_stack
            .with_user_config(&config_toml_path, user_config);
        config.active_profile = active_profile.map(ToOwned::to_owned);
        config
    }

    #[test]
    fn provider_scope_picker_disables_missing_profile() {
        let params = build_provider_scope_picker_params(
            ProviderFlowSource::SlashCommand,
            SettingsScope::Global,
            ProviderScreen::Root,
            None,
        );
        assert_eq!(params.items.len(), 2);
        assert!(params.items[1].is_disabled);
    }

    #[test]
    fn provider_scope_picker_snapshot() {
        let params = build_provider_scope_picker_params(
            ProviderFlowSource::SettingsModel,
            SettingsScope::ActiveProfile,
            ProviderScreen::Detail {
                provider_id: "acme".to_string(),
            },
            Some("dev"),
        );
        assert_snapshot!("provider_scope_picker", scope_picker_summary(&params));
    }

    #[tokio::test]
    async fn provider_root_snapshot_marks_scoped_default() {
        let config = provider_test_config(Some("dev")).await;
        let params = build_provider_view_params(
            &config,
            &ProviderDraft::new(),
            ProviderFlowSource::SlashCommand,
            SettingsScope::ActiveProfile,
            &ProviderScreen::Root,
        );

        assert_snapshot!(
            "provider_root_profile",
            provider_view_focus_summary(&params)
        );
    }

    #[tokio::test]
    async fn provider_create_snapshot_lists_defaults_and_advanced_fields() {
        let config = provider_test_config(Some("dev")).await;
        let params = build_provider_view_params(
            &config,
            &ProviderDraft::new(),
            ProviderFlowSource::SlashCommand,
            SettingsScope::ActiveProfile,
            &ProviderScreen::Create,
        );

        assert_snapshot!(
            "provider_create_profile",
            provider_view_focus_summary(&params)
        );
    }

    #[tokio::test]
    async fn provider_root_footer_note_mentions_e_shortcut() {
        let config = provider_test_config(Some("dev")).await;
        let params = build_provider_view_params(
            &config,
            &ProviderDraft::new(),
            ProviderFlowSource::SlashCommand,
            SettingsScope::ActiveProfile,
            &ProviderScreen::Root,
        );

        assert_eq!(
            params.footer_note.as_ref().map(ToString::to_string),
            Some(
                "Enter sets the current provider. Press e to open details. Provider definitions save globally; only the current provider selection follows the chosen scope."
                    .to_string()
            )
        );
    }

    #[tokio::test]
    async fn provider_detail_snapshot_lists_all_provider_fields() {
        let config = provider_test_config(Some("dev")).await;
        let params = build_provider_view_params(
            &config,
            &ProviderDraft::new(),
            ProviderFlowSource::SettingsModel,
            SettingsScope::ActiveProfile,
            &ProviderScreen::Detail {
                provider_id: "acme".to_string(),
            },
        );

        assert_snapshot!(
            "provider_detail_profile",
            provider_view_focus_summary(&params)
        );
    }

    #[tokio::test]
    async fn provider_detail_marks_present_api_key() {
        let mut config = provider_test_config(Some("dev")).await;
        config
            .model_providers
            .get_mut("acme")
            .expect("acme provider should exist")
            .api_key = Some("secret-inline".to_string());
        let params = build_provider_view_params(
            &config,
            &ProviderDraft::new(),
            ProviderFlowSource::SettingsModel,
            SettingsScope::ActiveProfile,
            &ProviderScreen::Detail {
                provider_id: "acme".to_string(),
            },
        );
        let item = params
            .items
            .iter()
            .find(|item| item.name == "API key")
            .expect("API key item should exist");

        assert_eq!(
            item.description.as_deref(),
            Some("A secure API key is already stored for this provider.")
        );
        assert_eq!(
            item.selected_description.as_deref(),
            Some(
                "Leave the field blank to keep the existing secure value. Type CLEAR to remove it. A secure key is already present."
            )
        );
    }

    #[tokio::test]
    async fn provider_field_initial_text_prefills_existing_provider_values() {
        let config = provider_test_config(Some("dev")).await;

        assert_eq!(
            provider_field_initial_text(
                &config,
                &ProviderDraft::new(),
                Some("acme"),
                ProviderField::Name,
            ),
            "Acme"
        );
        assert_eq!(
            provider_field_initial_text(
                &config,
                &ProviderDraft::new(),
                Some("acme"),
                ProviderField::BaseUrl,
            ),
            "https://acme.example/v1"
        );
        assert_eq!(
            provider_field_initial_text(
                &config,
                &ProviderDraft::new(),
                Some("acme"),
                ProviderField::WireApi,
            ),
            "responses"
        );
        assert_eq!(
            provider_field_initial_text(
                &config,
                &ProviderDraft::new(),
                Some("acme"),
                ProviderField::RequiresOpenAiAuth,
            ),
            "false"
        );
    }

    #[tokio::test]
    async fn provider_field_initial_text_uses_create_draft() {
        let mut draft = ProviderDraft::new();
        draft.update_field(ProviderField::Name, "Draft provider".to_string());
        draft.update_field(
            ProviderField::BaseUrl,
            "https://draft.example/v1".to_string(),
        );
        let config = provider_test_config(Some("dev")).await;

        assert_eq!(
            provider_field_initial_text(&config, &draft, None, ProviderField::Name),
            "Draft provider"
        );
        assert_eq!(
            provider_field_initial_text(&config, &draft, None, ProviderField::BaseUrl),
            "https://draft.example/v1"
        );
        assert_eq!(
            provider_field_initial_text(&config, &draft, None, ProviderField::WireApi),
            "responses"
        );
        assert_eq!(
            provider_field_initial_text(&config, &draft, None, ProviderField::RequiresOpenAiAuth,),
            "true"
        );
        assert_eq!(
            provider_field_initial_text(&config, &draft, None, ProviderField::SupportsWebsockets),
            ""
        );
    }

    #[tokio::test]
    async fn provider_scope_picker_returns_to_current_screen() {
        let config = provider_test_config(Some("dev")).await;
        let params = build_provider_view_params(
            &config,
            &ProviderDraft::new(),
            ProviderFlowSource::SlashCommand,
            SettingsScope::ActiveProfile,
            &ProviderScreen::Detail {
                provider_id: "acme".to_string(),
            },
        );
        let scope_item = params.items.first().expect("scope item should be first");
        let action = scope_item
            .actions
            .first()
            .expect("scope item should have an action");
        let (tx_raw, mut rx) = unbounded_channel();
        action(&AppEventSender::new(tx_raw));

        let event = rx.try_recv().expect("expected scope picker event");
        let AppEvent::OpenProviderScopePicker {
            source,
            current_scope,
            current_screen,
        } = event
        else {
            panic!("expected OpenProviderScopePicker event");
        };
        assert_eq!(source, ProviderFlowSource::SlashCommand);
        assert_eq!(current_scope, SettingsScope::ActiveProfile);
        assert_eq!(
            current_screen,
            ProviderScreen::Detail {
                provider_id: "acme".to_string()
            }
        );
    }

    #[tokio::test]
    async fn provider_root_row_sets_current_provider() {
        let config = provider_test_config(Some("dev")).await;
        let params = build_provider_view_params(
            &config,
            &ProviderDraft::new(),
            ProviderFlowSource::SlashCommand,
            SettingsScope::ActiveProfile,
            &ProviderScreen::Root,
        );
        let item = params
            .items
            .iter()
            .find(|item| item.name == "openai")
            .expect("openai item should exist");
        let action = item
            .actions
            .first()
            .expect("provider row should have an action");
        let (tx_raw, mut rx) = unbounded_channel();
        action(&AppEventSender::new(tx_raw));

        let event = rx.try_recv().expect("expected persist event");
        let AppEvent::PersistDefaultModelProvider {
            id,
            scope,
            navigation,
        } = event
        else {
            panic!("expected PersistDefaultModelProvider event");
        };
        assert_eq!(id, "openai");
        assert_eq!(scope, SettingsScope::ActiveProfile);
        assert_eq!(navigation, ProviderFlowNavigation::ExitFlow);
    }

    #[tokio::test]
    async fn provider_root_row_shortcut_opens_detail() {
        let config = provider_test_config(Some("dev")).await;
        let params = build_provider_view_params(
            &config,
            &ProviderDraft::new(),
            ProviderFlowSource::SlashCommand,
            SettingsScope::ActiveProfile,
            &ProviderScreen::Root,
        );
        let item = params
            .items
            .iter()
            .find(|item| item.name == "openai")
            .expect("openai item should exist");
        let shortcut = item
            .shortcut_action
            .as_ref()
            .expect("provider row should have a shortcut action");
        assert_eq!(
            shortcut.binding,
            crate::key_hint::plain(crossterm::event::KeyCode::Char('e'))
        );
        let action = shortcut
            .actions
            .first()
            .expect("shortcut should have an action");
        let (tx_raw, mut rx) = unbounded_channel();
        action(&AppEventSender::new(tx_raw));

        let event = rx.try_recv().expect("expected open detail event");
        let AppEvent::OpenProviderFlow {
            source,
            scope,
            screen,
        } = event
        else {
            panic!("expected OpenProviderFlow event");
        };
        assert_eq!(source, ProviderFlowSource::SlashCommand);
        assert_eq!(scope, SettingsScope::ActiveProfile);
        assert_eq!(
            screen,
            ProviderScreen::Detail {
                provider_id: "openai".to_string()
            }
        );
    }

    #[tokio::test]
    async fn provider_detail_set_current_returns_to_root() {
        let config = provider_test_config(Some("dev")).await;
        let params = build_provider_view_params(
            &config,
            &ProviderDraft::new(),
            ProviderFlowSource::SlashCommand,
            SettingsScope::ActiveProfile,
            &ProviderScreen::Detail {
                provider_id: "openai".to_string(),
            },
        );
        let item = params
            .items
            .iter()
            .find(|item| item.name == "Set current provider")
            .expect("detail screen should expose current-provider action");
        let action = item
            .actions
            .first()
            .expect("current-provider action should exist");
        let (tx_raw, mut rx) = unbounded_channel();
        action(&AppEventSender::new(tx_raw));

        let event = rx.try_recv().expect("expected persist event");
        let AppEvent::PersistDefaultModelProvider {
            id,
            scope,
            navigation,
        } = event
        else {
            panic!("expected PersistDefaultModelProvider event");
        };
        assert_eq!(id, "openai");
        assert_eq!(scope, SettingsScope::ActiveProfile);
        assert_eq!(
            navigation,
            ProviderFlowNavigation::ReturnToRoot {
                source: ProviderFlowSource::SlashCommand,
                scope: SettingsScope::ActiveProfile,
            }
        );
    }

    #[tokio::test]
    async fn settings_model_provider_opens_provider_flow() {
        let config = provider_test_config(Some("dev")).await;
        let params = crate::settings::view::build_settings_view_params(
            &config,
            SettingsScope::ActiveProfile,
            &crate::settings::data::SettingsScreen::Section {
                section_key: "model".to_string(),
            },
            None,
        )
        .expect("settings view");
        let item = params
            .items
            .iter()
            .find(|item| item.name == "provider")
            .expect("provider item should exist");
        let action = item
            .actions
            .first()
            .expect("provider item should be actionable");
        let (tx_raw, mut rx) = unbounded_channel();
        action(&AppEventSender::new(tx_raw));

        let event = rx.try_recv().expect("expected provider flow event");
        let AppEvent::OpenProviderFlow {
            source,
            scope,
            screen,
        } = event
        else {
            panic!("expected OpenProviderFlow event");
        };
        assert_eq!(source, ProviderFlowSource::SettingsModel);
        assert_eq!(scope, SettingsScope::ActiveProfile);
        assert_eq!(screen, ProviderScreen::Root);
    }
}
