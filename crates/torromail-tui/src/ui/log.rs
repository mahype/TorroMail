//! The log: the app's table with the same six columns, sortable, newest first
//! by default. Ordering keys off the real timestamp, never the rendered time.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use super::{clock, panel};
use crate::app::{App, LogColumn};
use crate::theme;

pub fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let lang = app.lang;
    let snapshot = &app.snapshot;
    let block = panel(lang.t("Log"), true)
        .title(Line::styled(format!(" {} ", snapshot.audit.len()), theme::muted()).right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if snapshot.audit.is_empty() {
        frame.render_widget(
            Paragraph::new(format!(
                " {}",
                lang.t("No activity yet. It appears here the moment an assistant does something.")
            ))
            .style(theme::muted()),
            inner,
        );
        return;
    }

    let mut entries: Vec<_> = snapshot
        .audit
        .iter()
        .map(|entry| {
            let account = if entry.account.is_empty() { String::new() } else { snapshot.account_name(&entry.account) };
            (entry, account, lang.event(&entry.tool))
        })
        .collect();
    entries.sort_by(|a, b| {
        let order = match app.log_sort {
            LogColumn::Time => a.0.timestamp.total_cmp(&b.0.timestamp),
            LogColumn::Client => a.0.client.cmp(&b.0.client),
            LogColumn::Account => a.1.cmp(&b.1),
            LogColumn::Event => a.2.cmp(&b.2),
            LogColumn::Details => a.0.detail.cmp(&b.0.detail),
            LogColumn::Result => a.0.result.cmp(&b.0.result),
        }
        // Equal keys fall back to time, so a sort by client still reads as a
        // timeline within each client.
        .then(a.0.timestamp.total_cmp(&b.0.timestamp));
        if app.log_descending { order.reverse() } else { order }
    });

    let header = Row::new(LogColumn::ALL.map(|column| {
        let title = lang.t(column.title());
        if column == app.log_sort {
            Cell::from(Line::from(vec![
                Span::styled(format!(" {title} "), theme::bold()),
                Span::styled(if app.log_descending { "▼" } else { "▲" }, Style::new().fg(theme::ACCENT)),
            ]))
        } else {
            Cell::from(Span::styled(format!(" {title}"), theme::faint()))
        }
    }))
    .bottom_margin(1);
    let rows = entries.iter().map(|(entry, account, event)| {
        let failed = entry.result == "error";
        Row::new(vec![
            Cell::from(Span::styled(format!(" {}", clock(entry.timestamp, snapshot.taken_at)), theme::muted())),
            Cell::from(format!(" {}", entry.client)),
            Cell::from(format!(" {}", if account.is_empty() { "—" } else { account })),
            Cell::from(format!(" {event}")),
            Cell::from(Span::styled(format!(" {}", entry.detail), theme::muted())),
            Cell::from(Span::styled(
                format!(" {}", lang.result(&entry.result)),
                Style::new().fg(if failed { theme::ACCENT } else { theme::GREEN }),
            )),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(13),
            Constraint::Length(13),
            Constraint::Length(11),
            Constraint::Length(25),
            Constraint::Fill(1),
            Constraint::Length(10),
        ],
    )
    .column_spacing(0)
    .header(header)
    .row_highlight_style(theme::selected());
    let mut state = TableState::default().with_selected(Some(app.log_index));
    frame.render_stateful_widget(table, inner, &mut state);
    // The outcome of an export sits over the table's last line.
    if let Some(message) = &app.message {
        let line = Rect { y: inner.y + inner.height.saturating_sub(1), height: 1, ..inner };
        let colour = if message.is_error { theme::ACCENT } else { theme::GREEN };
        frame.render_widget(
            Paragraph::new(format!(" {} {}", lang.t(message.text), message.detail)).style(Style::new().fg(colour)),
            line,
        );
    }

}
