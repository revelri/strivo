//! Generic text-input modal — a single line with a prompt, value, and
//! cursor. Consumed by Schedule add/edit, recording rename/move, and
//! the M2 settings string/int/path editors.
//!
//! The modal itself is dumb data on AppState (`text_input` field). The
//! caller embeds a [`TextInputPurpose`] so the Enter handler knows what
//! to do with the committed value.

use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

use crate::app::AppState;
use crate::tui::theme::Theme;

/// What the input modal is for. The Enter handler matches on this to
/// route the committed string back to the right pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextInputPurpose {
    RenameRecording {
        job_id: uuid::Uuid,
    },
    MoveRecording {
        job_id: uuid::Uuid,
    },
    ScheduleAddChannel,
    ScheduleAddCron {
        channel: String,
    },
    ScheduleAddDuration {
        channel: String,
        cron: String,
    },
    ScheduleEditCron {
        index: usize,
    },
    ScheduleEditDuration {
        index: usize,
    },
    /// Edit a settings string value — used by the M2 settings tab.
    SettingsString {
        key: &'static str,
    },
    /// Edit a settings integer value (post-validated as u64).
    SettingsInt {
        key: &'static str,
    },
    /// Edit a settings path. Tilde is expanded on commit.
    SettingsPath {
        key: &'static str,
    },
    /// Command palette (M4.2.a). Value is parsed as a KeyAction name
    /// and dispatched through apply_key_action.
    CommandPalette,
    /// Channel marks (M4.2.b). Single-char value sets / jumps a mark.
    SetMark,
    JumpMark,
}

/// Live state for an open text-input modal.
#[derive(Debug, Clone)]
pub struct TextInputState {
    pub purpose: TextInputPurpose,
    pub prompt: String,
    pub value: String,
    pub cursor: usize,
    /// Optional validation error from the most recent edit. Rendered
    /// in red below the line; non-empty here means Enter is disabled.
    pub error: Option<String>,
}

impl TextInputState {
    pub fn new(
        purpose: TextInputPurpose,
        prompt: impl Into<String>,
        initial: impl Into<String>,
    ) -> Self {
        let value: String = initial.into();
        let cursor = value.chars().count();
        Self {
            purpose,
            prompt: prompt.into(),
            value,
            cursor,
            error: None,
        }
    }

    /// Handle a key event. Returns `true` when Enter has committed a
    /// valid value (the caller drains `value` and acts on `purpose`).
    pub fn handle_key(&mut self, key: &crossterm::event::KeyEvent) -> TextInputResult {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc => TextInputResult::Cancel,
            KeyCode::Enter => {
                if self.error.is_some() || self.value.is_empty() {
                    TextInputResult::Idle
                } else {
                    TextInputResult::Commit
                }
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let chars: Vec<char> = self.value.chars().collect();
                    let new = self.cursor - 1;
                    self.value = chars
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != new)
                        .map(|(_, c)| *c)
                        .collect();
                    self.cursor = new;
                }
                TextInputResult::Idle
            }
            KeyCode::Delete => {
                let chars: Vec<char> = self.value.chars().collect();
                if self.cursor < chars.len() {
                    self.value = chars
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != self.cursor)
                        .map(|(_, c)| *c)
                        .collect();
                }
                TextInputResult::Idle
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
                TextInputResult::Idle
            }
            KeyCode::Right => {
                let len = self.value.chars().count();
                if self.cursor < len {
                    self.cursor += 1;
                }
                TextInputResult::Idle
            }
            KeyCode::Home => {
                self.cursor = 0;
                TextInputResult::Idle
            }
            KeyCode::End => {
                self.cursor = self.value.chars().count();
                TextInputResult::Idle
            }
            KeyCode::Char(c) => {
                let mut chars: Vec<char> = self.value.chars().collect();
                chars.insert(self.cursor, c);
                self.value = chars.iter().collect();
                self.cursor += 1;
                TextInputResult::Idle
            }
            _ => TextInputResult::Idle,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextInputResult {
    /// No state change worth acting on.
    Idle,
    /// User pressed Esc — caller should drop the modal.
    Cancel,
    /// User pressed Enter on a non-empty, non-erroring value.
    Commit,
}

/// Render the modal centered on `area`.
pub fn render(frame: &mut Frame, area: Rect, app: &AppState, enter_progress: f32) {
    let Some(ref st) = app.text_input else {
        return;
    };

    let height = if st.error.is_some() { 7 } else { 5 };
    let [_, center_v, _] = Layout::vertical([
        Constraint::Percentage(30),
        Constraint::Length(height),
        Constraint::Percentage(40),
    ])
    .areas(area);

    let [_, center, _] = Layout::horizontal([
        Constraint::Percentage(15),
        Constraint::Min(60),
        Constraint::Percentage(15),
    ])
    .areas(center_v);

    frame.render_widget(Clear, center);

    let border_style = Theme::border_ramp(enter_progress).add_modifier(Modifier::BOLD);
    let block = Block::default()
        .title(format!(" {} ", st.prompt))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style);

    let inner = block.inner(center);
    frame.render_widget(block, center);

    // Body: value with an inline cursor glyph.
    let chars: Vec<char> = st.value.chars().collect();
    let cursor = st.cursor.min(chars.len());
    let before: String = chars[..cursor].iter().collect();
    let after: String = chars[cursor..].iter().collect();

    let mut rows: Vec<Line> = Vec::new();
    rows.push(Line::from(vec![
        Span::raw("  "),
        Span::raw(before),
        Span::styled("▌", Style::new().fg(Theme::primary())),
        Span::raw(after),
    ]));
    rows.push(Line::raw(""));
    if let Some(ref err) = st.error {
        rows.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(err.clone(), Style::new().fg(Theme::red())),
        ]));
    }
    rows.push(Line::from(vec![Span::styled(
        "  Enter commit · Esc cancel",
        Style::new().fg(Theme::muted()),
    )]));

    frame.render_widget(Paragraph::new(rows), inner);
}
