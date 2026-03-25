use std::cell::RefCell;

use codex_core::ModelProviderInfo;
use codex_core::WireApi;
use codex_core::config::Config;
use codex_core::provider_login_capabilities;
use codex_core::validate_model_provider_id;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
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
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

use crate::bottom_pane::textarea::TextArea;
use crate::bottom_pane::textarea::TextAreaState;
use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::StepStateProvider;
use crate::render::renderable::ColumnRenderable;
use crate::render::renderable::Renderable;
use crate::selection_list::selection_option_row;

use super::onboarding_screen::StepState;

#[derive(Clone)]
struct ProviderEntry {
    id: String,
    provider: ModelProviderInfo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Id,
    Name,
    BaseUrl,
}

impl Field {
    fn next(self) -> Self {
        match self {
            Self::Id => Self::Name,
            Self::Name => Self::BaseUrl,
            Self::BaseUrl => Self::Id,
        }
    }

    fn prev(self) -> Self {
        match self {
            Self::Id => Self::BaseUrl,
            Self::Name => Self::Id,
            Self::BaseUrl => Self::Name,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Id => "Provider ID",
            Self::Name => "Display name",
            Self::BaseUrl => "Base URL",
        }
    }
}

#[derive(Clone)]
struct ProviderDraft {
    id: String,
    name: String,
    base_url: String,
    focused_field: Field,
}

impl ProviderDraft {
    fn new() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            base_url: String::new(),
            focused_field: Field::Id,
        }
    }

    fn current_field_value(&self) -> &str {
        match self.focused_field {
            Field::Id => &self.id,
            Field::Name => &self.name,
            Field::BaseUrl => &self.base_url,
        }
    }

    fn set_current_field_value(&mut self, value: String) {
        match self.focused_field {
            Field::Id => self.id = value,
            Field::Name => self.name = value,
            Field::BaseUrl => self.base_url = value,
        }
    }

    fn to_provider(&self) -> ModelProviderInfo {
        ModelProviderInfo {
            name: self.name.trim().to_string(),
            base_url: Some(self.base_url.trim().to_string()).filter(|value| !value.is_empty()),
            auth_strategy: codex_core::ModelProviderAuthStrategy::None,
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
            requires_openai_auth: true,
            supports_websockets: false,
        }
    }
}

struct CreateState {
    draft: ProviderDraft,
    textarea: TextArea,
    textarea_state: RefCell<TextAreaState>,
}

enum Mode {
    List,
    Create(Box<CreateState>),
}

pub(crate) struct ProviderWidget {
    providers: Vec<ProviderEntry>,
    highlighted: usize,
    selected: Option<String>,
    mode: Mode,
    error: Option<String>,
}

impl ProviderWidget {
    pub(crate) fn new(config: &Config) -> Self {
        let mut providers: Vec<ProviderEntry> = config
            .model_providers
            .iter()
            .map(|(id, provider)| ProviderEntry {
                id: id.clone(),
                provider: provider.clone(),
            })
            .collect();
        providers.sort_by(|left, right| left.id.cmp(&right.id));
        let highlighted = providers
            .iter()
            .position(|entry| entry.id == config.model_provider_id)
            .unwrap_or(0);
        Self {
            providers,
            highlighted,
            selected: None,
            mode: Mode::List,
            error: None,
        }
    }

    pub(crate) fn selected_provider(&self) -> Option<(&str, &ModelProviderInfo)> {
        let selected = self.selected.as_deref()?;
        self.providers
            .iter()
            .find(|entry| entry.id == selected)
            .map(|entry| (entry.id.as_str(), &entry.provider))
    }

    pub(crate) fn selected_requires_auth(&self) -> Option<bool> {
        self.selected_provider().map(|(provider_id, provider)| {
            provider_login_capabilities(provider_id, provider).requires_auth()
        })
    }

    pub(crate) fn clear_selection(&mut self) {
        self.selected = None;
        self.mode = Mode::List;
        self.error = None;
    }

    fn list_len(&self) -> usize {
        self.providers.len() + 1
    }

    fn begin_create(&mut self) {
        let draft = ProviderDraft::new();
        let mut textarea = TextArea::new();
        textarea.set_text_clearing_elements(draft.current_field_value());
        self.mode = Mode::Create(Box::new(CreateState {
            draft,
            textarea,
            textarea_state: RefCell::new(TextAreaState::default()),
        }));
        self.error = None;
    }

    fn sync_draft_from_textarea(create: &mut CreateState) {
        create
            .draft
            .set_current_field_value(create.textarea.text().to_string());
    }

    fn sync_textarea_from_draft(create: &mut CreateState) {
        create
            .textarea
            .set_text_clearing_elements(create.draft.current_field_value());
    }

    fn save_created_provider(&mut self) {
        let Mode::Create(create) = &mut self.mode else {
            return;
        };
        Self::sync_draft_from_textarea(create);

        let provider_id = create.draft.id.trim().to_string();
        if let Err(err) = validate_model_provider_id(&provider_id) {
            self.error = Some(err);
            return;
        }
        if self.providers.iter().any(|entry| entry.id == provider_id) {
            self.error = Some("Provider ID already exists.".to_string());
            return;
        }
        if create.draft.name.trim().is_empty() {
            self.error = Some("Display name is required.".to_string());
            return;
        }
        if create.draft.base_url.trim().is_empty() {
            self.error = Some("Base URL is required.".to_string());
            return;
        }

        let provider = create.draft.to_provider();
        self.providers.push(ProviderEntry {
            id: provider_id.clone(),
            provider,
        });
        self.providers.sort_by(|left, right| left.id.cmp(&right.id));
        self.highlighted = self
            .providers
            .iter()
            .position(|entry| entry.id == provider_id)
            .unwrap_or(0);
        self.selected = Some(provider_id);
        self.mode = Mode::List;
        self.error = None;
    }

    fn handle_list_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.list_len() != 0 {
                    self.highlighted = self.highlighted.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.highlighted + 1 < self.list_len() {
                    self.highlighted += 1;
                }
            }
            KeyCode::Enter => {
                if self.highlighted == self.providers.len() {
                    self.begin_create();
                } else if let Some(entry) = self.providers.get(self.highlighted) {
                    self.selected = Some(entry.id.clone());
                    self.error = None;
                }
            }
            _ => {}
        }
    }

    fn handle_create_key_event(&mut self, key_event: KeyEvent) {
        let Mode::Create(create) = &mut self.mode else {
            return;
        };
        match key_event {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.mode = Mode::List;
                self.error = None;
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
                Self::sync_draft_from_textarea(create);
                create.draft.focused_field = create.draft.focused_field.prev();
                Self::sync_textarea_from_draft(create);
                self.error = None;
            }
            KeyEvent {
                code: KeyCode::Tab, ..
            } => {
                Self::sync_draft_from_textarea(create);
                create.draft.focused_field = create.draft.focused_field.next();
                Self::sync_textarea_from_draft(create);
                self.error = None;
            }
            KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                ..
            } => self.save_created_provider(),
            other => {
                create.textarea.input(other);
                Self::sync_draft_from_textarea(create);
                self.error = None;
            }
        }
    }

    fn render_list(&self, area: Rect, buf: &mut Buffer) {
        let mut column = ColumnRenderable::new();
        column.push(Line::from("Choose a model provider".bold()));
        column.push(Line::from(
            "Pick an existing provider or create a custom one before signing in.".dim(),
        ));
        column.push("");

        for (idx, entry) in self.providers.iter().enumerate() {
            let label = format!("{} ({})", entry.provider.name, entry.id);
            column.push(selection_option_row(idx, label, idx == self.highlighted));
        }
        column.push(selection_option_row(
            self.providers.len(),
            "Create custom provider".to_string(),
            self.highlighted == self.providers.len(),
        ));

        if let Some(error) = &self.error {
            column.push("");
            column.push(Line::from(error.clone().red()));
        }

        column.push("");
        column.push(Line::from("Press Enter to continue".dim()));
        column.render(area, buf);
    }

    fn render_create(&self, area: Rect, buf: &mut Buffer, create: &CreateState) {
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(area);

        Paragraph::new(Line::from("Create custom provider".bold())).render(rows[0], buf);
        Paragraph::new(Line::from("< Back to provider list (Esc)".cyan())).render(rows[1], buf);
        Paragraph::new(Line::from("Tab switches fields. Enter saves.".dim()))
            .wrap(Wrap { trim: false })
            .render(rows[2], buf);

        self.render_field(
            rows[3],
            buf,
            Field::Id,
            &create.draft.id,
            &create.textarea,
            &create.textarea_state,
        );
        self.render_field(
            rows[4],
            buf,
            Field::Name,
            &create.draft.name,
            &create.textarea,
            &create.textarea_state,
        );
        self.render_field(
            rows[5],
            buf,
            Field::BaseUrl,
            &create.draft.base_url,
            &create.textarea,
            &create.textarea_state,
        );
        let mut footer = ColumnRenderable::new();
        if let Some(error) = &self.error {
            footer.push(Line::from(error.clone().red()));
        }
        footer.render(rows[6], buf);
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
        let focused =
            matches!(&self.mode, Mode::Create(create) if create.draft.focused_field == field);
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
            Paragraph::new(value.to_string())
                .wrap(Wrap { trim: false })
                .render(chunks[1], buf);
        }
    }
}

impl KeyboardHandler for ProviderWidget {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if key_event.kind == KeyEventKind::Release {
            return;
        }
        match self.mode {
            Mode::List => self.handle_list_key_event(key_event),
            Mode::Create(_) => self.handle_create_key_event(key_event),
        }
    }

    fn handle_paste(&mut self, pasted: String) {
        let Mode::Create(create) = &mut self.mode else {
            return;
        };
        if pasted.is_empty() {
            return;
        }
        create.textarea.insert_str(&pasted);
        Self::sync_draft_from_textarea(create);
        self.error = None;
    }
}

impl StepStateProvider for ProviderWidget {
    fn get_step_state(&self) -> StepState {
        if self.selected.is_some() {
            StepState::Complete
        } else {
            StepState::InProgress
        }
    }
}

impl WidgetRef for &ProviderWidget {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        match &self.mode {
            Mode::List => self.render_list(area, buf),
            Mode::Create(create) => self.render_create(area, buf, create),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_backend::VT100Backend;
    use codex_core::ModelProviderAuthStrategy;
    use pretty_assertions::assert_eq;
    use ratatui::Terminal;
    use ratatui::widgets::WidgetRef;

    fn provider(name: &str, auth_strategy: ModelProviderAuthStrategy) -> ModelProviderInfo {
        ModelProviderInfo {
            name: name.to_string(),
            base_url: Some("https://api.example.com".to_string()),
            auth_strategy,
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
        }
    }

    fn compact_render(output: &str) -> String {
        output
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render(widget: &ProviderWidget, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(VT100Backend::new(width, height)).expect("terminal");
        terminal
            .draw(|f| (&widget).render_ref(f.area(), f.buffer_mut()))
            .expect("draw");
        compact_render(&terminal.backend().to_string())
    }

    #[test]
    fn create_provider_renders_back_action_snapshot() {
        let draft = ProviderDraft::new();
        let mut textarea = TextArea::new();
        textarea.set_text_clearing_elements(draft.current_field_value());
        let widget = ProviderWidget {
            providers: vec![ProviderEntry {
                id: "openai".to_string(),
                provider: provider("OpenAI", ModelProviderAuthStrategy::OpenAi),
            }],
            highlighted: 1,
            selected: None,
            mode: Mode::Create(Box::new(CreateState {
                draft,
                textarea,
                textarea_state: RefCell::new(TextAreaState::default()),
            })),
            error: None,
        };

        insta::assert_snapshot!(
                                                                                                                                                    render(&widget, 70, 24),
                                                                                                                                                    @r"
Create custom provider
< Back to provider list (Esc)
Tab switches fields. Enter saves.
> Provider ID
  Display name
  Base URL
"
                                                                                                                                                );
    }

    #[test]
    fn clear_selection_preserves_created_provider_and_highlight() {
        let mut widget = ProviderWidget {
            providers: vec![ProviderEntry {
                id: "acme".to_string(),
                provider: provider("Acme", ModelProviderAuthStrategy::ApiKey),
            }],
            highlighted: 0,
            selected: Some("acme".to_string()),
            mode: Mode::List,
            error: Some("stale error".to_string()),
        };

        widget.clear_selection();

        assert_eq!(widget.selected, None);
        assert_eq!(widget.highlighted, 0);
        assert_eq!(widget.providers.len(), 1);
        assert_eq!(widget.providers[0].id, "acme");
        assert_eq!(widget.error, None);
        assert!(matches!(widget.mode, Mode::List));
    }
}
