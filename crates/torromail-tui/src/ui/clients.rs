//! MCP clients: every assistant TorroMail knows, where it stands, and the
//! snippet for the ones set up by hand. The key in the snippet is always the
//! masked stand-in here — this surface holds no keys yet.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use torromail_control::clients::{self, ClientKind};
use torromail_control::policy::ClientAccountAccess;

use super::{field, panel, rule};
use crate::app::{App, Focus};
use crate::data::ClientView;
use crate::i18n::Lang;
use crate::theme;

fn standing(lang: Lang, client: &ClientView) -> (&'static str, ratatui::style::Color, &'static str) {
    let manual = client.descriptor.kind == ClientKind::Manual;
    if !client.is_paired() {
        return if manual {
            ("+", theme::MUTED, lang.t("Set up by hand"))
        } else if client.installed {
            ("○", theme::MUTED, lang.t("Not connected"))
        } else {
            ("○", theme::FAINT, lang.t("Not installed"))
        };
    }
    if !manual && !client.has_current_key {
        return ("▲", theme::AMBER, lang.t("Access key missing"));
    }
    if client.connection.is_some() {
        ("●", theme::GREEN, lang.t("Connected"))
    } else {
        ("●", theme::AMBER, lang.t("Configured"))
    }
}

pub fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let lang = app.lang;
    let [list, detail] = Layout::horizontal([Constraint::Length(30), Constraint::Min(0)]).areas(area);

    let block = panel(lang.t("Assistants"), false);
    let inner = block.inner(list);
    frame.render_widget(block, list);
    let mut lines = vec![Line::default()];
    for (index, client) in app.snapshot.clients.iter().enumerate() {
        let selected = index == app.client_index;
        let (mark, colour, word) = standing(lang, client);
        let style = if selected { theme::selected() } else { Style::new() };
        let bar = Span::styled(if selected { "▌" } else { " " }, Style::new().fg(theme::ACCENT));
        let width = usize::from(inner.width).saturating_sub(4);
        lines.push(
            Line::from(vec![
                bar.clone(),
                Span::styled(format!(" {mark} "), Style::new().fg(colour)),
                Span::raw(lang.t(client.descriptor.display_name)),
            ])
            .style(style),
        );
        lines.push(Line::from(vec![bar, Span::styled(format!("   {word:<width$}"), theme::muted())]).style(style));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    let Some(client) = app.snapshot.clients.get(app.client_index) else { return };
    let block = panel(lang.t(client.descriptor.display_name), true);
    let inner = block.inner(detail);
    frame.render_widget(block, detail);
    let body = Rect { x: inner.x + 1, y: inner.y + 1, width: inner.width.saturating_sub(2), height: inner.height.saturating_sub(1) };

    let name = client.descriptor.display_name;
    let tick = |good: bool, text: String| {
        Line::from(vec![
            Span::styled(if good { "✓ " } else { "· " }, Style::new().fg(if good { theme::GREEN } else { theme::FAINT })),
            Span::styled(text, if good { Style::new() } else { theme::muted() }),
        ])
    };
    let editing = app.focus == Focus::Detail;
    let mut lines = Vec::new();
    if app.confirm_discard {
        lines.push(Line::styled(lang.t("Discard unsaved changes? (y/n)"), Style::new().fg(theme::AMBER).add_modifier(Modifier::BOLD)));
        lines.push(Line::default());
    } else if app.confirm_disconnect {
        lines.push(Line::styled(
            format!("{} {name}? {}", lang.t("Disconnect"), lang.t("(y/n)")),
            Style::new().fg(theme::AMBER).add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::default());
    } else if let Some(message) = &app.message {
        let colour = if message.is_error { theme::ACCENT } else { theme::GREEN };
        lines.push(Line::styled(lang.t(message.text), Style::new().fg(colour).add_modifier(Modifier::BOLD)));
        if !message.detail.is_empty() {
            lines.push(Line::styled(message.detail.clone(), theme::muted()));
        }
        lines.push(Line::default());
    } else if editing && app.is_dirty() {
        lines.push(Line::styled(format!("● {}", lang.t("Unsaved changes")), Style::new().fg(theme::AMBER)));
        lines.push(Line::default());
    }

    match &client.connection {
        Some(connection) => {
            lines.push(Line::styled(
                format!("● {name} {}", lang.t("has connected to the server")),
                Style::new().fg(theme::GREEN).add_modifier(Modifier::BOLD),
            ));
            let version = if connection.reported_version.is_empty() {
                String::new()
            } else {
                format!(" · {} {}", lang.t("Version"), connection.reported_version)
            };
            lines.push(Line::styled(
                format!(
                    "  {} {}{version}",
                    lang.t("Last connected"),
                    lang.ago(app.snapshot.taken_at.saturating_sub(connection.last_connected as u64))
                ),
                theme::muted(),
            ));
        }
        None => lines.push(Line::styled(format!("○ {}", lang.t("Has not connected yet")), theme::muted())),
    }
    lines.push(Line::default());

    if client.descriptor.kind == ClientKind::Automatic {
        lines.push(rule(lang.t("Connection status"), body.width));
        lines.push(tick(
            client.installed,
            lang.t(if client.installed { "Installed on this machine" } else { "Not found on this machine" }).to_owned(),
        ));
        lines.push(tick(
            client.configured,
            lang.t(if client.configured {
                "Written into the client's configuration"
            } else {
                "Not written into the client's configuration yet"
            })
            .to_owned(),
        ));
        if client.is_paired() || client.configured {
            lines.push(match app.snapshot.server_tools {
                Some(count) => tick(true, format!("{} — {count} {}", lang.t("TorroMail server answers"), lang.t("tools"))),
                None => tick(false, lang.t("TorroMail server did not answer").to_owned()),
            });
            lines.push(tick(
                client.has_current_key,
                lang.t(if client.has_current_key {
                    "The client holds its access key"
                } else {
                    "Access key missing — reconnect to fix it"
                })
                .to_owned(),
            ));
        }
        if let Some(path) = &client.config_path {
            lines.push(field(lang.t("Config file"), app.snapshot.tilde(path)));
        }
        lines.push(Line::default());
    } else if let Some(path) = client.descriptor.manual_config_path {
        lines.push(field(lang.t("Config file"), path));
        lines.push(Line::default());
    }

    if let Some(stored) = &client.access {
        let access = app.access_draft.as_ref().filter(|_| editing).unwrap_or(stored);
        let cursor = editing.then_some(app.cursor);
        let row = |index: usize, line: Line<'static>| if cursor == Some(index) { line.style(theme::selected()) } else { line };
        let radio = |on: bool, label: &'static str| {
            Line::from(vec![
                Span::styled(if on { "(•) " } else { "( ) " }, Style::new().fg(if on { theme::ACCENT } else { theme::FAINT })),
                Span::styled(lang.t(label), if on { theme::bold() } else { Style::new() }),
            ])
        };
        let all = *access == ClientAccountAccess::All;
        lines.push(rule(lang.t("Account access"), body.width));
        lines.push(row(0, radio(all, "All accounts")));
        lines.push(row(1, radio(!all, "Selected accounts")));
        for (index, view) in app.snapshot.accounts.iter().enumerate() {
            let ticked = match access {
                ClientAccountAccess::All => true,
                ClientAccountAccess::Selected(ids) => ids.contains(&view.account.id),
            };
            lines.push(row(
                index + 2,
                Line::from(vec![
                    Span::styled("  [", theme::faint()),
                    Span::styled(if ticked { "✓" } else { " " }, Style::new().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
                    Span::styled("] ", theme::faint()),
                    Span::styled(view.account.name.clone(), if all { theme::muted() } else { Style::new() }),
                    Span::styled(format!("  {}", view.account.email), theme::faint()),
                ]),
            ));
        }
        lines.push(Line::styled(
            lang.t(if all {
                "Includes accounts added later. Account permissions still apply."
            } else {
                "New accounts need a separate selection. Account permissions still apply."
            }),
            theme::faint(),
        ));
        lines.push(Line::default());
    }

    lines.push(rule(lang.t("Configuration"), body.width));
    let command = app
        .snapshot
        .server_binary
        .as_ref()
        .map_or_else(|| "torromail-mcp".to_owned(), |path| path.display().to_string());
    // Masked unless this very client's key was asked for; what `y` copies is
    // the same text with the one word swapped.
    let shown_key = match &app.revealed {
        Some((id, token)) if id == client.descriptor.id => token.clone(),
        _ => clients::masked_token(client.descriptor.id),
    };
    let snippet = clients::config_snippet(&command, client.descriptor.snippet_format, &shown_key);
    // Shown with half the indentation: the panel is narrow, and what gets
    // copied is the real text, not this rendering of it.
    lines.extend(snippet.lines().map(|line| {
        let indent = line.len() - line.trim_start().len();
        Line::styled(format!(" {}{}", " ".repeat(indent / 2), line.trim_start()), Style::new().fg(theme::CYAN))
    }));
    lines.push(Line::default());
    lines.push(Line::styled(
        lang.t("The snippet carries this client's personal access key — treat it like a password."),
        theme::faint(),
    ));
    if let Some(problem) = &app.snapshot.pairings_problem {
        lines.push(Line::styled(problem.clone(), Style::new().fg(theme::ACCENT)));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((app.detail_scroll, 0)), body);
}
