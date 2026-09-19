//! Settings, Updates and Help: quiet text screens.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

use super::{VERSION, field, panel, rule};
use crate::app::{App, Section};
use crate::theme;

pub fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let lang = app.lang;
    let block = panel(lang.t(app.section.title()), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let body = Rect { x: inner.x + 1, y: inner.y + 1, width: inner.width.saturating_sub(2), height: inner.height.saturating_sub(1) };

    let lines = match app.section {
        Section::Settings => settings(app, body.width),
        Section::Updates => vec![
            field(lang.t("Installed version"), VERSION),
            Line::default(),
            Line::styled(lang.t("Updates arrive through your package manager or the GitHub releases."), theme::muted()),
        ],
        _ => vec![
            rule(lang.t("About TorroMail"), body.width),
            field(lang.t("Version"), VERSION),
            field(lang.t("Data directory"), app.snapshot.data_directory.display().to_string()),
            Line::default(),
            field(lang.t("Documentation"), "https://github.com/mahype/TorroMail"),
            field(lang.t("Report a problem"), "https://github.com/mahype/TorroMail/issues"),
            Line::default(),
            Line::styled(
                lang.t("Every action an assistant takes is recorded in the log — that is the first place to look when something went differently than expected."),
                theme::muted(),
            ),
        ],
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
}

/// Language, the background check and its notifications, then the two jumps
/// the macOS app offers here too. The row order is `App::on_settings_key`'s.
fn settings(app: &App, width: u16) -> Vec<Line<'static>> {
    let lang = app.lang;
    let row = |index: usize, line: Line<'static>| if index == app.settings_index { line.style(theme::selected()) } else { line };
    let check = |on: bool, label: &'static str| {
        Line::from(vec![
            Span::styled("[", theme::faint()),
            Span::styled(if on { "✓" } else { " " }, Style::new().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
            Span::styled("] ", theme::faint()),
            Span::raw(lang.t(label)),
        ])
    };
    let jump = |label: &'static str| Line::from(vec![Span::raw(lang.t(label)), Span::styled("  ›", theme::faint())]);
    let language = match app.settings.language {
        None => lang.t("System"),
        Some(crate::i18n::Lang::De) => "Deutsch",
        Some(crate::i18n::Lang::En) => "English",
    };

    let mut lines = vec![
        rule(lang.t("General"), width),
        row(0, Line::from(vec![
            Span::styled(format!("{:<17} ", lang.t("Language")), theme::muted()),
            Span::styled("‹ ", theme::faint()),
            Span::styled(language, theme::bold()),
            Span::styled(" ›", theme::faint()),
        ])),
        Line::default(),
        rule(lang.t("In the background"), width),
        row(1, check(app.snapshot.autocheck, "Check the accounts every 15 minutes")),
        Line::styled(
            format!("    {}", lang.t("Also while this window is closed, so a broken account shows up before an assistant trips over it.")),
            theme::faint(),
        ),
        row(2, check(app.settings.notifications, "Notify me when an account stops working")),
        Line::styled(format!("    {}", lang.t("And again when it is reachable. Needs the background check.")), theme::faint()),
        Line::default(),
        rule(lang.t("MCP Clients"), width),
        row(3, jump("Set up assistants…")),
        Line::default(),
        rule(lang.t("Updates"), width),
        row(4, jump("Open Updates…")),
        Line::default(),
    ];
    if let Some(message) = &app.message {
        let colour = if message.is_error { theme::ACCENT } else { theme::GREEN };
        lines.push(Line::styled(lang.t(message.text), Style::new().fg(colour).add_modifier(Modifier::BOLD)));
        if !message.detail.is_empty() {
            lines.push(Line::styled(message.detail.clone(), theme::muted()));
        }
    }
    lines
}
