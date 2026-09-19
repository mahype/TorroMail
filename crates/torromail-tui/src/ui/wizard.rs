//! The add-account wizard, drawn over whatever is behind it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use torromail_control::PermissionPreset;

use super::panel;
use crate::i18n::Lang;
use crate::theme;
use crate::wizard::{Hint, Step, Wizard};

pub fn draw(frame: &mut Frame<'_>, area: Rect, lang: Lang, wizard: &Wizard) {
    let width = area.width.min(68);
    let height = area.height.min(26);
    let modal = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, modal);
    let block = panel(lang.t("New Account"), true);
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    let body = Rect { x: inner.x + 2, y: inner.y + 1, width: inner.width.saturating_sub(4), height: inner.height.saturating_sub(1) };

    // Where we are: done steps ticked, the current one bold.
    let position = Step::ALL.iter().position(|step| *step == wizard.step).unwrap_or(0);
    let mut stepper = Vec::new();
    for (index, step) in Step::ALL.iter().enumerate() {
        let (mark, style) = match index.cmp(&position) {
            std::cmp::Ordering::Less => ("✓ ", Style::new().fg(theme::GREEN)),
            std::cmp::Ordering::Equal => ("● ", theme::bold()),
            std::cmp::Ordering::Greater => ("○ ", theme::faint()),
        };
        stepper.push(Span::styled(format!("{mark}{}", lang.t(step.title())), style));
        if index + 1 < Step::ALL.len() {
            stepper.push(Span::styled(" ── ", Style::new().fg(theme::LINE)));
        }
    }
    let mut lines = vec![Line::from(stepper), Line::default()];

    let input = |label: &'static str, value: &str, focused: bool, masked: bool| {
        let shown = if masked { "•".repeat(value.chars().count()) } else { value.to_owned() };
        let cursor = if focused { "█" } else { "" };
        vec![
            Line::styled(lang.t(label), if focused { theme::bold() } else { theme::muted() }),
            Line::from(vec![
                Span::styled(if focused { "▌ " } else { "│ " }, Style::new().fg(if focused { theme::ACCENT } else { theme::LINE })),
                Span::raw(shown),
                Span::styled(cursor, Style::new().fg(theme::SILVER)),
            ]),
            Line::default(),
        ]
    };

    match wizard.step {
        Step::Identity => {
            lines.push(Line::styled(lang.t("The name and address anything you approve is sent as."), theme::muted()));
            lines.push(Line::default());
            lines.extend(input("Email", &wizard.email, wizard.field == 0, false));
            lines.extend(input("Sender Name", &wizard.name, wizard.field == 1, false));
            if wizard.looking_up {
                lines.push(Line::styled(
                    format!("⠹ {} {} …", lang.t("Looking up the settings for"), wizard.email),
                    Style::new().fg(theme::CYAN).add_modifier(Modifier::BOLD),
                ));
            }
        }
        Step::SignIn => {
            lines.push(Line::styled(format!("{} {}", lang.t("Login for"), wizard.email), theme::bold()));
            lines.push(Line::default());
            if wizard.hint == Hint::Unknown {
                lines.push(Line::styled(
                    lang.t("Nothing could be found for this domain — the server details have to come from you."),
                    Style::new().fg(theme::AMBER),
                ));
            } else {
                lines.push(Line::from(vec![
                    Span::styled("✓ ", Style::new().fg(theme::GREEN)),
                    Span::styled(format!("{} · {}", lang.t("Settings found"), wizard.provider_label), Style::new().fg(theme::GREEN)),
                ]));
            }
            if wizard.manual {
                lines.push(Line::default());
                lines.extend(input("IMAP Server", &wizard.imap_host, wizard.field == 0, false));
                lines.extend(input("IMAP Port", &wizard.imap_port, wizard.field == 1, false));
                lines.extend(input("SMTP Server", &wizard.smtp_host, wizard.field == 2, false));
                lines.extend(input("SMTP Port", &wizard.smtp_port, wizard.field == 3, false));
            } else {
                lines.push(Line::styled(format!("  IMAP  {}:{}", wizard.imap_host, wizard.imap_port), theme::muted()));
                lines.push(Line::styled(format!("  SMTP  {}:{}", wizard.smtp_host, wizard.smtp_port), theme::muted()));
                lines.push(Line::default());
            }
            let note = match wizard.hint {
                Hint::AppPassword => Some("Your normal password will not work for mail apps. Create an app password and paste it here — it is a password just for TorroMail, and you can revoke it any time."),
                Hint::OAuthNotYet if wizard.setup_url.is_none() => Some("The one-click sign-in is not available in this version yet. Use the password for this mailbox — that only works if your administrator still allows it."),
                Hint::OAuthNotYet => Some("The one-click sign-in is not available in this version yet. An app password works just as well and needs two-factor to be on."),
                _ => None,
            };
            if let Some(note) = note {
                lines.push(Line::styled(lang.t(note), theme::muted()));
                if let Some(url) = &wizard.setup_url {
                    lines.push(Line::styled(url.clone(), Style::new().fg(theme::CYAN)));
                }
                lines.push(Line::default());
            }
            let on_password = if wizard.manual { wizard.field == 4 } else { true };
            let label = if wizard.setup_url.is_some() { "App password" } else { "Password" };
            lines.extend(input(label, &wizard.password.0, on_password, true));
        }
        Step::Rights => {
            lines.push(Line::styled(lang.t("What may connected assistants do with this account?"), theme::bold()));
            lines.push(Line::default());
            let titles = ["Read only", "Read + drafts", "Tidy up", "Full access"];
            for (index, _) in PermissionPreset::ALL.iter().enumerate() {
                let on = index == wizard.preset;
                let line = Line::from(vec![
                    Span::styled(if on { "(•) " } else { "( ) " }, Style::new().fg(if on { theme::ACCENT } else { theme::FAINT })),
                    Span::styled(lang.t(titles[index]), if on { theme::bold() } else { Style::new() }),
                ]);
                lines.push(if index == wizard.field { line.style(theme::selected()) } else { line });
            }
            lines.push(Line::default());
            lines.push(Line::styled(lang.t("You can change this any time."), theme::muted()));
            lines.push(Line::default());
            if wizard.checking {
                lines.push(Line::styled(
                    format!("⠹ {}", lang.t("Checking the connection…")),
                    Style::new().fg(theme::CYAN).add_modifier(Modifier::BOLD),
                ));
            }
        }
        Step::Done => {
            lines.push(Line::styled(
                format!("✓ {} {}", lang.t("Connected as"), wizard.created.as_deref().unwrap_or_default()),
                Style::new().fg(theme::GREEN).add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::default());
            lines.push(Line::styled(lang.t("Connect an assistant in the MCP Clients section so it can use this account."), theme::muted()));
        }
    }

    if let Some(error) = &wizard.error {
        lines.push(Line::styled(lang.t_owned(error), Style::new().fg(theme::ACCENT).add_modifier(Modifier::BOLD)));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
}
