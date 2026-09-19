//! The overview: the same cards as the app's dashboard. The attention card is
//! absent whenever everything is fine — its appearing *is* the message.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use super::{clock, health_mark, panel};
use crate::app::App;
use crate::theme;

pub fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let lang = app.lang;
    let snapshot = &app.snapshot;
    // Nothing set up yet: say what to do, instead of four empty cards.
    if snapshot.accounts.is_empty() {
        getting_started(frame, area, app);
        return;
    }
    let broken: Vec<_> = snapshot.accounts.iter().filter(|view| view.health.is_broken()).collect();
    let attention_height = if broken.is_empty() { 0 } else { broken.len() as u16 * 2 + 2 };
    let accounts_height = (snapshot.accounts.len().max(2) as u16 + 2).min(8);
    let [status, attention, accounts, activity] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(attention_height),
        Constraint::Length(accounts_height),
        Constraint::Min(4),
    ])
    .areas(area);

    // Status: two independent facts, side by side.
    let block = panel(lang.t("Status"), false);
    let inner = block.inner(status);
    frame.render_widget(block, status);
    let [left, right] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(inner);
    let ready = snapshot.server_binary.is_some();
    frame.render_widget(
        Paragraph::new(vec![
            tile_title(ready, lang.t(if ready { "Ready for assistants" } else { "Not reachable for assistants" })),
            Line::styled(
                format!("   {}", lang.t(if ready {
                    "The MCP server is installed."
                } else {
                    "torromail-mcp was not found next to this program or on the PATH."
                })),
                theme::muted(),
            ),
        ])
        .wrap(ratatui::widgets::Wrap { trim: false }),
        left,
    );
    let connected = snapshot.connected_clients();
    let names: Vec<&str> = connected.iter().map(|client| client.descriptor.display_name).collect();
    frame.render_widget(
        Paragraph::new(vec![
            tile_title(
                !connected.is_empty(),
                &if connected.is_empty() {
                    lang.t("No assistant connected").to_owned()
                } else {
                    format!("{} {}", names.join(", "), lang.t("connected"))
                },
            ),
            Line::styled(
                format!("   {}", lang.t(if connected.is_empty() {
                    "Connect one in the MCP Clients section to start using your mail."
                } else {
                    "Every access is logged."
                })),
                theme::muted(),
            ),
        ])
        .wrap(ratatui::widgets::Wrap { trim: false }),
        right,
    );

    if !broken.is_empty() {
        let block = panel(lang.t("Needs your attention"), false)
            .border_style(Style::new().fg(theme::AMBER))
            .title_style(Style::new().fg(theme::AMBER));
        let inner = block.inner(attention);
        frame.render_widget(block, attention);
        let mut lines = Vec::new();
        for view in broken {
            let checked = view
                .last_checked
                .map(|at| format!("{} {}", lang.t("Last checked"), lang.ago(snapshot.taken_at.saturating_sub(at))))
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled(" ▲ ", Style::new().fg(theme::AMBER)),
                Span::styled(view.account.name.clone(), theme::bold()),
                Span::styled(format!("  {}   {checked}", view.account.email), theme::muted()),
            ]));
            if let crate::data::AccountHealth::Failed(reason) = &view.health {
                lines.push(Line::raw(format!("   {reason}")));
            }
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }

    let block = panel(lang.t("Mail Accounts"), false);
    let inner = block.inner(accounts);
    frame.render_widget(block, accounts);
    if snapshot.accounts.is_empty() {
        frame.render_widget(
            Paragraph::new(vec![
                Line::raw(format!(" {}", lang.t("No accounts yet."))),
                Line::styled(
                    format!(" {}", lang.t("Press n to add your first account.")),
                    theme::muted(),
                ),
            ]),
            inner,
        );
    } else {
        let rows = snapshot.accounts.iter().map(|view| {
            let (mark, word) = health_mark(lang, &view.health);
            Row::new(vec![
                Cell::from(format!(" {}", view.account.name)),
                Cell::from(Span::styled(view.account.email.clone(), theme::muted())),
                Cell::from(Line::from(vec![mark, Span::styled(format!(" {word}"), theme::muted())])),
            ])
        });
        frame.render_widget(
            Table::new(rows, [Constraint::Percentage(30), Constraint::Percentage(40), Constraint::Percentage(30)]),
            inner,
        );
    }

    let block = panel(lang.t("Recent Activity"), false);
    let inner = block.inner(activity);
    frame.render_widget(block, activity);
    if snapshot.audit.is_empty() {
        frame.render_widget(
            Paragraph::new(format!(
                " {}",
                lang.t("No activity yet. It appears here the moment an assistant does something.")
            ))
            .style(theme::muted())
            .wrap(ratatui::widgets::Wrap { trim: false }),
            inner,
        );
        return;
    }
    let header = Row::new(["Time", "Client", "Account", "Event", "Result"].map(|title| format!(" {}", lang.t(title))))
        .style(theme::faint());
    let rows = snapshot.audit.iter().take(usize::from(inner.height.saturating_sub(1))).map(|entry| {
        let failed = entry.result == "error";
        Row::new(vec![
            Cell::from(Span::styled(format!(" {}", clock(entry.timestamp, snapshot.taken_at)), theme::muted())),
            Cell::from(format!(" {}", entry.client)),
            Cell::from(format!(" {}", if entry.account.is_empty() { "—".to_owned() } else { snapshot.account_name(&entry.account) })),
            Cell::from(format!(" {}", lang.event(&entry.tool))),
            Cell::from(Span::styled(
                format!(" {}", lang.result(&entry.result)),
                Style::new().fg(if failed { theme::ACCENT } else { theme::GREEN }),
            )),
        ])
    });
    frame.render_widget(
        Table::new(
            rows,
            [Constraint::Length(14), Constraint::Length(16), Constraint::Length(14), Constraint::Min(20), Constraint::Length(12)],
        )
        .header(header),
        inner,
    );
}

fn tile_title(good: bool, text: &str) -> Line<'static> {
    let colour = if good { theme::GREEN } else { theme::AMBER };
    Line::from(vec![
        Span::styled(" ● ", Style::new().fg(colour)),
        Span::styled(text.to_owned(), Style::new().fg(colour).add_modifier(Modifier::BOLD)),
    ])
}

/// The first-run card: the three steps, with the key that starts each.
fn getting_started(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let lang = app.lang;
    let block = panel(lang.t("Getting started"), true);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let body = Rect { x: inner.x + 2, y: inner.y + 1, width: inner.width.saturating_sub(4), height: inner.height.saturating_sub(1) };

    let step = |number: &'static str, key: &'static str, title: &'static str, detail: &'static str| {
        vec![
            Line::from(vec![
                Span::styled(format!("{number}  "), Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)),
                Span::styled(lang.t(title), Style::new().add_modifier(Modifier::BOLD)),
                Span::raw("   "),
                Span::styled(format!(" {key} "), Style::new().bg(theme::KEY).add_modifier(Modifier::BOLD)),
            ]),
            Line::styled(format!("   {}", lang.t(detail)), theme::muted()),
            Line::default(),
        ]
    };
    let mut lines = vec![
        Line::styled(lang.t("Your mailboxes for AI assistants."), theme::muted()),
        Line::default(),
    ];
    lines.extend(step("1", "n", "Add a mail account", "TorroMail talks to your mail server. It never becomes your mail client."));
    lines.extend(step("2", "2", "Decide what is allowed", "Per account you pick what assistants may read and do."));
    lines.extend(step("3", "3", "Connect an assistant", "Connect one in the MCP Clients section to start using your mail."));
    if app.snapshot.server_binary.is_none() {
        lines.push(Line::styled(
            format!("▲ {}", lang.t("torromail-mcp was not found next to this program or on the PATH.")),
            Style::new().fg(theme::AMBER),
        ));
    }
    frame.render_widget(Paragraph::new(lines).wrap(ratatui::widgets::Wrap { trim: false }), body);
}
