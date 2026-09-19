//! MCP clients: every assistant TorroMail knows, where it stands, and the
//! snippet for the ones set up by hand. The key in the snippet is always the
//! masked stand-in here — this surface holds no keys yet.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use torromail_control::clients::{self, ClientKind};

use super::{field, panel, rule};
use crate::app::App;
use crate::data::ClientView;
use crate::i18n::Lang;
use crate::theme;

fn standing(lang: Lang, client: &ClientView) -> (&'static str, ratatui::style::Color, &'static str) {
    if client.descriptor.kind == ClientKind::Manual {
        ("+", theme::MUTED, lang.t("Set up by hand"))
    } else if !client.installed {
        ("○", theme::FAINT, lang.t("Not installed"))
    } else if client.configured && client.connection.is_some() {
        ("●", theme::GREEN, lang.t("Connected"))
    } else if client.configured {
        ("●", theme::AMBER, lang.t("Configured"))
    } else {
        ("○", theme::MUTED, lang.t("Not connected"))
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
    let mut lines = Vec::new();
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
            if client.installed { format!("{name} ✓") } else { lang.t("Not found on this machine").to_owned() },
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
        if let Some(path) = &client.config_path {
            lines.push(field(lang.t("Config file"), path.display().to_string()));
        }
        lines.push(Line::default());
    } else if let Some(path) = client.descriptor.manual_config_path {
        lines.push(field(lang.t("Config file"), path));
        lines.push(Line::default());
    }

    lines.push(rule(lang.t("Configuration"), body.width));
    let command = app
        .snapshot
        .server_binary
        .as_ref()
        .map_or_else(|| "torromail-mcp".to_owned(), |path| path.display().to_string());
    let snippet = clients::config_snippet(
        &command,
        client.descriptor.snippet_format,
        &clients::masked_token(client.descriptor.id),
    );
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
    lines.push(Line::styled(
        lang.t("Connecting from here follows once keys can be stored on this platform."),
        theme::faint(),
    ));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
}
