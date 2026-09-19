//! Mail accounts: the list, and one account's sections as tabs. Read-only for
//! now — every value shown is the stored one, nothing here writes.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use torromail_control::{
    CacheLevel, ConnectionSecurity, LoginMethod, PermissionPreset, PermissionSet, ReadAccess, mailbox_names,
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
                lang.t("Accounts are added in the macOS app for now; this surface shows them.")
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
    let lines = match app.account_tab {
        0 => connection(lang, view, body.width, app.snapshot.taken_at),
        1 => {
            let editing = (app.focus == Focus::Detail).then_some(app);
            let shown = app.draft.as_ref().filter(|_| editing.is_some()).map_or(&view.account.permissions, |draft| &draft.permissions);
            permissions(lang, shown, body.width, editing)
        }
        2 => folders(lang, view, body.width),
        _ => cache(lang, view, body.width),
    };
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
}

fn connection(lang: Lang, view: &AccountView, width: u16, now: u64) -> Vec<Line<'static>> {
    let account = &view.account;
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

    let Some(app) = editing else {
        lines.push(Line::styled(lang.t("Applies to the whole account."), theme::faint()));
        return lines;
    };
    if app.confirm_discard {
        lines.push(Line::styled(lang.t("Discard unsaved changes? (y/n)"), Style::new().fg(theme::AMBER).add_modifier(Modifier::BOLD)));
    } else if let Some(message) = &app.message {
        let colour = if message.is_error { theme::ACCENT } else { theme::GREEN };
        lines.push(Line::styled(lang.t(message.text), Style::new().fg(colour).add_modifier(Modifier::BOLD)));
        if !message.detail.is_empty() {
            lines.push(Line::styled(message.detail.clone(), theme::muted()));
        }
    } else if app.is_dirty() {
        lines.push(Line::styled(format!("● {}", lang.t("Unsaved changes")), Style::new().fg(theme::AMBER)));
    }
    lines
}

fn folders(lang: Lang, view: &AccountView, width: u16) -> Vec<Line<'static>> {
    let account = &view.account;
    let permissions = &account.permissions;
    let mut lines = vec![
        rule(lang.t("Per-folder permissions"), width),
        field(lang.t("Per-folder permissions"), lang.t(if permissions.per_folder { "on" } else { "off" })),
        field(lang.t("Standard"), lang.t("Standard – all folders")),
    ];
    for (mailbox, folder_rule) in &permissions.folder_rules {
        let (word, colour) = match (folder_rule.read, folder_rule.write) {
            (true, true) => continue,
            (false, false) => (lang.t("no access"), theme::ACCENT),
            (true, false) => (lang.t("read only"), theme::AMBER),
            (false, true) => (lang.t("write only"), theme::AMBER),
        };
        lines.push(Line::from(vec![
            Span::raw(format!("  {:<16}", mailbox_names::display_name(mailbox))),
            Span::styled(format!("{} · {word}", lang.t("adjusted")), Style::new().fg(colour)),
        ]));
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
        lines.push(field(lang.t(label), value));
    }
    lines
}

fn cache(lang: Lang, view: &AccountView, width: u16) -> Vec<Line<'static>> {
    let account = &view.account;
    let label = |level: CacheLevel| {
        lang.t(match level {
            CacheLevel::Off => "Off",
            CacheLevel::Headers => "Subject & sender",
            CacheLevel::Bodies => "Full messages",
            CacheLevel::Attachments => "Messages & attachments",
        })
    };
    let ceiling = CacheLevel::ceiling(account.permissions.read);
    let mut lines = vec![rule(lang.t("Search & Cache"), width), field(lang.t("Cache"), label(account.cache_level))];
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
