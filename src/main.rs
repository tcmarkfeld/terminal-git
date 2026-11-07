mod commit;
mod git;
mod view;

use std::collections::HashSet;

use anyhow::Result;
use ratatui::{
    DefaultTerminal,
    crossterm::{
        event::{
            self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
            KeyboardEnhancementFlags, MouseButton, MouseEvent, MouseEventKind, PopKeyboardEnhancementFlags,
            PushKeyboardEnhancementFlags,
        },
        execute,
        terminal::supports_keyboard_enhancement,
    },
    layout::{Position, Rect},
    widgets::ListState,
};
use two_face::re_exports::syntect::{highlighting::Theme, parsing::SyntaxSet};

use commit::{Action, CommitDialog};
use git::{FileDiff, Repo};
use view::Row;

/// Narrower than this (e.g. a phone), long lines wrap instead of scrolling sideways.
const WRAP_BELOW: u16 = 60;

pub struct Picker {
    query: String,
    matches: Vec<usize>,
    state: ListState,
    list_area: Rect,
}

pub struct App {
    repo: Repo,
    args: Vec<String>,
    files: Vec<FileDiff>,
    rows: Vec<Row>,
    /// Index into `rows` of each file's header.
    header_rows: Vec<usize>,
    /// Screen lines as (index into `rows`, wrapped segment); collapsed files contribute only their header.
    lines: Vec<(usize, usize)>,
    /// Index into `lines` of each file's header.
    file_rows: Vec<usize>,
    collapsed: HashSet<String>,
    body: Rect,
    scroll: usize,
    hscroll: usize,
    /// Code columns per screen line when wrapping, 0 when not.
    wrap: usize,
    height: usize,
    picker: Option<Picker>,
    commit: Option<CommitDialog>,
    /// Runs after the next draw so the dialog can show progress first.
    pending: Option<Action>,
    syntaxes: SyntaxSet,
    theme: Theme,
}

impl App {
    fn reload(&mut self) -> Result<()> {
        self.files = git::load(&self.args)?;
        (self.rows, self.header_rows) = view::build_rows(&self.files, &self.syntaxes, &self.theme);
        self.layout();
        self.clamp_scroll();
        Ok(())
    }

    fn clamp_scroll(&mut self) {
        // Allow overscroll so the last file's header can reach the top.
        let last_file = self.file_rows.last().copied().unwrap_or(0);
        self.scroll = self.scroll.min(self.lines.len().saturating_sub(self.height).max(last_file));
    }

    fn layout(&mut self) {
        self.lines.clear();
        self.file_rows.clear();
        for i in 0..self.header_rows.len() {
            let header = self.header_rows[i];
            // Each header after the first is preceded by a blank spacer row.
            let end = self.header_rows.get(i + 1).map_or(self.rows.len(), |&next| next - 1);
            if i > 0 {
                self.push_line(header - 1);
            }
            self.file_rows.push(self.lines.len());
            self.push_line(header);
            if !self.collapsed.contains(&self.files[i].path) {
                (header + 1..end).for_each(|row| self.push_line(row));
            }
        }
    }

    fn push_line(&mut self, row: usize) {
        let segments = view::segments(&self.rows[row], self.wrap);
        self.lines.extend((0..segments).map(|segment| (row, segment)));
    }

    /// Re-wraps when the width crosses into or changes within wrapping mode, keeping the top row in place.
    fn set_width(&mut self, width: u16) {
        let gutter = view::num_width(&self.rows) + 4;
        let wrap = if width < WRAP_BELOW { (width as usize).saturating_sub(gutter).max(10) } else { 0 };
        if wrap == self.wrap {
            return;
        }
        let top = self.lines.get(self.scroll).map(|line| line.0);
        self.wrap = wrap;
        self.layout();
        self.scroll = top.and_then(|top| self.lines.iter().position(|line| line.0 == top)).unwrap_or(0);
    }

    /// Keeps the file's header at the same screen position.
    fn set_collapsed(&mut self, file: usize, collapsed: bool) {
        let Some(path) = self.files.get(file).map(|f| f.path.clone()) else { return };
        let offset = self.file_rows[file].saturating_sub(self.scroll);
        if collapsed {
            self.collapsed.insert(path);
        } else {
            self.collapsed.remove(&path);
        }
        self.layout();
        self.scroll = self.file_rows[file].saturating_sub(offset);
        self.clamp_scroll();
    }

    fn scroll_by(&mut self, delta: isize) {
        self.scroll = self.scroll.saturating_add_signed(delta);
        self.clamp_scroll();
    }

    /// The file whose rows are at the top of the viewport.
    fn current_file(&self) -> usize {
        self.file_rows.partition_point(|&row| row <= self.scroll).saturating_sub(1)
    }

    fn jump_to_file(&mut self, file: usize) {
        if let Some(&row) = self.file_rows.get(file) {
            self.scroll = row;
            self.clamp_scroll();
        }
    }

    fn open_picker(&mut self) {
        let mut picker = Picker { query: String::new(), matches: Vec::new(), state: ListState::default(), list_area: Rect::default() };
        picker.matches = (0..self.files.len()).collect();
        picker.state.select(Some(self.current_file()));
        self.picker = Some(picker);
    }

    fn filter_picker(&mut self) {
        let Some(picker) = self.picker.as_mut() else { return };
        let query = picker.query.to_lowercase();
        picker.matches = (0..self.files.len()).filter(|&i| fuzzy_match(&self.files[i].path.to_lowercase(), &query)).collect();
        picker.state.select(if picker.matches.is_empty() { None } else { Some(0) });
        self.preview_selection();
    }

    fn move_selection(&mut self, delta: isize) {
        let Some(picker) = self.picker.as_mut() else { return };
        let Some(selected) = picker.state.selected() else { return };
        let last = picker.matches.len().saturating_sub(1);
        picker.state.select(Some(selected.saturating_add_signed(delta).min(last)));
        self.preview_selection();
    }

    /// The diff behind the popup follows the selection.
    fn preview_selection(&mut self) {
        let Some(picker) = &self.picker else { return };
        if let Some(&file) = picker.state.selected().and_then(|i| picker.matches.get(i)) {
            self.jump_to_file(file);
        }
    }

    /// Returns false when the app should quit.
    fn on_key(&mut self, key: KeyEvent) -> Result<bool> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let cmd = key.modifiers.contains(KeyModifiers::SUPER);
        let toggles_commit = matches!(key.code, KeyCode::Char('/')) && (ctrl || cmd)
            || ctrl && matches!(key.code, KeyCode::Char('7' | '_'));
        // Committing only makes sense for the working tree, not a custom `git diff` range.
        if toggles_commit && self.args.is_empty() {
            self.picker = None;
            self.commit = match self.commit {
                Some(_) => None,
                None => Some(CommitDialog::new(self.files.len())),
            };
            return Ok(true);
        }
        if (ctrl || cmd) && key.code == KeyCode::Char('p') {
            self.commit = None;
            if self.picker.take().is_none() {
                self.open_picker();
            }
            return Ok(true);
        }
        if let Some(dialog) = self.commit.as_mut() {
            match key.code {
                KeyCode::Esc => self.commit = None,
                KeyCode::Char('c') if ctrl => return Ok(false),
                _ => self.pending = dialog.on_key(key),
            }
            return Ok(true);
        }
        if let Some(picker) = self.picker.as_mut() {
            match key.code {
                KeyCode::Esc => self.picker = None,
                KeyCode::Enter => self.picker = None,
                KeyCode::Up => self.move_selection(-1),
                KeyCode::Down | KeyCode::Tab => self.move_selection(1),
                KeyCode::Char('k') if ctrl => self.move_selection(-1),
                KeyCode::Char('n' | 'j') if ctrl => self.move_selection(1),
                KeyCode::PageUp => self.move_selection(-10),
                KeyCode::PageDown => self.move_selection(10),
                KeyCode::Char('c') if ctrl => return Ok(false),
                KeyCode::Backspace => {
                    picker.query.pop();
                    self.filter_picker();
                }
                KeyCode::Char(c) if !ctrl && !cmd => {
                    picker.query.push(c);
                    self.filter_picker();
                }
                _ => {}
            }
            return Ok(true);
        }

        let page = self.height.max(2) as isize;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(false),
            KeyCode::Char('c') if ctrl => return Ok(false),
            KeyCode::Char('d') if ctrl => self.scroll_by(page / 2),
            KeyCode::Char('u') if ctrl => self.scroll_by(-page / 2),
            KeyCode::Char('f') if ctrl => self.scroll_by(page - 2),
            KeyCode::Char('b') if ctrl => self.scroll_by(-(page - 2)),
            KeyCode::Char('j') | KeyCode::Down => self.scroll_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.scroll_by(-1),
            KeyCode::Char('d') => self.scroll_by(page / 2),
            KeyCode::Char('u') => self.scroll_by(-page / 2),
            KeyCode::Char(' ') | KeyCode::PageDown => self.scroll_by(page - 2),
            KeyCode::Char('b') | KeyCode::PageUp => self.scroll_by(-(page - 2)),
            KeyCode::Char('g') | KeyCode::Home => self.scroll = 0,
            KeyCode::Char('G') | KeyCode::End => self.scroll = usize::MAX,
            KeyCode::Char('n' | ']') => self.jump_to_file(self.current_file() + 1),
            KeyCode::Char('p' | 'N' | '[') => {
                let file = self.current_file();
                // From mid-file, go to the top of the current file first.
                let target = if self.file_rows[file] < self.scroll { file } else { file.saturating_sub(1) };
                self.jump_to_file(target);
            }
            KeyCode::Char('h') | KeyCode::Left => self.hscroll = self.hscroll.saturating_sub(4),
            KeyCode::Char('l') | KeyCode::Right => self.hscroll += 4,
            KeyCode::Char('0') => self.hscroll = 0,
            KeyCode::Char('f') => self.open_picker(),
            KeyCode::Char('m') => self.set_collapsed(self.current_file(), true),
            KeyCode::Char('o') => self.set_collapsed(self.current_file(), false),
            KeyCode::Char('r') => self.reload()?,
            _ => {}
        }
        self.clamp_scroll();
        Ok(true)
    }

    fn perform(&mut self, action: Action) -> Result<()> {
        let Some(dialog) = self.commit.as_mut() else { return Ok(()) };
        dialog.run(action, &self.files, &self.repo.branch);
        self.reload()?;
        if let Some(dialog) = self.commit.as_mut() {
            dialog.reset_files(self.files.len());
        }
        Ok(())
    }

    fn on_mouse(&mut self, mouse: MouseEvent) {
        if let Some(dialog) = self.commit.as_mut() {
            self.pending = dialog.on_mouse(mouse);
            return;
        }
        let pos = Position::new(mouse.column, mouse.row);
        if let Some(picker) = self.picker.as_mut() {
            match mouse.kind {
                MouseEventKind::ScrollDown => self.move_selection(1),
                MouseEventKind::ScrollUp => self.move_selection(-1),
                MouseEventKind::Down(MouseButton::Left) if picker.list_area.contains(pos) => {
                    let index = picker.state.offset() + (mouse.row - picker.list_area.y) as usize;
                    if index < picker.matches.len() {
                        if picker.state.selected() == Some(index) {
                            self.picker = None;
                        } else {
                            picker.state.select(Some(index));
                            self.preview_selection();
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        match mouse.kind {
            MouseEventKind::ScrollDown => self.scroll_by(3),
            MouseEventKind::ScrollUp => self.scroll_by(-3),
            MouseEventKind::ScrollRight => self.hscroll += 4,
            MouseEventKind::ScrollLeft => self.hscroll = self.hscroll.saturating_sub(4),
            MouseEventKind::Down(MouseButton::Left) if self.body.contains(pos) => {
                let line = self.scroll + (mouse.row - self.body.y) as usize;
                // The top line is always a header: either the real one or the current file's pinned one.
                let file = if mouse.row == self.body.y {
                    Some(self.current_file())
                } else {
                    self.file_rows.iter().position(|&row| row == line)
                };
                if let Some(file) = file.filter(|_| !self.files.is_empty()) {
                    self.set_collapsed(file, !self.collapsed.contains(&self.files[file].path));
                }
            }
            _ => {}
        }
    }
}

/// Characters of `query` appear in order in `text`.
fn fuzzy_match(text: &str, query: &str) -> bool {
    let mut chars = text.chars();
    query.chars().all(|q| q == ' ' || chars.any(|c| c == q))
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|frame| view::draw(frame, app))?;
        if let Some(action) = app.pending.take() {
            app.perform(action)?;
            continue;
        }
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if !app.on_key(key)? {
                    return Ok(());
                }
            }
            Event::Mouse(mouse) => app.on_mouse(mouse),
            _ => {}
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let repo = git::open()?;
    let mut app = App {
        repo,
        args,
        files: Vec::new(),
        rows: Vec::new(),
        header_rows: Vec::new(),
        lines: Vec::new(),
        file_rows: Vec::new(),
        collapsed: HashSet::new(),
        body: Rect::default(),
        scroll: 0,
        hscroll: 0,
        wrap: 0,
        height: 0,
        picker: None,
        commit: None,
        pending: None,
        syntaxes: two_face::syntax::extra_newlines(),
        theme: view::theme(),
    };
    app.reload()?;

    let mut terminal = ratatui::init();
    // Needed for the terminal to report Cmd (SUPER) and Ctrl+/ distinctly.
    let enhanced = supports_keyboard_enhancement().unwrap_or(false);
    if enhanced {
        execute!(std::io::stdout(), PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES))?;
    }
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let result = run(&mut terminal, &mut app);
    execute!(std::io::stdout(), DisableMouseCapture)?;
    if enhanced {
        execute!(std::io::stdout(), PopKeyboardEnhancementFlags)?;
    }
    ratatui::restore();
    result
}
