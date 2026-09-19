//! The Torro palette: red and silver from the brand, everything else quiet.
//! Backgrounds stay the terminal's own, so the surface sits inside whatever
//! theme the user runs rather than fighting it.

use ratatui::style::{Color, Modifier, Style};

pub const RED: Color = Color::Rgb(213, 12, 12);
pub const ACCENT: Color = Color::Rgb(238, 58, 51);
pub const SILVER: Color = Color::Rgb(196, 195, 195);
pub const MUTED: Color = Color::Rgb(151, 138, 141);
pub const FAINT: Color = Color::Rgb(106, 94, 97);
pub const LINE: Color = Color::Rgb(90, 76, 80);
pub const GREEN: Color = Color::Rgb(134, 207, 125);
pub const AMBER: Color = Color::Rgb(236, 183, 85);
pub const CYAN: Color = Color::Rgb(121, 191, 211);
pub const SELECTION: Color = Color::Rgb(52, 39, 43);
pub const KEY: Color = Color::Rgb(61, 49, 53);

#[must_use]
pub fn bold() -> Style {
    Style::new().add_modifier(Modifier::BOLD)
}

#[must_use]
pub fn muted() -> Style {
    Style::new().fg(MUTED)
}

#[must_use]
pub fn faint() -> Style {
    Style::new().fg(FAINT)
}

#[must_use]
pub fn selected() -> Style {
    Style::new().bg(SELECTION).add_modifier(Modifier::BOLD)
}
