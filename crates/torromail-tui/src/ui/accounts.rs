//! Mail accounts: the list, and one account's sections as tabs. Read-only for
//! now — every value shown is the stored one, nothing here writes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use torromail_control::{
    CacheLevel, ConnectionSecurity, LoginMethod, MailAccount, PermissionPreset, PermissionSet, ReadAccess, mailbox_names,
};

use super::{field, health_mark, panel, rule};
use crate::app::{ACCOUNT_TABS, App, Focus};
use crate::data::{AccountHealth, AccountView};
use crate::i18n::Lang;
use crate::theme;

pub fn draw(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let lang = app.lang;
    let [list, detail] = Layout::horizontal([Constraint::Length(30), Constraint::Min(0)]).areas(area);

    let block = panel(lang.t("Mail Accounts"), false);
    let inner = block.inner(list);
    frame.render_widget(block, list);
    let mut lines = vec![Line::default()];
    for (index, view) in app.snapshot.accounts.iter().enumerate() {
        let selected = index == app.account_index;
        let (mark, _) = health_mark(lang, &view.health);
        let style = if selected { theme::selected() } else { Style::new() };
        let width = usize::from(inner.width);
        let bar = Span::styled(if selected { "▌" } else { " " }, Style::new().fg(theme::ACCENT));
        lines.push(Line::from(vec![bar.clone(), Span::raw(" "), mark, Span::raw(format!(" {}", view.account.name))]).style(style));
        lines.push(Line::from(vec![bar, Span::styled(format!("   {:<w$}", view.account.email, w = width.saturating_sub(4)), theme::muted())]).style(style));
        lines.push(Line::default());
    }
    if app.snapshot.accounts.is_empty() {
        lines.push(Line::styled(format!(" {}", lang.t("No accounts yet.")), theme::muted()));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    let Some(view) = app.snapshot.accounts.get(app.account_index) else {
        let block = panel(lang.t("Mail Accounts"), true);
        let inner = block.inner(detail);
        frame.render_widget(block, detail);
        frame.render_widget(
            Paragraph::new(format!(
                " {}",
                lang.t("Press n to add your first account.")
            ))
            .style(theme::muted())
            .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    };

    let (mark, word) = health_mark(lang, &view.health);
    let block = panel(&format!("{} · {}", view.account.name, view.account.email), true)
        .title(Line::from(vec![Span::raw(" "), mark, Span::raw(format!(" {word} "))]).right_aligned());
    let inner = block.inner(detail);
    frame.render_widget(block, detail);
    let [tabs, _, body] =
        Layout::vertical([Constraint::Length(2), Constraint::Length(1), Constraint::Min(0)]).areas(inner);

    let mut names = vec![Span::raw(" ")];
    let mut underline = vec![Span::raw(" ")];
    for (index, tab) in ACCOUNT_TABS.iter().enumerate() {
        let label = lang.t(tab);
        let on = index == app.account_tab;
        names.push(Span::styled(label, if on { theme::bold() } else { theme::muted() }));
        names.push(Span::raw("   "));
        let mark = if on { "━" } else { "─" };
        underline.push(Span::styled(
            mark.repeat(label.chars().count()),
            Style::new().fg(if on { theme::ACCENT } else { theme::LINE }),
        ));
        underline.push(Span::styled("───", Style::new().fg(theme::LINE)));
    }
    frame.render_widget(Paragraph::new(vec![Line::from(names), Line::from(underline)]), tabs);

    let body = Rect { x: body.x + 1, width: body.width.saturating_sub(2), ..body };
    let editing = (app.focus == Focus::Detail).then_some(app);
    let shown = app.draft.as_ref().filter(|_| editing.is_some()).unwrap_or(&view.account);
    let mut lines = match app.account_tab {
        0 => connection(lang, view, shown, body.width, app.snapshot.taken_at, editing),
        1 => permissions(lang, &shown.permissions, body.width, editing),
        2 => folders(lang, shown, body.width, editing),
        _ => cache(lang, shown, body.width, editing),
    };
    if let Some(app) = editing {
        lines.push(Line::default());
        lines.extend(edit_status(lang, app));
    }
    // Outside of editing, the account's last message sits on top of any tab.
    if app.focus != Focus::Detail {
        if app.confirm_remove {
            lines.splice(0..0, [
                Line::styled(
                    format!("{} „{}“? {}", lang.t("Remove"), view.account.name, lang.t("(y/n)")),
                    Style::new().fg(theme::AMBER).add_modifier(Modifier::BOLD),
                ),
                Line::styled(
                    lang.t("This removes the configuration, the stored password and the local cache. Your mail stays on the server."),
                    theme::muted(),
                ),
                Line::default(),
            ]);
        } else if let Some(message) = &app.message {
            let colour = if message.is_error { theme::ACCENT } else { theme::GREEN };
            let mut shown = vec![Line::styled(lang.t(message.text), Style::new().fg(colour).add_modifier(Modifier::BOLD))];
            if !message.detail.is_empty() {
                shown.push(Line::styled(message.detail.clone(), theme::muted()));
            }
            shown.push(Line::default());
            lines.splice(0..0, shown);
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((app.detail_scroll, 0)), body);
}

fn connection(
    lang: Lang,
    view: &AccountView,
    account: &MailAccount,
    width: u16,
    now: u64,
    editing: Option<&App>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let AccountHealth::Failed(reason) = &view.health {
        lines.push(Line::styled(
            format!("▲ {}", lang.t("TorroMail can no longer reach this account.")),
            Style::new().fg(theme::AMBER).add_modifier(Modifier::BOLD),
        ));
        lines.push(Line::raw(format!("  {reason}")));
    }
    if let Some(at) = view.last_checked {
        lines.push(Line::styled(
            format!("  {} {}", lang.t("Last checked"), lang.ago(now.saturating_sub(at))),
            theme::muted(),
        ));
    }
    if !lines.is_empty() {
        lines.push(Line::default());
    }
    let security = |security: ConnectionSecurity| match security {
        ConnectionSecurity::Tls => "SSL/TLS",
        ConnectionSecurity::StartTls => "STARTTLS",
    };

    let Some(app) = editing else {
        lines.extend([
            rule(lang.t("Identity"), width),
            field(lang.t("Sender Name"), account.name.clone()),
            field("E-Mail", account.email.clone()),
            Line::default(),
            rule(lang.t("Connection"), width),
            field(lang.t("Provider"), account.provider.clone()),
            field(
                lang.t("Login"),
                match account.login_method {
                    LoginMethod::Password => lang.t("Password").to_owned(),
                    LoginMethod::OAuth => format!("OAuth · {}", account.oauth_issuer.map_or("", |issuer| issuer.as_str())),
                },
            ),
            field(lang.t("Username"), account.username.clone()),
            field(lang.t("IMAP Server"), format!("{}:{} · {}", account.imap_host, account.imap_port, security(account.imap_security))),
            field(lang.t("SMTP Server"), format!("{}:{} · {}", account.smtp_host, account.smtp_port, security(account.smtp_security))),
        ]);
        return lines;
    };

    // Editing: one row per fact, the row order `App::connection_text` uses.
    let masked = "•".repeat(app.extras.password.0.chars().count());
    let rows: [(&str, String, bool); 9] = [
        ("Sender Name", account.name.clone(), true),
        ("Username", account.username.clone(), true),
        ("IMAP Server", account.imap_host.clone(), true),
        ("IMAP Port", app.extras.imap_port.clone(), true),
        ("IMAP encryption", format!("‹ {} ›", security(account.imap_security)), false),
        ("SMTP Server", account.smtp_host.clone(), true),
        ("SMTP Port", app.extras.smtp_port.clone(), true),
        ("SMTP encryption", format!("‹ {} ›", security(account.smtp_security)), false),
        ("New password", masked, true),
    ];
    lines.push(rule(lang.t("Connection"), width));
    for (index, (label, value, is_text)) in rows.into_iter().enumerate() {
        let on = index == app.cursor;
        let mut spans = vec![Span::styled(format!("{:<17} ", lang.t(label)), theme::muted()), Span::raw(value)];
        if on && is_text {
            spans.push(Span::styled("█", Style::new().fg(theme::SILVER)));
        }
        let line = Line::from(spans);
        lines.push(if on { line.style(theme::selected()) } else { line });
    }
    lines.push(Line::default());
    lines.push(Line::styled(lang.t("Leave the password empty to keep the stored one. Saving checks the connection."), theme::faint()));
    lines
}

fn read_label(lang: Lang, read: ReadAccess) -> &'static str {
    lang.t(match read {
        ReadAccess::None => "No read access",
        ReadAccess::Headers => "Subject and sender",
        ReadAccess::FullMessage => "Full message",
        ReadAccess::WithAttachments => "Message and attachments",
    })
}

/// The permissions tab. While editing it shows the draft and a cursor; the
/// row order here is the one `App::toggle_row` acts on.
fn permissions(lang: Lang, permissions: &PermissionSet, width: u16, editing: Option<&App>) -> Vec<Line<'static>> {
    let current = permissions.matching_preset();
    let cursor = editing.map(|app| app.cursor);
    let mut row = 0;
    let mut editable = |line: Line<'static>| {
        let line = if cursor == Some(row) { line.style(theme::selected()) } else { line };
        row += 1;
        line
    };
    let preset_title = |preset: PermissionPreset| match preset {
        PermissionPreset::ReadOnly => "Read only",
        PermissionPreset::ReadAndDrafts => "Read + drafts",
        PermissionPreset::TidyUp => "Tidy up",
        PermissionPreset::FullAccess => "Full access",
    };
    let check = |on: bool, label: &'static str, note: &'static str| {
        Line::from(vec![
            Span::styled("[", theme::faint()),
            Span::styled(if on { "✓" } else { " " }, Style::new().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
            Span::styled("] ", theme::faint()),
            Span::styled(format!("{:<18}", lang.t(label)), if on { Style::new() } else { theme::muted() }),
            Span::styled(if note.is_empty() { "" } else { lang.t(note) }, theme::faint()),
        ])
    };

    let mut lines = vec![
        Line::styled(lang.t("What connected assistants may do with this account."), theme::muted()),
        Line::default(),
    ];
    let heading = match current {
        Some(_) => rule(lang.t("Preset"), width),
        None => rule(&format!("{} · {}", lang.t("Preset"), lang.t("adjusted")), width),
    };
    lines.push(heading);
    for preset in PermissionPreset::ALL {
        let on = current == Some(preset);
        lines.push(editable(Line::from(vec![
            Span::styled(if on { "(•) " } else { "( ) " }, Style::new().fg(if on { theme::ACCENT } else { theme::FAINT })),
            Span::styled(lang.t(preset_title(preset)), if on { theme::bold() } else { Style::new() }),
        ])));
    }
    lines.extend([Line::default(), rule(lang.t("Read"), width)]);
    lines.push(editable(Line::from(vec![
        Span::styled(format!("{:<17} ", lang.t("Read depth")), theme::muted()),
        Span::styled("‹ ", theme::faint()),
        Span::styled(read_label(lang, permissions.read), theme::bold()),
        Span::styled(" ›", theme::faint()),
    ])));
    lines.extend([Line::default(), rule(lang.t("Write"), width)]);
    for line in [
        check(permissions.write.drafts, "Create drafts", ""),
        check(permissions.send, "Send", ""),
        check(permissions.write.mark, "Mark", "read state, flags"),
        check(permissions.write.r#move, "Move", ""),
        check(permissions.write.trash, "Delete", "to the Trash"),
        check(permissions.write.permanent_delete, "Permanent delete", "manual only"),
    ] {
        lines.push(editable(line));
    }
    lines.push(Line::default());

    if editing.is_none() {
        lines.push(Line::styled(lang.t("Applies to the whole account."), theme::faint()));
    }
    lines
}

/// What an edit in progress has to say for itself: the question before a
/// discard, the last message, or that there is something unsaved.
fn edit_status(lang: Lang, app: &App) -> Vec<Line<'static>> {
    if app.confirm_discard {
        return vec![Line::styled(lang.t("Discard unsaved changes? (y/n)"), Style::new().fg(theme::AMBER).add_modifier(Modifier::BOLD))];
    }
    if let Some(message) = &app.message {
        let colour = if message.is_error { theme::ACCENT } else { theme::GREEN };
        let mut lines = vec![Line::styled(lang.t(message.text), Style::new().fg(colour).add_modifier(Modifier::BOLD))];
        if !message.detail.is_empty() {
            lines.push(Line::styled(message.detail.clone(), theme::muted()));
        }
        return lines;
    }
    if app.is_dirty() {
        return vec![Line::styled(format!("● {}", lang.t("Unsaved changes")), Style::new().fg(theme::AMBER))];
    }
    Vec::new()
}

fn folders(lang: Lang, account: &MailAccount, width: u16, editing: Option<&App>) -> Vec<Line<'static>> {
    let permissions = &account.permissions;
    let cursor = editing.map(|app| app.cursor);
    let mut row = 0;
    let mut editable = |line: Line<'static>| {
        let line = if cursor == Some(row) { line.style(theme::selected()) } else { line };
        row += 1;
        line
    };
    let rule_words = |folder_rule: Option<&torromail_control::FolderRule>| match folder_rule.map(|rule| (rule.read, rule.write)) {
        None | Some((true, true)) => (lang.t("Standard").to_owned(), theme::MUTED),
        Some((false, false)) => (format!("{} · {}", lang.t("adjusted"), lang.t("no access")), theme::ACCENT),
        Some((true, false)) => (format!("{} · {}", lang.t("adjusted"), lang.t("read only")), theme::AMBER),
        Some((false, true)) => (format!("{} · {}", lang.t("adjusted"), lang.t("write only")), theme::AMBER),
    };

    let mut lines = vec![rule(lang.t("Per-folder permissions"), width)];
    lines.push(editable(Line::from(vec![
        Span::styled("[", theme::faint()),
        Span::styled(if permissions.per_folder { "✓" } else { " " }, Style::new().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
        Span::styled("] ", theme::faint()),
        Span::raw(lang.t("Per-folder permissions")),
    ])));
    // While editing, every folder the server has; otherwise only the ones
    // that differ, because "standard" is not news.
    let listed: Vec<String> = match editing {
        Some(app) => app.extras.folders.clone(),
        None => permissions.folder_rules.keys().cloned().collect(),
    };
    for mailbox in &listed {
        let (words, colour) = rule_words(permissions.folder_rules.get(mailbox));
        let line = Line::from(vec![
            Span::raw(format!("  {:<22}", mailbox_names::display_name(mailbox))),
            Span::styled(words, Style::new().fg(colour)),
        ]);
        lines.push(if editing.is_some() { editable(line) } else { line });
    }
    if editing.is_none() {
        lines.push(Line::styled(lang.t("Off: all folders use the permissions above."), theme::faint()));
    }

    lines.extend([Line::default(), rule(lang.t("Special folders"), width)]);
    let special = &account.special_mailboxes;
    for (label, chosen) in [
        ("Drafts", &special.drafts),
        ("Sent", &special.sent),
        ("Archive", &special.archive),
        ("Junk", &special.junk),
        ("Trash", &special.trash),
    ] {
        let value = if special.manual && !chosen.is_empty() {
            mailbox_names::display_name(chosen)
        } else {
            lang.t("Automatic").to_owned()
        };
        let line = if editing.is_some() {
            Line::from(vec![
                Span::styled(format!("{:<17} ", lang.t(label)), theme::muted()),
                Span::styled("‹ ", theme::faint()),
                Span::raw(value),
                Span::styled(" ›", theme::faint()),
            ])
        } else {
            field(lang.t(label), value)
        };
        lines.push(if editing.is_some() { editable(line) } else { line });
    }
    lines
}

fn cache(lang: Lang, account: &MailAccount, width: u16, editing: Option<&App>) -> Vec<Line<'static>> {
    let label = |level: CacheLevel| {
        lang.t(match level {
            CacheLevel::Off => "Off",
            CacheLevel::Headers => "Subject & sender",
            CacheLevel::Bodies => "Full messages",
            CacheLevel::Attachments => "Messages & attachments",
        })
    };
    let ceiling = CacheLevel::ceiling(account.permissions.read);
    let level = Line::from(vec![
        Span::styled(format!("{:<17} ", lang.t("Cache")), theme::muted()),
        Span::styled("‹ ", theme::faint()),
        Span::styled(label(account.cache_level), theme::bold()),
        Span::styled(" ›", theme::faint()),
    ]);
    let mut lines = vec![
        rule(lang.t("Search & Cache"), width),
        if editing.is_some() { level.style(theme::selected()) } else { level },
    ];
    if account.cache_level > ceiling {
        lines.push(Line::styled(
            format!("{} „{}“", lang.t("Limited by the read permission to"), label(ceiling)),
            Style::new().fg(theme::AMBER),
        ));
    }
    lines.extend([
        Line::default(),
        Line::styled(
            lang.t("More caching makes search faster but stores mail content on this machine."),
            theme::faint(),
        ),
    ]);
    lines
}
