//! Settings, Updates and Help: quiet text screens.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
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
        Section::Settings => vec![
            Line::raw(lang.t("Nothing to set here yet.")),
            Line::styled(lang.t("Autostart and notifications arrive with the background service."), theme::muted()),
        ],
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
