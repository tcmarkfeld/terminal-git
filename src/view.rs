use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Clear, List, ListItem, Paragraph},
};
use two_face::re_exports::syntect::{
    easy::HighlightLines,
    highlighting::{Color as SyntaxColor, ScopeSelectors, StyleModifier, Theme, ThemeItem},
    parsing::SyntaxSet,
};

use crate::App;
use crate::git::{FileDiff, Kind, Status};

pub const BG: Color = Color::Rgb(19, 20, 23);
pub const SURFACE: Color = Color::Rgb(28, 29, 33);
pub const RAISED: Color = Color::Rgb(40, 42, 49);
pub const TEXT: Color = Color::Rgb(228, 229, 233);
pub const MUTED: Color = Color::Rgb(138, 143, 152);
pub const FAINT: Color = Color::Rgb(84, 88, 98);
pub const ACCENT: Color = Color::Rgb(133, 125, 255);
const ADD_BG: Color = Color::Rgb(27, 48, 33);
pub const ADD_FG: Color = Color::Rgb(108, 207, 138);
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
    Gap { count: u32, context: String },
    Code { kind: Kind, num: u32, spans: Vec<(Color, String)> },
    Note(&'static str),
    Blank,
}

pub fn build_rows(files: &[FileDiff], syntaxes: &SyntaxSet, theme: &Theme) -> (Vec<Row>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut file_rows = Vec::new();
    for (i, file) in files.iter().enumerate() {
        if i > 0 {
            rows.push(Row::Blank);
        }
        file_rows.push(rows.len());
        rows.push(Row::Header(i));
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
            for line in &hunk.lines {
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
                rows.push(Row::Code { kind: line.kind, num, spans });
            }
        }
    }
    (rows, file_rows)
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    frame.render_widget(Block::new().bg(BG), area);
    let [top, _, body, bottom] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)])
            .areas(area);
    app.height = body.height as usize;
    app.body = body;
    app.set_width(body.width);
    app.clamp_scroll();

    draw_top_bar(frame, app, top);
    draw_footer(frame, bottom);

    if app.files.is_empty() {
        let msg = Paragraph::new("No changes").fg(MUTED).centered();
        frame.render_widget(msg, Rect { y: body.y + body.height / 2, height: 1, ..body });
    } else {
        draw_body(frame, app, body);
    }
    if app.picker.is_some() {
        draw_picker(frame, app, area);
    }
    if let Some(dialog) = app.commit.as_mut() {
        crate::commit::draw(frame, dialog, &app.files, area);
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

fn draw_footer(frame: &mut Frame, area: Rect) {
    let hints = [("⌘P", "files"), ("⌘/", "git"), ("j k", "scroll"), ("n p", "next/prev file"), ("m o", "collapse/open"), ("h l", "pan"), ("r", "reload"), ("q", "quit")];
    let spans: Vec<Span> = std::iter::once(Span::raw("  "))
        .chain(hints.iter().flat_map(|(key, label)| [Span::raw(*key).fg(MUTED), Span::raw(format!(" {label}    ")).fg(FAINT)]))
        .collect();
    frame.render_widget(Line::from(spans), area);
}

fn draw_body(frame: &mut Frame, app: &App, area: Rect) {
    let width = num_width(&app.rows);
    for (i, &(row, segment)) in app.lines.iter().skip(app.scroll).take(area.height as usize).enumerate() {
        let rect = Rect { y: area.y + i as u16, height: 1, ..area };
        draw_row(frame, app, &app.rows[row], segment, rect, width);
    }
    // Sticky header: keep the current file's card pinned while scrolling through it.
    let file = app.current_file();
    if app.file_rows[file] < app.scroll {
        draw_row(frame, app, &Row::Header(file), 0, Rect { height: 1, ..area }, width);
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

fn draw_row(frame: &mut Frame, app: &App, row: &Row, segment: usize, area: Rect, width: usize) {
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
        Row::Code { kind, num, spans } => {
            let (bg, bar, num_fg) = match kind {
                Kind::Added => (ADD_BG, Span::raw("▌").fg(ADD_FG), ADD_FG),
                Kind::Removed => (DEL_BG, Span::raw("▌").fg(DEL_FG), DEL_FG),
                Kind::Context => (BG, Span::raw(" "), FAINT),
            };
            // Wrapped continuation lines leave the number blank.
            let num = if segment == 0 { num.to_string() } else { String::new() };
            let mut out = vec![bar, Span::raw(format!("{num:>width$}   ")).fg(num_fg)];
            let mut skip = if app.wrap > 0 { segment * app.wrap } else { app.hscroll };
            for (fg, text) in spans {
                let n = text.chars().count();
                if skip >= n {
                    skip -= n;
                    continue;
                }
                let text: String = text.chars().skip(skip).collect();
                skip = 0;
                let span = Span::raw(text).fg(*fg);
                out.push(if *kind == Kind::Removed { span.add_modifier(Modifier::DIM) } else { span });
            }
            frame.render_widget(Line::from(out).bg(bg), area);
        }
        Row::Note(text) => {
            let line = Line::from(vec![Span::raw(" ".repeat(width + 4)), Span::raw(*text).fg(FAINT).italic()]);
            frame.render_widget(line, area);
        }
        Row::Blank => {}
    }
}

fn draw_picker(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(picker) = app.picker.as_mut() else { return };
    let width = area.width.saturating_sub(8).min(90);
    let height = (app.files.len() as u16 + 4).min(area.height.saturating_sub(4)).max(6);
    let [popup] = Layout::horizontal([Constraint::Length(width)]).flex(ratatui::layout::Flex::Center).areas(area);
    let [popup] = Layout::vertical([Constraint::Length(height)]).flex(ratatui::layout::Flex::Center).areas(popup);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(RAISED))
        .bg(SURFACE)
        .title(Line::from(vec![
            Span::raw(" Files ").fg(TEXT).bold(),
            Span::raw(format!("{} ", picker.matches.len())).fg(MUTED),
        ]));
    let inner = block.inner(popup);
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);

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
