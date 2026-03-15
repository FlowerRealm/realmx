use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::provider_usage::ProviderUsageEditorState;
use crate::render::renderable::Renderable;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::widgets::Clear;
use ratatui::widgets::Paragraph;
use ratatui::widgets::StatefulWidgetRef;
use ratatui::widgets::Widget;
use std::cell::RefCell;

use super::BottomPaneView;
use super::CancellationEvent;
use super::textarea::TextArea;
use super::textarea::TextAreaState;

pub(crate) struct ProviderUsageScriptEditorView {
    app_event_tx: AppEventSender,
    provider_id: String,
    provider_name: String,
    script_path: String,
    has_existing_script: bool,
    textarea: TextArea,
    textarea_state: RefCell<TextAreaState>,
    complete: bool,
    error: Option<String>,
}

impl ProviderUsageScriptEditorView {
    pub(crate) fn new(state: ProviderUsageEditorState, app_event_tx: AppEventSender) -> Self {
        let mut textarea = TextArea::new();
        textarea.set_text_clearing_elements(&state.initial_contents);
        Self {
            app_event_tx,
            provider_id: state.provider_id,
            provider_name: state.provider_name,
            script_path: state.script_path.display().to_string(),
            has_existing_script: state.has_existing_script,
            textarea,
            textarea_state: RefCell::new(TextAreaState::default()),
            complete: false,
            error: None,
        }
    }

    fn save(&mut self) {
        let script = self.textarea.text().trim().to_string();
        if script.is_empty() {
            self.error = Some("Usage script cannot be empty.".to_string());
            return;
        }

        self.app_event_tx
            .send(AppEvent::PersistProviderUsageScript {
                provider_id: self.provider_id.clone(),
                script,
            });
        self.complete = true;
    }

    fn delete(&mut self) {
        if !self.has_existing_script {
            self.error = Some("No usage script exists yet for this provider.".to_string());
            return;
        }

        self.app_event_tx.send(AppEvent::DeleteProviderUsageScript {
            provider_id: self.provider_id.clone(),
        });
        self.complete = true;
    }
}

impl BottomPaneView for ProviderUsageScriptEditorView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        self.error = None;
        match key_event {
            KeyEvent {
                code: KeyCode::Esc, ..
            } => {
                self.complete = true;
            }
            KeyEvent {
                code: KeyCode::Char('s'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => self.save(),
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers,
                ..
            } if modifiers.contains(KeyModifiers::CONTROL) => self.delete(),
            other => self.textarea.input(other),
        }
    }

    fn on_ctrl_c(&mut self) -> CancellationEvent {
        self.complete = true;
        CancellationEvent::Handled
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn handle_paste(&mut self, pasted: String) -> bool {
        if pasted.is_empty() {
            return false;
        }
        self.textarea.insert_str(&pasted);
        true
    }
}

impl Renderable for ProviderUsageScriptEditorView {
    fn desired_height(&self, width: u16) -> u16 {
        let editor_width = width.saturating_sub(2).max(1);
        self.textarea
            .desired_height(editor_width)
            .saturating_add(6)
            .clamp(14, 24)
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        Clear.render(area, buf);

        Paragraph::new(Line::from(
            format!(
                "Edit remote usage script: {} ({})",
                self.provider_name, self.provider_id
            )
            .bold(),
        ))
        .render(
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
            buf,
        );
        Paragraph::new(Line::from(self.script_path.clone().cyan())).render(
            Rect {
                x: area.x,
                y: area.y.saturating_add(1),
                width: area.width,
                height: 1,
            },
            buf,
        );
        Paragraph::new(Line::from(
            "Ctrl+S saves. Ctrl+R deletes the file. Esc cancels.".dim(),
        ))
        .render(
            Rect {
                x: area.x,
                y: area.y.saturating_add(2),
                width: area.width,
                height: 1,
            },
            buf,
        );

        let editor_area = Rect {
            x: area.x,
            y: area.y.saturating_add(3),
            width: area.width,
            height: area.height.saturating_sub(5),
        };
        Clear.render(editor_area, buf);
        let mut textarea_state = self.textarea_state.borrow_mut();
        StatefulWidgetRef::render_ref(&(&self.textarea), editor_area, buf, &mut textarea_state);

        let footer = self
            .error
            .clone()
            .map(ratatui::prelude::Stylize::red)
            .unwrap_or_else(|| {
                if self.has_existing_script {
                    "Project usage.js enables remote usage polling for this provider.".dim()
                } else {
                    "Saving will create .codex/providers/<provider-id>/usage.js.".dim()
                }
            });
        Paragraph::new(Line::from(footer)).render(
            Rect {
                x: area.x,
                y: area.y.saturating_add(area.height.saturating_sub(1)),
                width: area.width,
                height: 1,
            },
            buf,
        );
    }

    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> {
        if area.height <= 5 {
            return None;
        }
        self.textarea.cursor_pos_with_state(
            Rect {
                x: area.x,
                y: area.y.saturating_add(3),
                width: area.width,
                height: area.height.saturating_sub(5),
            },
            *self.textarea_state.borrow(),
        )
    }
}
