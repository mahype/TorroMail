//! Typing into a one-line field: the keys that edit text, the caret they
//! move, and which keys are commands rather than characters.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Whether a key is a Ctrl shortcut rather than something typed.
///
/// On Windows AltGr arrives as Ctrl+Alt, so a German `@` (AltGr+Q), `€`, `{`
/// or `\` carries Ctrl too. The character has already been resolved by
/// then; treating it as a shortcut would make those characters untypable.
#[must_use]
pub fn is_command(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && !key.modifiers.contains(KeyModifiers::ALT)
}

/// Applies an editing key to `text`. The caret is counted in characters from
/// the end, so 0 — the default, and where a freshly focused field wants it —
/// is behind the last character. Returns whether the key was an editing key.
pub fn edit(text: &mut String, caret_back: &mut usize, code: KeyCode) -> bool {
    let length = text.chars().count();
    *caret_back = (*caret_back).min(length);
    let at = length - *caret_back;
    match code {
        KeyCode::Char(character) => text.insert(byte_index(text, at), character),
        KeyCode::Backspace if at > 0 => {
            text.remove(byte_index(text, at - 1));
        }
        KeyCode::Delete if *caret_back > 0 => {
            text.remove(byte_index(text, at));
            *caret_back -= 1;
        }
        KeyCode::Left => *caret_back = (*caret_back + 1).min(length),
        KeyCode::Right => *caret_back = caret_back.saturating_sub(1),
        KeyCode::Home => *caret_back = length,
        KeyCode::End => *caret_back = 0,
        KeyCode::Backspace | KeyCode::Delete => {}
        _ => return false,
    }
    true
}

/// `shown` split at the caret, for drawing: what is before it, the character
/// under it (a space at the end), and what follows.
#[must_use]
pub fn split(shown: &str, caret_back: usize) -> (String, String, String) {
    let length = shown.chars().count();
    let at = length - caret_back.min(length);
    let before = shown.chars().take(at).collect();
    let under = shown.chars().nth(at).map_or_else(|| " ".to_owned(), String::from);
    let after = shown.chars().skip(at + 1).collect();
    (before, under, after)
}

fn byte_index(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(keys: &[KeyCode]) -> (String, usize) {
        let (mut text, mut caret) = (String::new(), 0);
        for key in keys {
            edit(&mut text, &mut caret, *key);
        }
        (text, caret)
    }

    #[test]
    fn typing_inserts_at_the_caret() {
        use KeyCode::{Char, Left};
        assert_eq!(run(&[Char('a'), Char('c'), Left, Char('b')]).0, "abc");
    }

    #[test]
    fn deleting_works_on_both_sides_of_the_caret() {
        use KeyCode::{Backspace, Char, Delete, Home, Left};
        assert_eq!(run(&[Char('a'), Char('b'), Char('c'), Left, Backspace]).0, "ac");
        assert_eq!(run(&[Char('a'), Char('b'), Home, Delete]), ("b".to_owned(), 1));
    }

    #[test]
    fn multibyte_characters_are_one_step() {
        use KeyCode::{Backspace, Char, Left};
        assert_eq!(run(&[Char('ü'), Char('€'), Left, Backspace]).0, "€");
    }

    #[test]
    fn the_caret_stays_inside_the_text() {
        use KeyCode::{Char, Left, Right};
        assert_eq!(run(&[Char('a'), Left, Left, Left]).1, 1);
        assert_eq!(run(&[Char('a'), Right, Right]).1, 0);
    }

    #[test]
    fn altgr_is_typing_not_a_shortcut() {
        let altgr = KeyEvent::new(KeyCode::Char('@'), KeyModifiers::CONTROL | KeyModifiers::ALT);
        assert!(!is_command(&altgr));
        assert!(is_command(&KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn split_marks_the_character_under_the_caret() {
        assert_eq!(split("abc", 1), ("ab".to_owned(), "c".to_owned(), String::new()));
        assert_eq!(split("abc", 0), ("abc".to_owned(), " ".to_owned(), String::new()));
    }
}
