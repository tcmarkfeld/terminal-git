use anyhow::Result;
use ratatui::{
    Frame,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind},
    layout::{Constraint, Flex, Layout, Position, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, ListState},
};

use crate::git::{self, FileDiff};
use crate::view::{ACCENT, ADD_FG, DEL_FG, FAINT, MUTED, RAISED, SURFACE, TEXT, status_letter};

#[derive(Clone, Copy, PartialEq)]
pub enum Action {
    Commit,
    CommitAndPush,
    Push,
}

const BUTTONS: [(Action, &str); 3] =
    [(Action::Commit, " Commit ⏎ "), (Action::CommitAndPush, " Commit & Push ⌘\\ "), (Action::Push, " Push ⌘U ")];

enum Notice {
    Working(&'static str),
    Done(String),
    Error(String),
}

pub struct CommitDialog {
    message: String,
    selected: Vec<bool>,
    files_focused: bool,
    state: ListState,
    notice: Option<Notice>,
    upstream: Option<(String, usize)>,
    message_area: Rect,
    list_area: Rect,
    button_areas: [Rect; 3],
}

impl CommitDialog {
    pub fn new(files: usize) -> Self {
        let mut dialog = CommitDialog {
            message: String::new(),
            selected: Vec::new(),
            files_focused: false,
            state: ListState::default(),
            notice: None,
            upstream: git::upstream(),
            message_area: Rect::default(),
            list_area: Rect::default(),
            button_areas: [Rect::default(); 3],
        };
        dialog.reset_files(files);
        dialog
    }

    /// Everything starts selected, with the message box focused.
    pub fn reset_files(&mut self, files: usize) {
        self.files_focused = false;
        self.selected = vec![true; files];
        self.state.select(if files == 0 { None } else { Some(0) });
    }

    /// Validates, then returns the action for the caller to run after the next redraw.
    fn request(&mut self, action: Action) -> Option<Action> {
        if action != Action::Push {
            if self.message.trim().is_empty() {
                self.notice = Some(Notice::Error("Write a commit message first".into()));
                return None;
            }
            if !self.selected.contains(&true) {
                self.notice = Some(Notice::Error("Select at least one file".into()));
                return None;
            }
        }
        self.notice = Some(Notice::Working(if action == Action::Commit { "Committing…" } else { "Pushing…" }));
        Some(action)
    }

    pub fn run(&mut self, action: Action, files: &[FileDiff], branch: &str) {
        let result = (|| -> Result<String> {
            let mut done = Vec::new();
            if action != Action::Push {
                let paths: Vec<&str> = files
                    .iter()
                    .zip(&self.selected)
                    .filter(|(_, selected)| **selected)
                    .flat_map(|(file, _)| std::iter::once(file.path.as_str()).chain(file.old_path.as_deref()))
                    .collect();
                let hash = git::commit(self.message.trim(), &paths)?;
                self.message.clear();
                done.push(format!("Committed {hash}"));
            }
            if action != Action::Commit {
                git::push(branch)?;
                done.push(if done.is_empty() { "Pushed".into() } else { "pushed".into() });
            }
            Ok(done.join(" and "))
        })();
        self.notice = Some(match result {
            Ok(done) => Notice::Done(done),
            Err(err) => Notice::Error(err.to_string()),
        });
        self.upstream = git::upstream();
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let cmd = key.modifiers.contains(KeyModifiers::SUPER);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        self.notice = None;
        match key.code {
            // Legacy terminals report Ctrl+\ as Ctrl+4.
            KeyCode::Char('\\') if ctrl || cmd => return self.request(Action::CommitAndPush),
            KeyCode::Char('4') if ctrl => return self.request(Action::CommitAndPush),
            KeyCode::Enter => return self.request(Action::Commit),
            // Not Ctrl+U: terminals send that for ⌘⌫, which clears the message below.
            KeyCode::Char('u') if cmd => return self.request(Action::Push),
            KeyCode::Tab | KeyCode::BackTab => self.files_focused = !self.files_focused,
            _ if self.files_focused => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.move_cursor(-1),
                KeyCode::Down | KeyCode::Char('j') => self.move_cursor(1),
                KeyCode::Char(' ') => {
                    if let Some(i) = self.state.selected() {
                        self.selected[i] = !self.selected[i];
                    }
                }
                KeyCode::Char('a') => {
                    let all = !self.selected.iter().all(|s| *s);
                    self.selected.fill(all);
                }
                _ => {}
            },
            KeyCode::Backspace if cmd => self.message.clear(),
            KeyCode::Char('u') if ctrl => self.message.clear(),
            KeyCode::Backspace if alt => self.delete_word(),
            KeyCode::Char('w') if ctrl => self.delete_word(),
            KeyCode::Backspace => {
                self.message.pop();
            }
            KeyCode::Char(c) if !ctrl && !cmd => self.message.push(c),
            _ => {}
        }
        None
    }

    pub fn on_mouse(&mut self, mouse: MouseEvent) -> Option<Action> {
        let pos = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::ScrollDown if self.list_area.contains(pos) => self.move_cursor(1),
            MouseEventKind::ScrollUp if self.list_area.contains(pos) => self.move_cursor(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(i) = self.button_areas.iter().position(|area| area.contains(pos)) {
                    return self.request(BUTTONS[i].0);
                }
                if self.list_area.contains(pos) {
                    let i = self.state.offset() + (mouse.row - self.list_area.y) as usize;
                    if i < self.selected.len() {
                        self.files_focused = true;
                        self.state.select(Some(i));
                        self.selected[i] = !self.selected[i];
                    }
                } else if self.message_area.contains(pos) {
                    self.files_focused = false;
                }
            }
            _ => {}
        }
        None
    }

    fn move_cursor(&mut self, delta: isize) {
        if let Some(i) = self.state.selected() {
            self.state.select(Some(i.saturating_add_signed(delta).min(self.selected.len().saturating_sub(1))));
        }
    }

    fn delete_word(&mut self) {
        self.message.truncate(self.message.trim_end().len());
        self.message.truncate(self.message.rfind(' ').map_or(0, |i| i + 1));
    }
}

pub fn draw(frame: &mut Frame, dialog: &mut CommitDialog, files: &[FileDiff], area: Rect) {
    let width = area.width.saturating_sub(8).min(84);
    let height = (files.len() as u16 + 9).min(area.height.saturating_sub(4)).max(12);
    let [popup] = Layout::horizontal([Constraint::Length(width)]).flex(Flex::Center).areas(area);
    let [popup] = Layout::vertical([Constraint::Length(height)]).flex(Flex::Center).areas(popup);

    let upstream = match &dialog.upstream {
        Some((name, 0)) => Line::from(vec![Span::raw(format!(" → {name} ")).fg(MUTED), Span::raw("up to date ").fg(FAINT)]),
        Some((name, ahead)) => Line::from(vec![Span::raw(format!(" → {name} ")).fg(MUTED), Span::raw(format!("↑{ahead} ")).fg(ACCENT)]),
        None => Line::from(Span::raw(" no upstream ").fg(FAINT)),
    };
    let hints = [("⇥", "files"), ("space", "toggle"), ("a", "all"), ("esc", "close")];
    let hints: Vec<Span> = hints
        .iter()
        .flat_map(|(key, label)| [Span::raw(format!(" {key}")).fg(MUTED), Span::raw(format!(" {label} ")).fg(FAINT)])
        .collect();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(RAISED))
        .bg(SURFACE)
        .title(Span::raw(" Commit ").fg(TEXT).bold())
        .title(upstream.right_aligned())
        .title_bottom(Line::from(hints));
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

    let [message_area, _, list_area, _, buttons_area, notice_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    dialog.message_area = message_area;
    dialog.list_area = list_area;

    // Keep the end of a long message visible.
    let room = (message_area.width as usize).saturating_sub(5);
    let skip = dialog.message.chars().count().saturating_sub(room);
    let message: String = dialog.message.chars().skip(skip).collect();
    let text = if message.is_empty() { Span::raw("Commit message…").fg(FAINT) } else { Span::raw(message).fg(TEXT) };
    let cursor = if dialog.files_focused { Span::raw("") } else { Span::raw("▏").fg(ACCENT) };
    let prompt = Span::raw(" › ").fg(if dialog.files_focused { FAINT } else { ACCENT });
    frame.render_widget(Line::from(vec![prompt, text, cursor]), message_area);

    let items: Vec<ListItem> = files
        .iter()
        .zip(&dialog.selected)
        .map(|(file, &selected)| {
            let (dir, name) = file.path.rsplit_once('/').map_or(("", file.path.as_str()), |(d, n)| (d, n));
            let (letter, fg) = status_letter(file.status);
            let stats = format!("+{} -{}", file.added, file.removed);
            let used = 7 + name.chars().count() + 2 + dir.chars().count() + stats.len() + 2;
            let pad = (list_area.width as usize).saturating_sub(used);
            let check = if selected { Span::raw(" ● ").fg(ACCENT) } else { Span::raw(" ○ ").fg(FAINT) };
            ListItem::new(Line::from(vec![
                check,
                Span::raw(format!("{letter}  ")).fg(fg),
                Span::raw(name).fg(if selected { TEXT } else { MUTED }),
                Span::raw(format!("  {dir}")).fg(FAINT),
                Span::raw(" ".repeat(pad)),
                Span::raw(format!("+{}", file.added)).fg(ADD_FG),
                Span::raw(format!(" -{}", file.removed)).fg(DEL_FG),
                Span::raw("  "),
            ]))
        })
        .collect();
    let highlight = if dialog.files_focused { Style::new().bg(RAISED) } else { Style::new() };
    frame.render_stateful_widget(List::new(items).highlight_style(highlight), list_area, &mut dialog.state);

    let mut x = buttons_area.x + 1;
    for (i, (action, label)) in BUTTONS.iter().enumerate() {
        let span = Span::raw(*label);
        let rect = Rect { x, width: span.width() as u16, ..buttons_area }.intersection(buttons_area);
        let span = if *action == Action::Commit { span.fg(Color::White).bg(ACCENT).bold() } else { span.fg(TEXT).bg(RAISED) };
        frame.render_widget(span, rect);
        dialog.button_areas[i] = rect;
        x += rect.width + 2;
    }
    let count = dialog.selected.iter().filter(|s| **s).count();
    let summary = Line::from(format!("{count} of {} files  ", files.len())).fg(MUTED).right_aligned();
    frame.render_widget(summary, buttons_area);

    let notice = match &dialog.notice {
        Some(Notice::Working(text)) => Span::raw(format!(" {text}")).fg(MUTED),
        Some(Notice::Done(text)) => Span::raw(format!(" ✓ {text}")).fg(ADD_FG),
        Some(Notice::Error(text)) => Span::raw(format!(" ✕ {text}")).fg(DEL_FG),
        None => Span::raw(""),
    };
    frame.render_widget(notice, notice_area);
}
