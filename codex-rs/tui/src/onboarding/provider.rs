use codex_core::config::Config;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::widgets::WidgetRef;
use ratatui::widgets::Wrap;

use crate::onboarding::onboarding_screen::KeyboardHandler;
use crate::onboarding::onboarding_screen::StepStateProvider;
use crate::render::Insets;
use crate::render::renderable::ColumnRenderable;
use crate::render::renderable::Renderable;
use crate::render::renderable::RenderableExt as _;
use crate::selection_list::selection_option_row;

use super::onboarding_screen::StepState;

pub(crate) struct ProviderWidget {
    providers: Vec<(String, String, bool)>,
    highlighted: usize,
    selected: Option<String>,
}

impl ProviderWidget {
    pub(crate) fn new(config: &Config) -> Self {
        let mut providers: Vec<(String, String, bool)> = config
            .model_providers
            .iter()
            .map(|(id, provider)| {
                (
                    id.clone(),
                    provider.name.clone(),
                    provider.requires_openai_auth,
                )
            })
            .collect();
        providers.sort_by(|left, right| left.0.cmp(&right.0));
        let highlighted = providers
            .iter()
            .position(|(id, _, _)| *id == config.model_provider_id)
            .unwrap_or(0);
        Self {
            providers,
            highlighted,
            selected: None,
        }
    }

    pub(crate) fn selected_provider_id(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub(crate) fn selected_requires_openai_auth(&self) -> Option<bool> {
        self.selected.as_ref().and_then(|selected| {
            self.providers
                .iter()
                .find(|(id, _, _)| id == selected)
                .map(|(_, _, requires_auth)| *requires_auth)
        })
    }
}

impl KeyboardHandler for ProviderWidget {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if key_event.kind == KeyEventKind::Release {
            return;
        }
        match key_event.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if !self.providers.is_empty() {
                    self.highlighted = self.highlighted.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.highlighted + 1 < self.providers.len() {
                    self.highlighted += 1;
                }
            }
            KeyCode::Enter => {
                if let Some((id, _, _)) = self.providers.get(self.highlighted) {
                    self.selected = Some(id.clone());
                }
            }
            _ => {}
        }
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
        let mut column = ColumnRenderable::new();
        column.push(Line::from("Choose a model provider".bold()));
        column.push(Line::from(
            "This decides whether login is needed. Use /provider later to add or edit custom providers."
                .dim(),
        ));
        column.push("");
        for (idx, (id, name, requires_auth)) in self.providers.iter().enumerate() {
            let label = if *requires_auth {
                format!("{name} ({id}, OpenAI auth)")
            } else {
                format!("{name} ({id})")
            };
            column.push(selection_option_row(idx, label, idx == self.highlighted));
        }
        column.push("");
        column.push(
            Paragraph::new("Press Enter to continue")
                .wrap(Wrap { trim: true })
                .inset(Insets::tlbr(
                    /*top*/ 0, /*left*/ 2, /*bottom*/ 0, /*right*/ 0,
                )),
        );
        column.render(area, buf);
    }
}
