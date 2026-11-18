use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::Stylize,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::view::{FAINT, MUTED, keycap, panel};

type Section = (&'static str, &'static [(&'static str, &'static str)]);

/// Every shortcut, shared by the in-app overlay and `tg help`.
const SECTIONS: &[Section] = &[
    ("Navigate", &[("j k", "Next / previous file"), ("d u", "Half page"), ("space b", "Full page"), ("g G", "Top / bottom"), ("h l", "Scroll sideways")]),
    ("Files", &[("m o", "Collapse / open file"), ("⌘O", "Find a file"), ("r", "Reload")]),
    ("Review", &[("v", "Select lines (or drag)"), ("j k", "Extend the selection"), ("c", "Comment to an agent")]),
    ("Git", &[("⌘/", "Open the commit dialog"), ("⏎", "Commit"), ("⌘\\", "Commit and push"), ("⌘U", "Push")]),
    ("General", &[("⌥/", "Shortcuts"), ("q", "Quit")]),
];

/// Width of the keycap column, so descriptions line up.
const KEYS_WIDTH: usize = 12;

/// The shortcut list for `tg help`.
pub fn text() -> String {
    SECTIONS
        .iter()
        .map(|(title, items)| {
            let items: String = items.iter().map(|(keys, label)| format!("  {keys:<10}{label}\n")).collect();
            format!("{title}:\n{items}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `spaced` puts a blank row between shortcuts so stacked keycaps don't touch.
fn section_lines(sections: &[Section], spaced: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (i, (title, items)) in sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        lines.push(Line::from(Span::raw(title.to_uppercase()).fg(FAINT).bold()));
        for (j, (keys, label)) in items.iter().enumerate() {
            if spaced && j > 0 {
                lines.push(Line::default());
            }
            let mut spans = Vec::new();
            for key in keys.split(' ') {
                spans.extend(keycap(key));
                spans.push(Span::raw(" "));
            }
            let used: usize = spans.iter().map(Span::width).sum();
            spans.push(Span::raw(" ".repeat(KEYS_WIDTH.saturating_sub(used))));
            spans.push(Span::raw(*label).fg(MUTED));
            lines.push(Line::from(spans));
        }
    }
    lines
}

pub fn draw(frame: &mut Frame, area: Rect) {
    // Two columns when there's room, otherwise one; spaced rows when they fit the height.
    let layout = |spaced| -> Vec<Vec<Line>> {
        if area.width >= 80 {
            vec![section_lines(&SECTIONS[..2], spaced), section_lines(&SECTIONS[2..], spaced)]
        } else {
            vec![section_lines(SECTIONS, spaced)]
        }
    };
    let fits = |columns: &Vec<Vec<Line>>| columns.iter().map(Vec::len).max().unwrap_or(0) + 4 <= area.height as usize;
    let spaced = layout(true);
    let columns = if fits(&spaced) { spaced } else { layout(false) };
    let rows = columns.iter().map(Vec::len).max().unwrap_or(0) as u16;
    let width = if columns.len() == 2 { 78 } else { 46.min(area.width.saturating_sub(4)) };
    let [popup] = Layout::horizontal([Constraint::Length(width)]).flex(Flex::Center).areas(area);
    let [popup] = Layout::vertical([Constraint::Length((rows + 4).min(area.height))]).flex(Flex::Center).areas(popup);

    let inner = panel(frame, popup, "Shortcuts", Line::default());

    let areas = Layout::horizontal(vec![Constraint::Fill(1); columns.len()]).spacing(4).split(inner);
    for (lines, area) in columns.into_iter().zip(areas.iter()) {
        frame.render_widget(Paragraph::new(lines), *area);
    }
}
