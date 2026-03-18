use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;

use codex_core::ModelProviderAuthStrategy;
use codex_core::ModelProviderInfo;
use codex_core::WireApi;
use codex_core::built_in_model_providers;
use codex_core::config::Config;
use codex_core::read_provider_api_key;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::StatefulWidgetRef;
use ratatui::widgets::Widget;
use ratatui::widgets::Wrap;

use crate::app_event::AppEvent;
use crate::app_event::ProviderApiKeyInput;
use crate::app_event_sender::AppEventSender;
use crate::provider_usage::can_edit_provider_usage_scripts;
use crate::render::renderable::ColumnRenderable;
use crate::render::renderable::Renderable;
use crate::selection_list::selection_option_row;

use super::BottomPaneView;
use super::CancellationEvent;
use super::textarea::TextArea;
use super::textarea::TextAreaState;

#[derive(Clone)]
struct ProviderRow {
    id: String,
    provider: ModelProviderInfo,
    is_builtin: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Id,
    Name,
    BaseUrl,
    ApiKey,
}

impl Field {
    fn next(self) -> Self {
        match self {
            Self::Id => Self::Name,
            Self::Name => Self::BaseUrl,
            Self::BaseUrl => Self::ApiKey,
            Self::ApiKey => Self::Id,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Id => Self::ApiKey,
            Self::Name => Self::Id,
            Self::BaseUrl => Self::Name,
            Self::ApiKey => Self::BaseUrl,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Id => "Provider ID",
            Self::Name => "Display name",
            Self::BaseUrl => "Base URL",
            Self::ApiKey => "API key",
        }
    }
}

#[derive(Clone)]
struct ProviderDraft {
    original_id: Option<String>,
    template: ModelProviderInfo,
    id: String,
    name: String,
    base_url: String,
    api_key: String,
    existing_api_key: bool,
    focused_field: Field,
}

impl ProviderDraft {
    fn new() -> Self {
        Self {
            original_id: None,
            template: ModelProviderInfo {
                name: String::new(),
                base_url: None,
                auth_strategy: ModelProviderAuthStrategy::ApiKey,
                oauth: None,
                api_key: None,
                env_key: None,
                env_key_instructions: None,
                experimental_bearer_token: None,
                wire_api: WireApi::Responses,
                query_params: None,
                http_headers: None,
                env_http_headers: None,
                request_max_retries: None,
                stream_max_retries: None,
                stream_idle_timeout_ms: None,
                requires_openai_auth: false,
                supports_websockets: false,
            },
            id: String::new(),
            name: String::new(),
            base_url: String::new(),
            api_key: String::new(),
            existing_api_key: false,
            focused_field: Field::Id,
        }
    }

    fn from_row(row: &ProviderRow, existing_api_key: bool) -> Self {
        Self {
            original_id: Some(row.id.clone()),
            template: row.provider.clone(),
            id: row.id.clone(),
            name: row.provider.name.clone(),
            base_url: row.provider.base_url.clone().unwrap_or_default(),
            api_key: String::new(),
            existing_api_key,
            focused_field: Field::Name,
        }
    }

    fn current_field_value(&self) -> &str {
        match self.focused_field {
            Field::Id => &self.id,
            Field::Name => &self.name,
            Field::BaseUrl => &self.base_url,
            Field::ApiKey => &self.api_key,
        }
    }

    fn set_current_field_value(&mut self, value: String) {
        match self.focused_field {
            Field::Id => self.id = value,
            Field::Name => self.name = value,
            Field::BaseUrl => self.base_url = value,
            Field::ApiKey => self.api_key = value,
        }
    }

    fn to_provider(&self) -> ModelProviderInfo {
        let mut provider = self.template.clone();
        provider.name = self.name.trim().to_string();
        provider.base_url =
            Some(self.base_url.trim().to_string()).filter(|value| !value.is_empty());
        provider.auth_strategy = ModelProviderAuthStrategy::ApiKey;
        provider.oauth = None;
        provider.api_key = None;
        provider.requires_openai_auth = false;
        provider
    }

    fn api_key_input(&self) -> ProviderApiKeyInput {
        let api_key = self.api_key.trim();
        if api_key.eq_ignore_ascii_case("clear") {
            ProviderApiKeyInput::Clear
        } else if !api_key.is_empty() {
            ProviderApiKeyInput::Set(api_key.to_string())
        } else {
            ProviderApiKeyInput::KeepExisting
        }
    }
}

struct EditState {
    draft: ProviderDraft,
    textarea: TextArea,
    textarea_state: RefCell<TextAreaState>,
}

enum Mode {
    List,
    Edit(Box<EditState>),
}

pub(crate) struct ProviderManagerView {
    app_event_tx: AppEventSender,
    codex_home: PathBuf,
    rows: Vec<ProviderRow>,
    selected_idx: usize,
    default_provider_id: String,
    builtin_ids: HashSet<String>,
    can_edit_usage_scripts: bool,
    complete: bool,
    error: Option<String>,
    mode: Mode,
}

impl ProviderManagerView {
    pub(crate) fn new(config: &Config, app_event_tx: AppEventSender) -> Self {
        let builtin_ids: HashSet<String> = built_in_model_providers(/*openai_base_url*/ None)
            .keys()
            .cloned()
            .collect();
        let mut rows: Vec<ProviderRow> = config
            .model_providers
            .iter()
            .map(
                |(id, provider): (&String, &ModelProviderInfo)| ProviderRow {
                    id: id.clone(),
                    provider: provider.clone(),
                    is_builtin: builtin_ids.contains(id),
                },
            )
            .collect();
        rows.sort_by(|left, right| left.id.cmp(&right.id));
        let selected_idx = rows
            .iter()
            .position(|row| row.id == config.model_provider_id)
            .unwrap_or(0);

        Self {
            app_event_tx,
            codex_home: config.codex_home.clone(),
            rows,
            selected_idx,
            default_provider_id: config.model_provider_id.clone(),
            builtin_ids,
            can_edit_usage_scripts: can_edit_provider_usage_scripts(config),
            complete: false,
            error: None,
            mode: Mode::List,
        }
    }

    fn selected_row(&self) -> Option<&ProviderRow> {
        self.rows.get(self.selected_idx)
    }

    fn move_selection(&mut self, delta: isize) {
        let len = self.rows.len();
        if len == 0 {
            self.selected_idx = 0;
            return;
        }
        self.selected_idx = (self.selected_idx as isize + delta).rem_euclid(len as isize) as usize;
    }

    fn begin_edit(&mut self, draft: ProviderDraft) {
        let mut textarea = TextArea::new();
        textarea.set_text_clearing_elements(draft.current_field_value());
        self.mode = Mode::Edit(Box::new(EditState {
            draft,
            textarea,
            textarea_state: RefCell::new(TextAreaState::default()),
        }));
    }

    fn start_new(&mut self) {
        self.begin_edit(ProviderDraft::new());
    }

    fn start_edit(&mut self) {
        let Some(row) = self.selected_row().cloned() else {
            return;
        };
        if row.is_builtin {
            self.error = Some(
                "Built-in providers cannot be edited. Create a custom provider instead."
                    .to_string(),
            );
            return;
        }
        let existing_api_key = read_provider_api_key(&self.codex_home, &row.id)
            .ok()
            .flatten()
            .is_some()
            || row.provider.inline_api_key().is_some();
        self.begin_edit(ProviderDraft::from_row(&row, existing_api_key));
    }

    fn sync_draft_from_textarea(edit: &mut EditState) {
        edit.draft
            .set_current_field_value(edit.textarea.text().to_string());
    }

    fn sync_textarea_from_draft(edit: &mut EditState) {
        edit.textarea
            .set_text_clearing_elements(edit.draft.current_field_value());
    }

    fn save_current_draft(&mut self) {
        let Mode::Edit(edit) = &mut self.mode else {
            return;
        };
        Self::sync_draft_from_textarea(edit);

        let provider_id = edit.draft.id.trim().to_string();
        if provider_id.is_empty() {
            self.error = Some("Provider ID is required.".to_string());
            return;
        }
        if !provider_id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
        {
            self.error =
                Some("Provider ID must use lowercase letters, digits, '-' or '_'.".to_string());
            return;
        }
        if edit.draft.name.trim().is_empty() {
            self.error = Some("Display name is required.".to_string());
            return;
        }
        if edit.draft.base_url.trim().is_empty() {
            self.error = Some("Base URL is required.".to_string());
            return;
        }
        if self.builtin_ids.contains(&provider_id)
            && edit.draft.original_id.as_deref() != Some(provider_id.as_str())
        {
            self.error = Some("Provider ID collides with a built-in provider.".to_string());
            return;
        }
        if self.rows.iter().any(|row| row.id == provider_id)
            && edit.draft.original_id.as_deref() != Some(provider_id.as_str())
        {
            self.error = Some("Provider ID already exists.".to_string());
            return;
        }

        if let Some(original_id) = edit.draft.original_id.as_deref()
            && original_id != provider_id
        {
            if original_id == self.default_provider_id {
                self.error = Some(
                    "Rename is disabled for the current default provider. Switch defaults first."
                        .to_string(),
                );
                return;
            }
            self.app_event_tx.send(AppEvent::RemoveModelProvider {
                id: original_id.to_string(),
            });
        }

        self.app_event_tx.send(AppEvent::PersistModelProvider {
            original_id: edit.draft.original_id.clone(),
            id: provider_id,
            provider: edit.draft.to_provider(),
            api_key_input: edit.draft.api_key_input(),
        });
        self.complete = true;
    }

    fn delete_selected(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        if row.is_builtin {
            self.error = Some("Built-in providers cannot be deleted.".to_string());
            return;
        }
        if row.id == self.default_provider_id {
            self.error = Some(
                "Switch away from the current default provider before deleting it.".to_string(),
            );
            return;
        }
        self.app_event_tx
            .send(AppEvent::RemoveModelProvider { id: row.id.clone() });
        self.complete = true;
    }

    fn activate_selected(&mut self) {
        if let Some(row) = self.selected_row() {
            self.app_event_tx
                .send(AppEvent::PersistDefaultModelProvider { id: row.id.clone() });
            self.complete = true;
        }
    }

    fn edit_usage_script(&mut self) {
        let Some(row) = self.selected_row() else {
            return;
        };
        if !self.can_edit_usage_scripts {
            self.error =
                Some("Usage scripts can only be edited inside a trusted project.".to_string());
            return;
        }
        self.app_event_tx
            .send(AppEvent::OpenProviderUsageScriptEditor { id: row.id.clone() });
        self.complete = true;
    }

    fn render_list(&self, area: Rect, buf: &mut Buffer) {
        let mut column = ColumnRenderable::new();
        column.push(Line::from("Manage providers".bold()));
        column.push(Line::from(
            "Enter switches default provider. n creates. e edits custom. u edits usage script. d deletes custom.".dim(),
        ));
        column.push(Line::from(
            "Editing only saves config. It does not switch the default provider.".dim(),
        ));
        column.push(Line::from(
            "API keys are stored securely outside config.toml.".dim(),
        ));
        column.push("");

        for (idx, row) in self.rows.iter().enumerate() {
            let mut label = format!("{} ({})", row.provider.name, row.id);
            if row.id == self.default_provider_id {
                label.push_str(" [default]");
            }
            if row.is_builtin {
                label.push_str(" [builtin]");
            } else {
                label.push_str(" [custom]");
            }
            column.push(selection_option_row(idx, label, idx == self.selected_idx));
            if let Some(base_url) = &row.provider.base_url {
                column.push(Line::from(format!("    {base_url}").dim()));
            }
        }

        if self.rows.is_empty() {
            column.push(Line::from("No providers configured.".dim()));
        }

        if let Some(error) = &self.error {
            column.push("");
            column.push(Line::from(error.clone().red()));
        }

        column.push("");
        column.push(Line::from("Esc closes".dim()));
        column.render(area, buf);
    }

    fn render_edit(&self, area: Rect, buf: &mut Buffer, edit: &EditState) {
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(area);

        Paragraph::new(Line::from("Edit provider".bold())).render(rows[0], buf);
        Paragraph::new(Line::from("Responses API only.".dim())).render(rows[1], buf);
        Paragraph::new(Line::from(
            "Tab/Shift+Tab switches fields. Enter saves. Esc cancels.".dim(),
        ))
        .render(rows[2], buf);

        self.render_field(
            rows[3],
            buf,
            Field::Id,
            &edit.draft.id,
            &edit.textarea,
            &edit.textarea_state,
        );
        self.render_field(
            rows[4],
            buf,
            Field::Name,
            &edit.draft.name,
            &edit.textarea,
            &edit.textarea_state,
        );
        self.render_field(
            rows[5],
            buf,
            Field::BaseUrl,
            &edit.draft.base_url,
            &edit.textarea,
            &edit.textarea_state,
        );
        self.render_field(
            rows[6],
            buf,
            Field::ApiKey,
            &edit.draft.api_key,
            &edit.textarea,
            &edit.textarea_state,
        );

        let mut footer = ColumnRenderable::new();
        footer.push(Line::from("Request type: responses".dim()));
        footer.push(Line::from(
            "Saving here does not switch the current default provider.".dim(),
        ));
        footer.push(Line::from(
            "Leave API key blank to keep the existing secure value. Type CLEAR to remove it.".dim(),
        ));
        if edit.draft.existing_api_key && edit.draft.api_key.trim().is_empty() {
            footer.push(Line::from(
                "A secure API key is already stored for this provider.".dim(),
            ));
        }
        if let Some(error) = &self.error {
            footer.push(Line::from(error.clone().red()));
        }
        footer.render(rows[7], buf);
    }

    fn render_field(
        &self,
        area: Rect,
        buf: &mut Buffer,
        field: Field,
        value: &str,
        textarea: &TextArea,
        textarea_state: &RefCell<TextAreaState>,
    ) {
        let focused = matches!(&self.mode, Mode::Edit(edit) if edit.draft.focused_field == field);
        let label = if focused {
            format!("> {}", field.label()).cyan().to_string()
        } else {
            format!("  {}", field.label())
        };
        let chunks = Layout::vertical([Constraint::Length(1), Constraint::Length(2)]).split(area);
        Paragraph::new(Line::from(label)).render(chunks[0], buf);
        Clear.render(chunks[1], buf);
        if focused {
            let mut state = textarea_state.borrow_mut();
            StatefulWidgetRef::render_ref(&textarea, chunks[1], buf, &mut state);
        } else {
            let display = if field == Field::ApiKey && !value.is_empty() {
                "•".repeat(value.chars().count())
            } else {
                value.to_string()
            };
            Paragraph::new(display)
                .wrap(Wrap { trim: false })
                .render(chunks[1], buf);
        }
    }
}

impl BottomPaneView for ProviderManagerView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        self.error = None;
        match &mut self.mode {
            Mode::List => match key_event {
                KeyEvent {
                    code: KeyCode::Up, ..
                }
                | KeyEvent {
                    code: KeyCode::Char('k'),
                    modifiers: KeyModifiers::NONE,
                    ..
                } => self.move_selection(/*delta*/ -1),
                KeyEvent {
                    code: KeyCode::Down,
                    ..
                }
                | KeyEvent {
                    code: KeyCode::Char('j'),
                    modifiers: KeyModifiers::NONE,
                    ..
                } => self.move_selection(/*delta*/ 1),
                KeyEvent {
                    code: KeyCode::Char('n'),
                    modifiers: KeyModifiers::NONE,
                    ..
                } => self.start_new(),
                KeyEvent {
                    code: KeyCode::Char('e'),
                    modifiers: KeyModifiers::NONE,
                    ..
                } => self.start_edit(),
                KeyEvent {
                    code: KeyCode::Char('d'),
                    modifiers: KeyModifiers::NONE,
                    ..
                } => self.delete_selected(),
                KeyEvent {
                    code: KeyCode::Char('u'),
                    modifiers: KeyModifiers::NONE,
                    ..
                } => self.edit_usage_script(),
                KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                    ..
                } => self.activate_selected(),
                KeyEvent {
                    code: KeyCode::Esc, ..
                } => {
                    self.complete = true;
                }
                _ => {}
            },
            Mode::Edit(edit) => match key_event {
                KeyEvent {
                    code: KeyCode::Esc, ..
                } => {
                    self.mode = Mode::List;
                }
                KeyEvent {
                    code: KeyCode::BackTab,
                    ..
                }
                | KeyEvent {
                    code: KeyCode::Tab,
                    modifiers: KeyModifiers::SHIFT,
                    ..
                } => {
                    Self::sync_draft_from_textarea(edit);
                    edit.draft.focused_field = edit.draft.focused_field.prev();
                    Self::sync_textarea_from_draft(edit);
                }
                KeyEvent {
                    code: KeyCode::Tab, ..
                } => {
                    Self::sync_draft_from_textarea(edit);
                    edit.draft.focused_field = edit.draft.focused_field.next();
                    Self::sync_textarea_from_draft(edit);
                }
                KeyEvent {
                    code: KeyCode::Enter,
                    modifiers: KeyModifiers::NONE,
                    ..
                } => self.save_current_draft(),
                other => {
                    edit.textarea.input(other);
                    Self::sync_draft_from_textarea(edit);
                }
            },
        }
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.complete = true;
        CancellationEvent::Handled
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        let Mode::Edit(edit) = &mut self.mode else {
            return false;
        };
        if pasted.is_empty() {
            return false;
        }
        edit.textarea.insert_str(&pasted);
        Self::sync_draft_from_textarea(edit);
        true
    }

    fn prefer_esc_to_handle_key_event(&self) -> bool {
        matches!(self.mode, Mode::Edit(_))
    }
}

impl Renderable for ProviderManagerView {
    fn desired_height(&self, _width: u16) -> u16 {
        22
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        Clear.render(area, buf);
        match &self.mode {
            Mode::List => self.render_list(area, buf),
            Mode::Edit(edit) => self.render_edit(area, buf, edit),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_core::config::ConfigBuilder;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use tokio::sync::mpsc::unbounded_channel;

    async fn config_with_custom_provider(id: &str, provider: ModelProviderInfo) -> Config {
        let mut config = ConfigBuilder::default()
            .codex_home(std::env::temp_dir())
            .build()
            .await
            .expect("config");
        config
            .model_providers
            .insert(id.to_string(), provider.clone());
        config.model_provider_id = id.to_string();
        config.model_provider = provider;
        config
    }

    fn provider_manager_view(
        config: &Config,
    ) -> (
        ProviderManagerView,
        tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    ) {
        let (tx, rx) = unbounded_channel();
        (
            ProviderManagerView::new(config, AppEventSender::new(tx)),
            rx,
        )
    }

    #[tokio::test]
    async fn editing_provider_canonicalizes_custom_auth_to_api_key() {
        let provider = ModelProviderInfo {
            name: "Original".to_string(),
            base_url: Some("https://example.com/v1".to_string()),
            auth_strategy: ModelProviderAuthStrategy::OAuthOrApiKey,
            oauth: Some(codex_core::ModelProviderOAuthConfig {
                url: Some("https://example.com/oauth".to_string()),
                scopes: Some(vec!["scope.read".to_string()]),
                oauth_resource: Some("https://example.com/resource".to_string()),
            }),
            api_key: Some("sk-old".to_string()),
            env_key: Some("CUSTOM_API_KEY".to_string()),
            env_key_instructions: Some("export CUSTOM_API_KEY=...".to_string()),
            experimental_bearer_token: Some("token".to_string()),
            wire_api: WireApi::Responses,
            query_params: Some(HashMap::from([(
                "api-version".to_string(),
                "2025-01-01".to_string(),
            )])),
            http_headers: Some(HashMap::from([(
                "x-extra-header".to_string(),
                "true".to_string(),
            )])),
            env_http_headers: Some(HashMap::from([(
                "x-env-header".to_string(),
                "CUSTOM_HEADER".to_string(),
            )])),
            request_max_retries: Some(3),
            stream_max_retries: Some(4),
            stream_idle_timeout_ms: Some(5_000),
            requires_openai_auth: true,
            supports_websockets: true,
        };
        let config = config_with_custom_provider("custom-provider", provider.clone()).await;
        let (mut view, mut rx) = provider_manager_view(&config);

        view.start_edit();
        let Mode::Edit(edit) = &mut view.mode else {
            panic!("expected edit mode");
        };
        edit.textarea.set_text_clearing_elements("Renamed provider");

        view.save_current_draft();

        assert_eq!(view.error, None);
        assert!(view.complete);
        match rx.try_recv().expect("persist event") {
            AppEvent::PersistModelProvider {
                id,
                provider: saved,
                ..
            } => {
                assert_eq!(id, "custom-provider");
                assert_eq!(saved.name, "Renamed provider");
                assert_eq!(saved.base_url, provider.base_url);
                assert_eq!(saved.auth_strategy, ModelProviderAuthStrategy::ApiKey);
                assert_eq!(saved.oauth, None);
                assert_eq!(saved.api_key, None);
                assert_eq!(saved.env_key, provider.env_key);
                assert_eq!(saved.env_key_instructions, provider.env_key_instructions);
                assert_eq!(
                    saved.experimental_bearer_token,
                    provider.experimental_bearer_token
                );
                assert_eq!(saved.query_params, provider.query_params);
                assert_eq!(saved.http_headers, provider.http_headers);
                assert_eq!(saved.env_http_headers, provider.env_http_headers);
                assert_eq!(saved.request_max_retries, provider.request_max_retries);
                assert_eq!(saved.stream_max_retries, provider.stream_max_retries);
                assert_eq!(
                    saved.stream_idle_timeout_ms,
                    provider.stream_idle_timeout_ms
                );
                assert!(!saved.requires_openai_auth);
                assert_eq!(saved.supports_websockets, provider.supports_websockets);
            }
            event => panic!("unexpected event: {event:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn cannot_rename_current_default_provider() {
        let provider = ModelProviderInfo {
            name: "Original".to_string(),
            base_url: Some("https://example.com/v1".to_string()),
            auth_strategy: ModelProviderAuthStrategy::None,
            oauth: None,
            api_key: Some("sk-old".to_string()),
            env_key: Some("CUSTOM_API_KEY".to_string()),
            env_key_instructions: None,
            experimental_bearer_token: None,
            wire_api: WireApi::Responses,
            query_params: None,
            http_headers: None,
            env_http_headers: None,
            request_max_retries: None,
            stream_max_retries: None,
            stream_idle_timeout_ms: None,
            requires_openai_auth: false,
            supports_websockets: false,
        };
        let config = config_with_custom_provider("custom-provider", provider).await;
        let (mut view, mut rx) = provider_manager_view(&config);

        view.start_edit();
        let Mode::Edit(edit) = &mut view.mode else {
            panic!("expected edit mode");
        };
        edit.draft.focused_field = Field::Id;
        edit.textarea.set_text_clearing_elements("renamed-provider");

        view.save_current_draft();

        assert_eq!(
            view.error.as_deref(),
            Some("Rename is disabled for the current default provider. Switch defaults first."),
        );
        assert!(!view.complete);
        assert!(rx.try_recv().is_err());
    }
}
