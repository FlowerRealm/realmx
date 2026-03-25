use codex_core::AuthManager;
use codex_core::config::Config;
#[cfg(target_os = "windows")]
use codex_core::windows_sandbox::WindowsSandboxLevelExt;
#[cfg(target_os = "windows")]
use codex_protocol::config_types::WindowsSandboxLevel;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::widgets::Clear;
use ratatui::widgets::WidgetRef;

use codex_protocol::config_types::ForcedLoginMethod;

use crate::LoginStatus;
use crate::onboarding::auth::AuthModeWidget;
use crate::onboarding::auth::SignInOption;
use crate::onboarding::auth::SignInState;
use crate::onboarding::provider::ProviderWidget;
use crate::onboarding::trust_directory::TrustDirectorySelection;
use crate::onboarding::trust_directory::TrustDirectoryWidget;
use crate::onboarding::welcome::WelcomeWidget;
use crate::tui::FrameRequester;
use crate::tui::Tui;
use crate::tui::TuiEvent;
use color_eyre::eyre::Result;
use std::sync::Arc;
use std::sync::RwLock;

#[allow(clippy::large_enum_variant)]
enum Step {
    Welcome(WelcomeWidget),
    Provider(ProviderWidget),
    Auth(AuthModeWidget),
    TrustDirectory(TrustDirectoryWidget),
}

pub(crate) trait KeyboardHandler {
    fn handle_key_event(&mut self, key_event: KeyEvent);
    fn handle_paste(&mut self, _pasted: String) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepState {
    Hidden,
    InProgress,
    Complete,
}

pub(crate) trait StepStateProvider {
    fn get_step_state(&self) -> StepState;
}

pub(crate) struct OnboardingScreen {
    request_frame: FrameRequester,
    steps: Vec<Step>,
    initial_provider_id: String,
    login_step_enabled: bool,
    is_done: bool,
    should_exit: bool,
}

pub(crate) struct OnboardingScreenArgs {
    pub show_trust_screen: bool,
    pub login_status: LoginStatus,
    pub auth_manager: Arc<AuthManager>,
    pub config: Config,
}

pub(crate) struct OnboardingResult {
    pub directory_trust_decision: Option<TrustDirectorySelection>,
    pub provider_changed: bool,
    pub should_exit: bool,
}

impl OnboardingScreen {
    pub(crate) fn new(tui: &mut Tui, args: OnboardingScreenArgs) -> Self {
        let OnboardingScreenArgs {
            show_trust_screen,
            login_status,
            auth_manager,
            config,
        } = args;
        let cwd = config.cwd.clone();
        let forced_chatgpt_workspace_id = config.forced_chatgpt_workspace_id.clone();
        let forced_login_method = config.forced_login_method;
        let codex_home = config.codex_home.clone();
        let cli_auth_credentials_store_mode = config.cli_auth_credentials_store_mode;
        let oauth_credentials_store_mode = config.mcp_oauth_credentials_store_mode;
        let mcp_oauth_callback_port = config.mcp_oauth_callback_port;
        let mcp_oauth_callback_uri = config.mcp_oauth_callback_url.clone();
        let login_step_enabled = true;
        let mut steps: Vec<Step> = Vec::new();
        steps.push(Step::Welcome(WelcomeWidget::new(
            !matches!(login_status, LoginStatus::NotAuthenticated),
            tui.frame_requester(),
            config.animations,
        )));
        steps.push(Step::Provider(ProviderWidget::new(&config)));
        if login_step_enabled {
            let highlighted_mode = match forced_login_method {
                Some(ForcedLoginMethod::Api) => SignInOption::ApiKey,
                _ => SignInOption::ChatGpt,
            };
            steps.push(Step::Auth(AuthModeWidget {
                request_frame: tui.frame_requester(),
                highlighted_mode,
                error: None,
                sign_in_state: Arc::new(RwLock::new(SignInState::PickMode)),
                provider_id: config.model_provider_id.clone(),
                provider: config.model_provider.clone(),
                codex_home: codex_home.clone(),
                cli_auth_credentials_store_mode,
                oauth_credentials_store_mode,
                mcp_oauth_callback_port,
                mcp_oauth_callback_uri,
                login_status,
                auth_manager,
                forced_chatgpt_workspace_id,
                forced_login_method,
                animations_enabled: config.animations,
                back_to_provider_selection_requested: false,
            }))
        }
        #[cfg(target_os = "windows")]
        let show_windows_create_sandbox_hint =
            WindowsSandboxLevel::from_config(&config) == WindowsSandboxLevel::Disabled;
        #[cfg(not(target_os = "windows"))]
        let show_windows_create_sandbox_hint = false;
        let highlighted = TrustDirectorySelection::Trust;
        if show_trust_screen {
            steps.push(Step::TrustDirectory(TrustDirectoryWidget {
                cwd,
                codex_home,
                show_windows_create_sandbox_hint,
                should_quit: false,
                selection: None,
                highlighted,
                error: None,
            }))
        }
        // TODO: add git warning.
        Self {
            request_frame: tui.frame_requester(),
            steps,
            initial_provider_id: config.model_provider_id,
            login_step_enabled,
            is_done: false,
            should_exit: false,
        }
    }

    fn effective_step_state(&self, step: &Step) -> StepState {
        match step {
            Step::Auth(_) => self.auth_step_state(),
            _ => step.intrinsic_state(),
        }
    }

    fn active_step_index(&self) -> Option<usize> {
        let auth_state = self.auth_step_state();
        let mut last_visible = None;
        for (idx, step) in self.steps.iter().enumerate() {
            let step_state = match step {
                Step::Auth(_) => auth_state,
                _ => step.intrinsic_state(),
            };
            match step_state {
                StepState::Hidden => continue,
                StepState::Complete => last_visible = Some(idx),
                StepState::InProgress => return Some(idx),
            }
        }

        last_visible
    }

    fn active_step(&self) -> Option<&Step> {
        let idx = self.active_step_index()?;
        self.steps.get(idx)
    }

    fn active_step_mut(&mut self) -> Option<&mut Step> {
        let idx = self.active_step_index()?;
        self.steps.get_mut(idx)
    }

    fn selected_provider_requires_auth(&self) -> bool {
        self.steps
            .iter()
            .find_map(|step| {
                if let Step::Provider(widget) = step {
                    widget.selected_requires_auth()
                } else {
                    None
                }
            })
            .unwrap_or(false)
    }

    fn auth_step_state(&self) -> StepState {
        if !self.login_step_enabled || !self.selected_provider_requires_auth() {
            return StepState::Hidden;
        }

        self.steps
            .iter()
            .find_map(|step| {
                if let Step::Auth(widget) = step {
                    let in_pick_mode = widget
                        .sign_in_state
                        .read()
                        .is_ok_and(|state| matches!(*state, SignInState::PickMode));
                    Some(if in_pick_mode && widget.is_authenticated() {
                        StepState::Hidden
                    } else {
                        widget.get_step_state()
                    })
                } else {
                    None
                }
            })
            .unwrap_or(StepState::Hidden)
    }

    fn is_auth_in_progress(&self) -> bool {
        self.auth_step_state() == StepState::InProgress
    }

    pub(crate) fn is_done(&self) -> bool {
        self.is_done
            || !self
                .steps
                .iter()
                .any(|step| self.effective_step_state(step) == StepState::InProgress)
    }

    pub fn directory_trust_decision(&self) -> Option<TrustDirectorySelection> {
        self.steps
            .iter()
            .find_map(|step| {
                if let Step::TrustDirectory(TrustDirectoryWidget { selection, .. }) = step {
                    Some(*selection)
                } else {
                    None
                }
            })
            .flatten()
    }

    pub fn should_exit(&self) -> bool {
        self.should_exit
    }

    fn is_api_key_entry_active(&self) -> bool {
        self.steps.iter().any(|step| {
            if let Step::Auth(widget) = step {
                return widget
                    .sign_in_state
                    .read()
                    .is_ok_and(|g| matches!(&*g, SignInState::ApiKeyEntry(_)));
            }
            false
        })
    }

    fn sync_auth_widget_provider(&mut self) {
        let selected_provider = self.steps.iter().find_map(|step| {
            if let Step::Provider(widget) = step {
                widget
                    .selected_provider()
                    .map(|(id, provider)| (id.to_string(), provider.clone()))
            } else {
                None
            }
        });
        if let Some((provider_id, provider)) = selected_provider {
            for step in &mut self.steps {
                if let Step::Auth(widget) = step {
                    widget.set_provider(provider_id.clone(), provider.clone());
                }
            }
        }
    }

    fn apply_auth_navigation_requests(&mut self) {
        let should_back_to_provider_selection = self
            .steps
            .iter_mut()
            .find_map(|step| {
                if let Step::Auth(widget) = step {
                    Some(widget.take_provider_selection_requested())
                } else {
                    None
                }
            })
            .unwrap_or(false);

        if !should_back_to_provider_selection {
            return;
        }

        for step in &mut self.steps {
            if let Step::Provider(widget) = step {
                widget.clear_selection();
                break;
            }
        }
    }
}

impl KeyboardHandler for OnboardingScreen {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if !matches!(key_event.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return;
        }
        let is_api_key_entry_active = self.is_api_key_entry_active();
        let should_quit = match key_event {
            KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: crossterm::event::KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                ..
            }
            | KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: crossterm::event::KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                ..
            } => true,
            KeyEvent {
                code: KeyCode::Char('q'),
                kind: KeyEventKind::Press,
                ..
            } => !is_api_key_entry_active,
            _ => false,
        };
        if should_quit {
            if self.is_auth_in_progress() {
                // If the user cancels the auth menu, exit the app rather than
                // leave the user at a prompt in an unauthed state.
                self.should_exit = true;
            }
            self.is_done = true;
        } else {
            if let Some(active_step) = self.active_step_mut() {
                active_step.handle_key_event(key_event);
            }
            self.apply_auth_navigation_requests();
            self.sync_auth_widget_provider();
            if self.steps.iter().any(|step| {
                if let Step::TrustDirectory(widget) = step {
                    widget.should_quit()
                } else {
                    false
                }
            }) {
                self.should_exit = true;
                self.is_done = true;
            }
        }
        self.request_frame.schedule_frame();
    }

    fn handle_paste(&mut self, pasted: String) {
        if pasted.is_empty() {
            return;
        }

        if let Some(active_step) = self.active_step_mut() {
            active_step.handle_paste(pasted);
        }
        self.request_frame.schedule_frame();
    }
}

impl WidgetRef for &OnboardingScreen {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        if let Some(step) = self.active_step() {
            if let Step::Welcome(widget) = step {
                widget.update_layout_area(area);
            }
            step.render_ref(area, buf);
        }
    }
}

impl KeyboardHandler for Step {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match self {
            Step::Welcome(widget) => widget.handle_key_event(key_event),
            Step::Provider(widget) => widget.handle_key_event(key_event),
            Step::Auth(widget) => widget.handle_key_event(key_event),
            Step::TrustDirectory(widget) => widget.handle_key_event(key_event),
        }
    }

    fn handle_paste(&mut self, pasted: String) {
        match self {
            Step::Welcome(_) => {}
            Step::Provider(widget) => widget.handle_paste(pasted),
            Step::Auth(widget) => widget.handle_paste(pasted),
            Step::TrustDirectory(widget) => widget.handle_paste(pasted),
        }
    }
}

impl Step {
    fn intrinsic_state(&self) -> StepState {
        match self {
            Step::Welcome(w) => w.get_step_state(),
            Step::Provider(w) => w.get_step_state(),
            Step::Auth(w) => w.get_step_state(),
            Step::TrustDirectory(w) => w.get_step_state(),
        }
    }
}

impl StepStateProvider for Step {
    fn get_step_state(&self) -> StepState {
        self.intrinsic_state()
    }
}

impl WidgetRef for Step {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        match self {
            Step::Welcome(widget) => {
                widget.render_ref(area, buf);
            }
            Step::Provider(widget) => {
                widget.render_ref(area, buf);
            }
            Step::Auth(widget) => {
                widget.render_ref(area, buf);
            }
            Step::TrustDirectory(widget) => {
                widget.render_ref(area, buf);
            }
        }
    }
}

pub(crate) async fn run_onboarding_app(
    args: OnboardingScreenArgs,
    tui: &mut Tui,
) -> Result<OnboardingResult> {
    use tokio_stream::StreamExt;

    let initial_providers = args.config.model_providers.clone();
    let codex_home = args.config.codex_home.clone();
    let active_profile = args.config.active_profile.clone();
    let mut onboarding_screen = OnboardingScreen::new(tui, args);
    // One-time guard to fully clear the screen after ChatGPT login success message is shown
    let mut did_full_clear_after_success = false;

    tui.draw(u16::MAX, |frame| {
        frame.render_widget_ref(&onboarding_screen, frame.area());
    })?;

    let tui_events = tui.event_stream();
    tokio::pin!(tui_events);

    while !onboarding_screen.is_done() {
        if let Some(event) = tui_events.next().await {
            match event {
                TuiEvent::Key(key_event) => {
                    onboarding_screen.handle_key_event(key_event);
                }
                TuiEvent::Paste(text) => {
                    onboarding_screen.handle_paste(text);
                }
                TuiEvent::Draw => {
                    if !did_full_clear_after_success
                        && onboarding_screen.steps.iter().any(|step| {
                            if let Step::Auth(w) = step {
                                w.sign_in_state.read().is_ok_and(|g| {
                                    matches!(&*g, super::auth::SignInState::ChatGptSuccessMessage)
                                })
                            } else {
                                false
                            }
                        })
                    {
                        // Reset any lingering SGR (underline/color) before clearing
                        let _ = ratatui::crossterm::execute!(
                            std::io::stdout(),
                            ratatui::crossterm::style::SetAttribute(
                                ratatui::crossterm::style::Attribute::Reset
                            ),
                            ratatui::crossterm::style::SetAttribute(
                                ratatui::crossterm::style::Attribute::NoUnderline
                            ),
                            ratatui::crossterm::style::SetForegroundColor(
                                ratatui::crossterm::style::Color::Reset
                            ),
                            ratatui::crossterm::style::SetBackgroundColor(
                                ratatui::crossterm::style::Color::Reset
                            )
                        );
                        let _ = tui.terminal.clear();
                        did_full_clear_after_success = true;
                    }
                    let _ = tui.draw(u16::MAX, |frame| {
                        frame.render_widget_ref(&onboarding_screen, frame.area());
                    });
                }
            }
        }
    }
    let selected_provider = onboarding_screen.steps.iter().find_map(|step| {
        if let Step::Provider(widget) = step {
            widget
                .selected_provider()
                .map(|(id, provider)| (id.to_string(), provider.clone()))
        } else {
            None
        }
    });
    let provider_changed = if let Some((provider_id, provider)) = selected_provider {
        let mut edits = codex_core::config::edit::ConfigEditsBuilder::new(&codex_home);
        let should_persist_provider = initial_providers
            .get(&provider_id)
            .is_none_or(|existing| *existing != provider);
        if should_persist_provider {
            edits = edits.set_model_provider(&provider_id, &provider);
        }
        let provider_changed = provider_id != onboarding_screen.initial_provider_id;
        if provider_changed {
            edits = edits
                .with_profile(active_profile.as_deref())
                .set_default_model_provider(&provider_id);
        }
        if should_persist_provider || provider_changed {
            let _ = edits.apply_blocking();
        }
        provider_changed
    } else {
        false
    };
    Ok(OnboardingResult {
        directory_trust_decision: onboarding_screen.directory_trust_decision(),
        provider_changed,
        should_exit: onboarding_screen.should_exit(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_backend::VT100Backend;
    use ratatui::Terminal;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn compact_render(output: &str) -> String {
        output
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_only_the_active_step_snapshot() {
        let codex_home = TempDir::new().expect("temp home");
        let screen = OnboardingScreen {
            request_frame: FrameRequester::test_dummy(),
            steps: vec![
                Step::Welcome(WelcomeWidget::new(
                    false,
                    FrameRequester::test_dummy(),
                    false,
                )),
                Step::TrustDirectory(TrustDirectoryWidget {
                    codex_home: codex_home.path().to_path_buf(),
                    cwd: PathBuf::from("/workspace/project"),
                    show_windows_create_sandbox_hint: false,
                    should_quit: false,
                    selection: None,
                    highlighted: TrustDirectorySelection::Trust,
                    error: None,
                }),
            ],
            initial_provider_id: "openai".to_string(),
            login_step_enabled: false,
            is_done: false,
            should_exit: false,
        };

        let mut terminal = Terminal::new(VT100Backend::new(72, 16)).expect("terminal");
        terminal
            .draw(|f| f.render_widget_ref(&screen, f.area()))
            .expect("draw");

        let rendered = compact_render(&terminal.backend().to_string());
        assert!(!rendered.contains("Welcome to Codex"));
        insta::assert_snapshot!(
                                                            rendered,
                                                            @r"
> You are in /workspace/project
Do you trust the contents of this directory? Working with untrusted
contents comes with higher risk of prompt injection.
› 1. Yes, continue
  2. No, quit
Press Enter to continue
"
                                                        );
    }
}
