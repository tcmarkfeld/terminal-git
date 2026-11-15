use anyhow::{Result, bail};
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::{Constraint, Flex, Layout, Position, Rect},
    style::{Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::agent::{self, Agent};
use crate::commit::edit_text;
use crate::view::{ACCENT, ADD_FG, BORDER, DEL_FG, FAINT, MUTED, RAISED, SURFACE, TEXT, key_hints};

/// Diff lines shown above the input; the full excerpt is still sent.
const PREVIEW_LINES: usize = 6;

pub struct CommentDialog {
    /// "src/view.rs lines 12-18"
    target: String,
    /// Selected diff lines, each prefixed with `+`, `-` or a space.
    excerpt: Vec<String>,
    text: String,
    agents: Result<Vec<Agent>>,
    state: ListState,
    error: Option<String>,
    list_area: Rect,
}

impl CommentDialog {
    pub fn new(target: String, excerpt: Vec<String>) -> Self {
        let agents = agent::discover();
        let state = ListState::default().with_selected(agents.as_ref().ok().filter(|a| !a.is_empty()).map(|_| 0));
        CommentDialog { target, excerpt, text: String::new(), agents, state, error: None, list_area: Rect::default() }
    }

    /// Returns true when the comment should be sent.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        self.error = None;
        let newline = key.modifiers.intersects(KeyModifiers::ALT | KeyModifiers::SHIFT);
        match key.code {
            KeyCode::Enter if newline => self.text.push('\n'),
            KeyCode::Enter => return true,
            KeyCode::Up => self.move_agent(-1),
            KeyCode::Down | KeyCode::Tab => self.move_agent(1),
            _ => edit_text(&mut self.text, key),
        }
        false
    }

    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        let pos = Position::new(mouse.column, mouse.row);
        if !self.list_area.contains(pos) {
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollDown => self.move_agent(1),
            MouseEventKind::ScrollUp => self.move_agent(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                let i = self.state.offset() + (mouse.row - self.list_area.y) as usize;
                if self.agents.as_ref().is_ok_and(|agents| i < agents.len()) {
                    self.state.select(Some(i));
                }
            }
            _ => {}
        }
    }

    fn move_agent(&mut self, delta: isize) {
        let count = self.agents.as_ref().map_or(0, Vec::len);
        if let Some(i) = self.state.selected() {
            self.state.select(Some(i.saturating_add_signed(delta).min(count.saturating_sub(1))));
        }
    }

    /// Sends the comment and returns a confirmation; failures stay in the dialog.
    pub fn send(&mut self) -> Option<String> {
        let result = (|| -> Result<String> {
            if self.text.trim().is_empty() {
                bail!("Write a comment first");
            }
            let agents = self.agents.as_ref().map_err(|err| anyhow::anyhow!("{err}"))?;
            let Some(agent) = self.state.selected().and_then(|i| agents.get(i)) else { bail!("No agent to send to") };
            let message = format!(
                "Comment on {}:\n\n```diff\n{}\n```\n\n{}",
                self.target,
                self.excerpt.join("\n"),
                self.text.trim()
            );
            agent::send(agent, &message)?;
            Ok(format!("Sent to {} ({})", agent.name, agent.place))
        })();
        result.map_err(|err| self.error = Some(err.to_string())).ok()
    }
}

pub fn draw(frame: &mut Frame, dialog: &mut CommentDialog, area: Rect) {
    let preview = dialog.excerpt.len().min(PREVIEW_LINES) + usize::from(dialog.excerpt.len() > PREVIEW_LINES);
    let agents = dialog.agents.as_ref().map_or(1, |a| a.len().clamp(1, 5));
    let width = area.width.saturating_sub(8).min(90);
    let height = (preview + agents + 11) as u16;
    let [popup] = Layout::horizontal([Constraint::Length(width)]).flex(Flex::Center).areas(area);
    let [popup] = Layout::vertical([Constraint::Length(height.min(area.height))]).flex(Flex::Center).areas(popup);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(BORDER))
        .bg(SURFACE)
        .title(Span::raw(" Comment ").fg(TEXT).bold())
        .title(Line::from(Span::raw(format!(" {} ", dialog.target)).fg(MUTED)).right_aligned());
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    let [excerpt_area, _, input_area, _, label_area, list_area, error_area, _, hints_area] = Layout::vertical([
        Constraint::Length(preview as u16),
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let hints = [("⏎", "send"), ("⌥⏎", "newline"), ("↑↓", "agent"), ("esc", "cancel")];
    frame.render_widget(key_hints(&hints), Rect { x: hints_area.x + 1, width: hints_area.width.saturating_sub(1), ..hints_area });
    dialog.list_area = list_area;

    let mut lines: Vec<Line> = dialog
        .excerpt
        .iter()
        .take(PREVIEW_LINES)
        .map(|line| {
            let fg = match line.as_bytes().first() {
                Some(b'+') => ADD_FG,
                Some(b'-') => DEL_FG,
                _ => MUTED,
            };
            Line::from(Span::raw(format!(" {line}")).fg(fg))
        })
        .collect();
    if dialog.excerpt.len() > PREVIEW_LINES {
        lines.push(Line::from(Span::raw(format!("   … {} more lines", dialog.excerpt.len() - PREVIEW_LINES)).fg(FAINT)));
    }
    frame.render_widget(Paragraph::new(lines), excerpt_area);

    let text = if dialog.text.is_empty() { Span::raw("Leave a comment…").fg(FAINT) } else { Span::raw(dialog.text.as_str()).fg(TEXT) };
    let mut input: Vec<Line> = vec![Line::from(vec![Span::raw(" › ").fg(ACCENT), text])];
    if dialog.text.contains('\n') {
        input = dialog
            .text
            .split('\n')
            .enumerate()
            .map(|(i, line)| Line::from(vec![Span::raw(if i == 0 { " › " } else { "   " }).fg(ACCENT), Span::raw(line).fg(TEXT)]))
            .collect();
    }
    if let Some(last) = input.last_mut() {
        last.push_span(Span::raw("▏").fg(ACCENT));
    }
    // Keep the end of a long comment in view.
    let width = (input_area.width as usize).max(1);
    let rows: usize = input.iter().map(|line| line.width().div_ceil(width).max(1)).sum();
    let rows = rows as u16;
    let input = Paragraph::new(input).wrap(Wrap { trim: false }).scroll((rows.saturating_sub(input_area.height), 0));
    frame.render_widget(input, input_area);

    frame.render_widget(Line::from(Span::raw(" Send to").fg(MUTED)), label_area);
    match &dialog.agents {
        Ok(agents) if !agents.is_empty() => {
            let items: Vec<ListItem> = agents
                .iter()
                .enumerate()
                .map(|(i, agent)| {
                    let dot = if dialog.state.selected() == Some(i) { Span::raw(" ● ").fg(ACCENT) } else { Span::raw(" ○ ").fg(FAINT) };
                    ListItem::new(Line::from(vec![
                        dot,
                        Span::raw(agent.name.as_str()).fg(TEXT).bold(),
                        Span::raw(format!("  {}", agent.place)).fg(MUTED),
                        Span::raw(format!("  {}", agent.cwd)).fg(FAINT),
                    ]))
                })
                .collect();
            frame.render_stateful_widget(List::new(items).highlight_style(Style::new().bg(RAISED)), list_area, &mut dialog.state);
        }
        Ok(_) => frame.render_widget(Span::raw("   No agents found in Rex").fg(FAINT), list_area),
        Err(err) => frame.render_widget(Span::raw(format!("   {err}")).fg(DEL_FG), list_area),
    }
    if let Some(error) = &dialog.error {
        frame.render_widget(Span::raw(format!(" ✕ {error}")).fg(DEL_FG), error_area);
    }
}
