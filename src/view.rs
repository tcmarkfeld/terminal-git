use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph},
};
use two_face::re_exports::syntect::{
    easy::HighlightLines,
    highlighting::{Color as SyntaxColor, ScopeSelectors, StyleModifier, Theme, ThemeItem},
    parsing::SyntaxSet,
};

use crate::App;
use std::ops::Range;

use crate::git::{self, FileDiff, Kind, Status};

pub const BG: Color = Color::Rgb(19, 20, 23);
pub const SURFACE: Color = Color::Rgb(28, 29, 33);
pub const RAISED: Color = Color::Rgb(40, 42, 49);
/// Text inputs inside popups.
pub const FIELD: Color = Color::Rgb(46, 48, 57);
/// Popup panels: lighter than anything in the diff, which is darkened behind them.
const POPUP: Color = Color::Rgb(34, 35, 42);
pub const TEXT: Color = Color::Rgb(228, 229, 233);
pub const MUTED: Color = Color::Rgb(138, 143, 152);
pub const FAINT: Color = Color::Rgb(84, 88, 98);
pub const ACCENT: Color = Color::Rgb(133, 125, 255);
const SELECT_BG: Color = Color::Rgb(55, 51, 122);
const ADD_BG: Color = Color::Rgb(27, 48, 33);
pub const ADD_FG: Color = Color::Rgb(108, 207, 138);
/// The words that changed within a replaced line.
const ADD_EMPHASIS: Color = Color::Rgb(44, 84, 55);
const DEL_EMPHASIS: Color = Color::Rgb(98, 40, 47);
const DEL_BG: Color = Color::Rgb(56, 29, 33);
pub const DEL_FG: Color = Color::Rgb(240, 116, 122);
const MOD_FG: Color = Color::Rgb(232, 190, 102);
const REN_FG: Color = Color::Rgb(110, 168, 254);

/// Syntax colors sampled from Linear's diff view.
pub fn theme() -> Theme {
    const KEYWORD: u32 = 0xDFA0CF;
    const VARIABLE: u32 = 0xE3BF8A;
    const STRING: u32 = 0xE6D19E;
    const FUNCTION: u32 = 0x85C8DA;
    const TYPE: u32 = 0x8CB4F0;
    const NUMBER: u32 = 0xA9B1FF;
    const COMMENT: u32 = 0x8B9099;
    const PUNCTUATION: u32 = 0xD2D4DA;
    let rules: [(&str, u32); 11] = [
        ("comment, punctuation.definition.comment", COMMENT),
        ("string, punctuation.definition.string", STRING),
        ("constant.numeric", NUMBER),
        ("keyword, storage, keyword.control, entity.name.tag", KEYWORD),
        ("punctuation, keyword.operator, meta.brace", PUNCTUATION),
        ("variable, variable.parameter, variable.other, meta.property, support.variable, entity.other.attribute-name", VARIABLE),
        ("entity.name.function, support.function, variable.function, meta.function-call entity.name", FUNCTION),
        ("entity.name.type, entity.name.class, support.type, support.class, storage.type.primitive, constant.language, variable.other.constant, entity.name.namespace", TYPE),
        ("markup.heading, markup.bold", FUNCTION),
        ("storage.type.function.arrow", PUNCTUATION),
        ("support.type.object, support.class.console, support.variable.object", VARIABLE),
    ];
    let color = |hex: u32| SyntaxColor { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8, a: 0xFF };
    let mut theme = Theme::default();
    theme.settings.foreground = Some(color(PUNCTUATION));
    theme.scopes = rules
        .iter()
        .map(|(scope, hex)| ThemeItem {
            scope: scope.parse::<ScopeSelectors>().expect("valid scope selector"),
            style: StyleModifier { foreground: Some(color(*hex)), background: None, font_style: None },
        })
        .collect();
    theme
}

pub enum Row {
    Header(usize),
    /// Bottom half-row of the header card.
    HeaderEnd,
    Gap { count: u32, context: String },
    Code { kind: Kind, num: u32, spans: Vec<(Color, String)>, changed: Option<Range<usize>> },
    Note(&'static str),
    /// Spacer above a header; its lower half is the card's top edge.
    Blank,
}

pub fn build_rows(files: &[FileDiff], syntaxes: &SyntaxSet, theme: &Theme) -> (Vec<Row>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut file_rows = Vec::new();
    for (i, file) in files.iter().enumerate() {
        rows.push(Row::Blank);
        file_rows.push(rows.len());
        rows.push(Row::Header(i));
        rows.push(Row::HeaderEnd);
        if file.binary {
            rows.push(Row::Note("Binary file"));
        } else if file.hunks.is_empty() {
            rows.push(Row::Note("No content changes"));
        }

        let name = file.path.rsplit('/').next().unwrap_or(&file.path);
        let syntax = name
            .rsplit_once('.')
            .and_then(|(_, ext)| syntaxes.find_syntax_by_extension(ext))
            .or_else(|| syntaxes.find_syntax_by_extension(name))
            .unwrap_or_else(|| syntaxes.find_syntax_plain_text());

        let mut next_old = 1;
        for hunk in &file.hunks {
            let count = hunk.old_start.saturating_sub(next_old);
            if count > 0 || !hunk.context.is_empty() {
                rows.push(Row::Gap { count, context: hunk.context.clone() });
            }
            next_old = hunk.old_start + hunk.old_len;

            let mut highlighter = HighlightLines::new(syntax, theme);
            for (line, changed) in hunk.lines.iter().zip(git::inline_changes(&hunk.lines)) {
                let spans = highlighter
                    .highlight_line(&format!("{}\n", line.text), syntaxes)
                    .map(|ranges| {
                        ranges
                            .into_iter()
                            .map(|(style, text)| {
                                let fg = style.foreground;
                                (Color::Rgb(fg.r, fg.g, fg.b), text.trim_end_matches('\n').to_string())
                            })
                            .collect()
                    })
                    .unwrap_or_else(|_| vec![(TEXT, line.text.clone())]);
                let num = if line.kind == Kind::Removed { line.old } else { line.new };
                rows.push(Row::Code { kind: line.kind, num, spans, changed });
            }
        }
    }
    (rows, file_rows)
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Block::new().bg(BG), area);
    let [top, gap, body, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)])
            .areas(area);
    app.height = body.height as usize;
    app.body = body;
    app.set_width(body.width);
    app.clamp_scroll();

    draw_top_bar(frame, app, top);
    draw_footer(frame, app, bottom);

    if app.files.is_empty() {
        let msg = Paragraph::new("No changes").fg(MUTED).centered();
        frame.render_widget(msg, Rect { y: body.y + body.height / 2, height: 1, ..body });
    } else {
        draw_body(frame, app, body);
        // A header at the top (real or pinned) borrows the row above as its card's top edge.
        let file = app.current_file();
        let top = app.lines.get(app.scroll).map(|line| &app.rows[line.0]);
        if app.file_rows[file] < app.scroll || matches!(top, Some(Row::Header(_))) {
            draw_row(frame, app, &Row::Blank, 0, gap, 0, false);
        }
    }
    if app.picker.is_some() || app.commit.is_some() || app.comment.is_some() || app.help {
        dim_backdrop(frame);
    }
    if app.picker.is_some() {
        draw_picker(frame, app, area);
    }
    if let Some(dialog) = app.commit.as_mut() {
        crate::commit::draw(frame, dialog, &app.files, area);
    }
    if let Some(dialog) = app.comment.as_mut() {
        crate::comment::draw(frame, dialog, area);
    }
    if app.help {
        crate::help::draw(frame, area);
    }
}

/// Keyboard hints as small keycaps, for the last row of a popup.
pub fn key_hints(items: &[(&str, &str)]) -> Line<'static> {
    let spans: Vec<Span> = items
        .iter()
        .flat_map(|(key, label)| keycap(key).into_iter().chain([Span::raw(format!(" {label}   ")).fg(MUTED)]))
        .collect();
    Line::from(spans)
}

/// A key drawn as a pill: half-cell blocks round off the ends.
pub fn keycap(key: &str) -> [Span<'static>; 3] {
    [Span::raw("▐").fg(RAISED), Span::raw(key.to_string()).fg(TEXT).bg(RAISED), Span::raw("▌").fg(RAISED)]
}

/// A popup surface with even padding on every side. A cell is about twice as tall as it is wide, so
/// the edges are half a row at the top and bottom and a full column at the sides. Draws the title
/// row and returns the content area below it.
pub fn panel(frame: &mut Frame, area: Rect, title: &str, aside: Line) -> Rect {
    if area.height < 4 || area.width < 4 {
        return Rect::default();
    }
    let fill = Rect { y: area.y + 1, height: area.height - 2, ..area };
    frame.render_widget(Clear, fill);
    frame.render_widget(Block::new().bg(POPUP), fill);
    // Edge cells keep the backdrop's background; the half blocks paint the panel's side of the cell.
    let buf = frame.buffer_mut();
    for x in area.left()..area.right() {
        for (y, symbol) in [(area.top(), "▄"), (area.bottom() - 1, "▀")] {
            let cell = &mut buf[(x, y)];
            cell.set_symbol(symbol).set_fg(POPUP);
            cell.modifier = Modifier::empty();
        }
    }
    let inner = Rect { x: area.x + 1, width: area.width - 2, ..fill };
    let [title_row, _, body] = Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    frame.render_widget(Span::raw(title.to_string()).fg(TEXT).bold(), title_row);
    frame.render_widget(aside.right_aligned(), title_row);
    body
}

/// Darkens everything drawn so far to 30% brightness, so a popup on top of it stands out.
fn dim_backdrop(frame: &mut Frame) {
    let scale = |c: u8| (u16::from(c) * 3 / 10) as u8;
    let darken = |color: Color| match color {
        Color::Rgb(r, g, b) => Color::Rgb(scale(r), scale(g), scale(b)),
        other => other,
    };
    for cell in &mut frame.buffer_mut().content {
        cell.fg = darken(cell.fg);
        cell.bg = darken(cell.bg);
    }
}

pub fn status_letter(status: Status) -> (&'static str, Color) {
    match status {
        Status::Modified => ("M", MOD_FG),
        Status::Added => ("A", ADD_FG),
        Status::Deleted => ("D", DEL_FG),
        Status::Renamed => ("R", REN_FG),
    }
}

fn draw_top_bar(frame: &mut Frame, app: &App, area: Rect) {
    let added: usize = app.files.iter().map(|f| f.added).sum();
    let removed: usize = app.files.iter().map(|f| f.removed).sum();
    let left = Line::from(vec![
        Span::raw("  "),
        Span::raw(&app.repo.name).fg(TEXT).bold(),
        Span::raw("  ›  ").fg(FAINT),
        Span::raw(&app.repo.branch).fg(MUTED),
        Span::raw(if app.args.is_empty() { String::new() } else { format!("  {}", app.args.join(" ")) }).fg(FAINT),
        Span::raw(format!("   +{added}")).fg(ADD_FG),
        Span::raw(format!(" -{removed}")).fg(DEL_FG),
    ]);
    frame.render_widget(left, area);
    if !app.files.is_empty() {
        let right = Line::from(vec![
            Span::raw(format!("{}", app.current_file() + 1)).fg(TEXT),
            Span::raw(format!(" / {} files  ", app.files.len())).fg(MUTED),
        ]);
        frame.render_widget(right.right_aligned(), area);
    }
}

/// Quiet by default: a confirmation on the left, one contextual hint on the right.
fn draw_footer(frame: &mut Frame, app: &mut App, area: Rect) {
    if let Some(notice) = &app.notice {
        frame.render_widget(Line::from(Span::raw(format!("  ✓ {notice}")).fg(ADD_FG)), area);
    }
    let hints: &[(&str, &str)] = if app.selection.is_some() { &[("c", "comment"), ("esc", "cancel")] } else { &[("⌥/", "shortcuts")] };
    let spans: Vec<Span> = hints
        .iter()
        .flat_map(|(key, label)| [Span::raw(*key).fg(MUTED), Span::raw(format!(" {label}  ")).fg(FAINT)])
        .collect();
    let hint = Line::from(spans);
    let width = hint.width() as u16;
    app.help_hint = if app.selection.is_some() { Rect::default() } else { Rect { x: area.right().saturating_sub(width), width, ..area } };
    frame.render_widget(hint.right_aligned(), area);
}

fn draw_body(frame: &mut Frame, app: &App, area: Rect) {
    let width = num_width(&app.rows);
    let selected = app.selection.map(|(anchor, cursor)| anchor.min(cursor)..=anchor.max(cursor));
    for (i, &(row, segment)) in app.lines.iter().skip(app.scroll).take(area.height as usize).enumerate() {
        let rect = Rect { y: area.y + i as u16, height: 1, ..area };
        let is_selected = selected.as_ref().is_some_and(|range| range.contains(&row));
        draw_row(frame, app, &app.rows[row], segment, rect, width, is_selected);
    }
    // Sticky header: keep the current file's card pinned while scrolling through it.
    let file = app.current_file();
    if app.file_rows[file] < app.scroll {
        draw_row(frame, app, &Row::Header(file), 0, Rect { height: 1, ..area }, width, false);
    }
}

/// How many screen lines a row takes when code wraps at `wrap` columns (0 = no wrapping).
pub fn segments(row: &Row, wrap: usize) -> usize {
    match row {
        Row::Code { spans, .. } if wrap > 0 => spans.iter().map(|(_, text)| text.chars().count()).sum::<usize>().div_ceil(wrap).max(1),
        _ => 1,
    }
}

pub fn num_width(rows: &[Row]) -> usize {
    let max = rows.iter().filter_map(|r| if let Row::Code { num, .. } = r { Some(*num) } else { None }).max();
    max.unwrap_or(0).to_string().len().max(3)
}

fn draw_row(frame: &mut Frame, app: &App, row: &Row, segment: usize, area: Rect, width: usize, selected: bool) {
    match row {
        Row::Header(i) => {
            let file = &app.files[*i];
            let (dir, name) = file.path.rsplit_once('/').map_or(("", file.path.as_str()), |(d, n)| (d, n));
            let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
            let badge: String = ext.chars().take(3).collect::<String>().to_uppercase();
            let (tag, tag_fg) = match file.status {
                Status::Added => ("new", ADD_FG),
                Status::Deleted => ("deleted", DEL_FG),
                Status::Renamed => ("renamed", REN_FG),
                Status::Modified => ("", MUTED),
            };
            let chevron = if app.collapsed.contains(&file.path) { " ▸ " } else { " ▾ " };
            let mut spans = vec![
                Span::raw(chevron).fg(MUTED),
                Span::raw(format!(" {:^3} ", if badge.is_empty() { "·" } else { &badge })).fg(MUTED).bg(RAISED),
                Span::raw("  "),
                Span::raw(name).fg(TEXT).bold(),
                Span::raw(format!("  {dir}")).fg(MUTED),
            ];
            if let Some(old) = &file.old_path {
                spans.push(Span::raw(format!("  ← {old}")).fg(FAINT));
            }
            if !tag.is_empty() {
                spans.push(Span::raw(format!("  {tag}")).fg(tag_fg));
            }
            let mut stats = Vec::new();
            if file.added > 0 {
                stats.push(Span::raw(format!("+{}", file.added)).fg(ADD_FG));
            }
            if file.removed > 0 {
                stats.push(Span::raw(format!(" -{}", file.removed)).fg(DEL_FG));
            }
            stats.push(Span::raw("  "));
            frame.render_widget(Clear, area);
            frame.render_widget(Line::from(spans).bg(SURFACE), area);
            frame.render_widget(Line::from(stats).right_aligned(), area);
        }
        Row::Gap { count, context } => {
            let label = match count {
                0 => String::new(),
                1 => "1 unchanged line".into(),
                n => format!("{n} unchanged lines"),
            };
            let line = Line::from(vec![
                Span::raw(format!(" {:>width$}  ", "⋯")).fg(FAINT),
                Span::raw(label).fg(MUTED),
                Span::raw(if context.is_empty() { "" } else { "   " }),
                Span::raw(context.as_str()).fg(FAINT).italic(),
            ]);
            frame.render_widget(line.bg(SURFACE), area);
        }
        Row::Code { kind, num, spans, changed } => {
            let (bg, bar, num_fg) = match kind {
                _ if selected => (SELECT_BG, Span::raw("▌").fg(ACCENT), ACCENT),
                Kind::Added => (ADD_BG, Span::raw("▌").fg(ADD_FG), ADD_FG),
                Kind::Removed => (DEL_BG, Span::raw("▌").fg(DEL_FG), DEL_FG),
                Kind::Context => (BG, Span::raw(" "), FAINT),
            };
            // Deleted lines show a dash instead of their old number; wrapped continuation lines show nothing.
            let num = match (segment, kind) {
                (0, Kind::Removed) => "–".to_string(),
                (0, _) => num.to_string(),
                _ => String::new(),
            };
            let num = Span::raw(format!("{num:>width$}   ")).fg(num_fg);
            let mut out = vec![bar, if selected { num.bold() } else { num }];
            let start = if app.wrap > 0 { segment * app.wrap } else { app.hscroll };
            let emphasis = if *kind == Kind::Added { ADD_EMPHASIS } else { DEL_EMPHASIS };
            let mut at = 0;
            for (fg, text) in spans {
                let styled = |text: String, emphasized: bool| {
                    let span = Span::raw(text).fg(*fg);
                    if emphasized { span.bg(emphasis) } else { span }
                };
                // Split the span where the changed words start and end.
                let (mut chunk, mut chunk_emphasized) = (String::new(), false);
                for c in text.chars() {
                    if at >= start {
                        let emphasized = !selected && changed.as_ref().is_some_and(|range| range.contains(&at));
                        if emphasized != chunk_emphasized && !chunk.is_empty() {
                            out.push(styled(std::mem::take(&mut chunk), chunk_emphasized));
                        }
                        chunk_emphasized = emphasized;
                        chunk.push(c);
                    }
                    at += 1;
                }
                if !chunk.is_empty() {
                    out.push(styled(chunk, chunk_emphasized));
                }
            }
            frame.render_widget(Line::from(out).bg(bg), area);
        }
        Row::Note(text) => {
            let line = Line::from(vec![Span::raw(" ".repeat(width + 4)), Span::raw(*text).fg(FAINT).italic()]);
            frame.render_widget(line, area);
        }
        // Half blocks give the header card half a row of padding above and below.
        Row::Blank => frame.render_widget(Line::from("▄".repeat(area.width as usize)).fg(SURFACE), area),
        Row::HeaderEnd => frame.render_widget(Line::from("▀".repeat(area.width as usize)).fg(SURFACE), area),
    }
}

fn draw_picker(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(picker) = app.picker.as_mut() else { return };
    let width = area.width.saturating_sub(8).min(90);
    let height = (app.files.len() as u16 + 6).min(area.height.saturating_sub(4)).max(8);
    let [popup] = Layout::horizontal([Constraint::Length(width)]).flex(ratatui::layout::Flex::Center).areas(area);
    let [popup] = Layout::vertical([Constraint::Length(height)]).flex(ratatui::layout::Flex::Center).areas(popup);

    let inner = panel(frame, popup, "Files", Line::from(picker.matches.len().to_string()).fg(MUTED));

    let [input, _, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let query = if picker.query.is_empty() {
        Span::raw("Filter files…").fg(FAINT)
    } else {
        Span::raw(picker.query.as_str()).fg(TEXT)
    };
    frame.render_widget(Line::from(vec![Span::raw(" › ").fg(ACCENT), query, Span::raw("▏").fg(ACCENT)]), input);

    let items: Vec<ListItem> = picker
        .matches
        .iter()
        .map(|&i| {
            let file = &app.files[i];
            let (dir, name) = file.path.rsplit_once('/').map_or(("", file.path.as_str()), |(d, n)| (d, n));
            let (letter, fg) = status_letter(file.status);
            let stats = format!("+{} -{}", file.added, file.removed);
            let used = 4 + name.chars().count() + 2 + dir.chars().count() + stats.len() + 2;
            let pad = (list_area.width as usize).saturating_sub(used);
            ListItem::new(Line::from(vec![
                Span::raw(format!(" {letter}  ")).fg(fg),
                Span::raw(name).fg(TEXT),
                Span::raw(format!("  {dir}")).fg(MUTED),
                Span::raw(" ".repeat(pad)),
                Span::raw(format!("+{}", file.added)).fg(ADD_FG),
                Span::raw(format!(" -{}", file.removed)).fg(DEL_FG),
                Span::raw("  "),
            ]))
        })
        .collect();
    let list = List::new(items).highlight_style(Style::new().bg(RAISED));
    frame.render_stateful_widget(list, list_area, &mut picker.state);
    picker.list_area = list_area;
}
