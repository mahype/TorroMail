//! Drawing. The frame around every screen — brand bar, menu, key hints — is
//! here; each section draws its own content into what is left.

mod accounts;
mod clients;
mod log;
mod overview;
mod simple;
mod wizard;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::app::{App, Section};
use crate::data::AccountHealth;
use crate::i18n::Lang;
use crate::theme;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const MENU_WIDTH: u16 = 26;

pub fn draw(frame: &mut Frame<'_>, app: &App) {
    let [bar, body, hints] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
    let [menu, content] = Layout::horizontal([Constraint::Length(MENU_WIDTH), Constraint::Min(0)]).areas(body);

    draw_bar(frame, bar);
    draw_menu(frame, menu, app);
    match app.section {
        Section::Overview => overview::draw(frame, content, app),
        Section::Accounts => accounts::draw(frame, content, app),
        Section::Clients => clients::draw(frame, content, app),
        Section::Log => log::draw(frame, content, app),
        Section::Settings | Section::Updates | Section::Help => simple::draw(frame, content, app),
    }
    if let Some(wizard) = &app.wizard {
        wizard::draw(frame, body, app.lang, wizard);
    }
    frame.render_widget(Paragraph::new(key_hints(app)), hints);
}

fn draw_bar(frame: &mut Frame<'_>, area: Rect) {
    let on_red = Style::new().bg(theme::RED).add_modifier(Modifier::BOLD);
    let version = format!("v{VERSION}  ");
    let wordmark = "  \\_ TORROMAIL _/";
    let gap = usize::from(area.width).saturating_sub(wordmark.len() + version.len());
    let line = Line::from(vec![
        Span::styled("  \\_ TORRO", on_red.fg(ratatui::style::Color::White)),
        Span::styled("MAIL", on_red.fg(theme::SILVER)),
        Span::styled(" _/", on_red.fg(ratatui::style::Color::White)),
        Span::styled(" ".repeat(gap), on_red),
        Span::styled(version, Style::new().bg(theme::RED).fg(ratatui::style::Color::Rgb(255, 210, 206))),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_menu(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let block = panel(app.lang.t("Menu"), false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let width = usize::from(inner.width);
    let mut lines = vec![Line::default()];
    for (index, section) in Section::ALL.iter().enumerate() {
        let label = format!(" {}  {}", index + 1, app.lang.t(section.title()));
        let padded = format!("{label:<width$}");
        lines.push(if *section == app.section {
            Line::styled(padded, Style::new().bg(theme::RED).fg(ratatui::style::Color::White).add_modifier(Modifier::BOLD))
        } else {
            Line::from(vec![
                Span::styled(format!(" {}  ", index + 1), theme::faint()),
                Span::raw(app.lang.t(section.title())),
            ])
        });
    }
    frame.render_widget(Paragraph::new(lines), inner);

    if inner.height > 12 {
        let tagline = Rect { x: inner.x + 1, y: inner.y + inner.height - 2, width: inner.width.saturating_sub(2), height: 2 };
        frame.render_widget(
            Paragraph::new(app.lang.t("Your mailboxes for AI assistants."))
                .style(theme::faint())
                .wrap(ratatui::widgets::Wrap { trim: true }),
            tagline,
        );
    }
}

fn key_hints(app: &App) -> Line<'static> {
    let lang = app.lang;
    let mut hints: Vec<(&str, &str)> = Vec::new();
    if let Some(wizard) = &app.wizard {
        hints.extend([("tab", "next field"), ("enter", "continue")]);
        if wizard.step == crate::wizard::Step::SignIn {
            hints.push((lang.t("ctrl+d"), "server details"));
        }
        hints.push(("esc", "back"));
        let mut spans = vec![Span::raw(" ")];
        for (key, label) in hints {
            spans.push(Span::styled(format!(" {key} "), Style::new().bg(theme::KEY).add_modifier(Modifier::BOLD)));
            spans.push(Span::styled(format!(" {}   ", lang.t(label)), theme::muted()));
        }
        return Line::from(spans);
    }
    match app.section {
        Section::Accounts if app.focus == crate::app::Focus::Detail => {
            hints.extend([("↑↓", "select"), (lang.t("space"), "toggle"), (lang.t("ctrl+s"), "save"), ("esc", "back")]);
            let mut spans = vec![Span::raw(" ")];
            for (key, label) in hints {
                spans.push(Span::styled(format!(" {key} "), Style::new().bg(theme::KEY).add_modifier(Modifier::BOLD)));
                spans.push(Span::styled(format!(" {}   ", lang.t(label)), theme::muted()));
            }
            return Line::from(spans);
        }
        Section::Accounts if app.account_tab == 1 => {
            hints.extend([("↑↓", "select"), ("tab", "tab"), ("enter", "edit"), ("n", "new account")]);
        }
        Section::Accounts => hints.extend([("↑↓", "select"), ("tab", "tab"), ("n", "new account")]),
        Section::Clients if app.focus == crate::app::Focus::Detail => {
            hints.extend([("↑↓", "select"), (lang.t("space"), "toggle"), (lang.t("ctrl+s"), "save"), ("esc", "back")]);
            let mut spans = vec![Span::raw(" ")];
            for (key, label) in hints {
                spans.push(Span::styled(format!(" {key} "), Style::new().bg(theme::KEY).add_modifier(Modifier::BOLD)));
                spans.push(Span::styled(format!(" {}   ", lang.t(label)), theme::muted()));
            }
            return Line::from(spans);
        }
        Section::Clients => {
            hints.push(("↑↓", "select"));
            if let Some(client) = app.snapshot.clients.get(app.client_index) {
                if client.can_connect() {
                    hints.push(("c", if client.is_paired() { "reconnect" } else { "connect" }));
                }
                if client.is_paired() {
                    hints.extend([("enter", "accounts"), ("y", "copy"), ("v", "key"), ("T", "disconnect")]);
                }
            }
        }
        Section::Log => hints.extend([("↑↓", "row"), ("s", "sort"), ("r", "reverse")]),
        _ => {}
    }
    if app.section != Section::Log {
        hints.push(("r", "reload"));
    }
    hints.extend([("1-7", "menu"), ("q", "quit")]);

    let mut spans = vec![Span::raw(" ")];
    for (key, label) in hints {
        spans.push(Span::styled(format!(" {key} "), Style::new().bg(theme::KEY).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {}   ", lang.t(label)), theme::muted()));
    }
    Line::from(spans)
}

/// A rounded panel. The one that has the user's attention wears the accent.
pub(crate) fn panel(title: &str, focused: bool) -> Block<'static> {
    let border = if focused { theme::ACCENT } else { theme::LINE };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(border))
        .title(Line::styled(format!(" {title} "), theme::bold()))
}

/// A section heading drawn as a labelled rule.
pub(crate) fn rule(label: &str, width: u16) -> Line<'static> {
    let used = label.chars().count() + 1;
    Line::from(vec![
        Span::styled(format!("{label} "), Style::new().fg(theme::SILVER).add_modifier(Modifier::BOLD)),
        Span::styled("─".repeat(usize::from(width).saturating_sub(used)), Style::new().fg(theme::LINE)),
    ])
}

/// A label and its value on one line, the value starting at a fixed column.
pub(crate) fn field(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![Span::styled(format!("{label:<17} "), theme::muted()), Span::raw(value.into())])
}

/// The dot and the word for an account's state — never the colour alone.
pub(crate) fn health_mark(lang: Lang, health: &AccountHealth) -> (Span<'static>, &'static str) {
    match health {
        AccountHealth::Connected => (Span::styled("●", Style::new().fg(theme::GREEN)), lang.t("Connected")),
        AccountHealth::Failed(_) => (Span::styled("▲", Style::new().fg(theme::AMBER)), lang.t("Not reachable")),
        AccountHealth::NeedsTest => (Span::styled("○", theme::muted()), lang.t("Not tested yet")),
        AccountHealth::NotConfigured => (Span::styled("○", theme::faint()), lang.t("No credentials stored yet")),
    }
}

/// Local wall-clock time; dated once the entry is not from today, so a log
/// spanning days never shows two identical times a week apart.
pub(crate) fn clock(timestamp: f64, now: u64) -> String {
    use chrono::{Local, TimeZone};
    let Some(moment) = Local.timestamp_opt(timestamp as i64, 0).single() else {
        return String::new();
    };
    let today = Local.timestamp_opt(now as i64, 0).single().map(|today| today.date_naive());
    if Some(moment.date_naive()) == today {
        moment.format("%H:%M:%S").to_string()
    } else {
        moment.format("%d.%m. %H:%M").to_string()
    }
}
