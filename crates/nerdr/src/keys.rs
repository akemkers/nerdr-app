//! GPUI keystrokes to the bytes a terminal sends: legacy xterm-style, or the kitty
//! keyboard protocol when the program asked for it.

use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum KeyAction {
    Press,
    Repeat,
    Release,
}

/// `None` when the key sends nothing (a lone modifier, a release, or a key we don't know).
pub fn encode(ks: &Keystroke, mode: TermMode, action: KeyAction) -> Option<Vec<u8>> {
    if mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL) {
        return kitty(ks, mode, action);
    }
    match action {
        KeyAction::Release => None,
        _ => legacy(ks, mode),
    }
}

fn legacy(ks: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let m = &ks.modifiers;
    // xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl
    let param = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.control as u8;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);

    // Cursor keys and friends: CSI 1;m X with modifiers, SS3 X in app-cursor mode.
    let letter = match ks.key.as_str() {
        "up" => Some(b'A'),
        "down" => Some(b'B'),
        "right" => Some(b'C'),
        "left" => Some(b'D'),
        "home" => Some(b'H'),
        "end" => Some(b'F'),
        "f1" => Some(b'P'),
        "f2" => Some(b'Q'),
        "f3" => Some(b'R'),
        "f4" => Some(b'S'),
        _ => None,
    };
    if let Some(l) = letter {
        let is_f = matches!(l, b'P'..=b'S');
        return Some(if param > 1 {
            format!("\x1b[1;{param}{}", l as char).into_bytes()
        } else if app_cursor || is_f {
            vec![0x1b, b'O', l]
        } else {
            vec![0x1b, b'[', l]
        });
    }
    let tilde = match ks.key.as_str() {
        "insert" => Some(2),
        "delete" => Some(3),
        "pageup" => Some(5),
        "pagedown" => Some(6),
        "f5" => Some(15),
        "f6" => Some(17),
        "f7" => Some(18),
        "f8" => Some(19),
        "f9" => Some(20),
        "f10" => Some(21),
        "f11" => Some(23),
        "f12" => Some(24),
        _ => None,
    };
    if let Some(n) = tilde {
        return Some(if param > 1 {
            format!("\x1b[{n};{param}~")
        } else {
            format!("\x1b[{n}~")
        }
        .into_bytes());
    }

    let alt = |mut b: Vec<u8>| {
        if m.alt {
            b.insert(0, 0x1b);
        }
        b
    };
    match ks.key.as_str() {
        "enter" => return Some(alt(vec![b'\r'])),
        "tab" if m.shift => return Some(b"\x1b[Z".to_vec()),
        "tab" => return Some(alt(vec![b'\t'])),
        "escape" => return Some(alt(vec![0x1b])),
        "backspace" if m.control => return Some(alt(vec![0x08])),
        "backspace" => return Some(alt(vec![0x7f])),
        "space" if m.control => return Some(alt(vec![0])),
        _ => {}
    }

    if m.control {
        let k = ks.key.as_bytes();
        if k.len() != 1 {
            return None;
        }
        let b = match k[0].to_ascii_lowercase() {
            c @ b'a'..=b'z' => c - b'a' + 1,
            b'@' | b'2' | b' ' => 0,
            b'[' | b'3' => 0x1b,
            b'\\' | b'4' => 0x1c,
            b']' | b'5' => 0x1d,
            b'^' | b'6' => 0x1e,
            b'_' | b'-' | b'/' | b'7' => 0x1f,
            b'8' | b'?' => 0x7f,
            _ => return None,
        };
        return Some(alt(vec![b]));
    }

    let text = match &ks.key_char {
        Some(t) => t.clone(),
        // With alt, some layouts give no key_char
        None if m.alt && ks.key.chars().count() == 1 => ks.key.clone(),
        None if ks.key == "space" => " ".into(),
        None => return None,
    };
    Some(alt(text.into_bytes()))
}

/// Keys with their own CSI number and final byte in the kitty protocol.
fn functional(key: &str) -> Option<(u8, char)> {
    Some(match key {
        "up" => (1, 'A'),
        "down" => (1, 'B'),
        "right" => (1, 'C'),
        "left" => (1, 'D'),
        "home" => (1, 'H'),
        "end" => (1, 'F'),
        "f1" => (1, 'P'),
        "f2" => (1, 'Q'),
        "f3" => (13, '~'),
        "f4" => (1, 'S'),
        "insert" => (2, '~'),
        "delete" => (3, '~'),
        "pageup" => (5, '~'),
        "pagedown" => (6, '~'),
        "f5" => (15, '~'),
        "f6" => (17, '~'),
        "f7" => (18, '~'),
        "f8" => (19, '~'),
        "f9" => (20, '~'),
        "f10" => (21, '~'),
        "f11" => (23, '~'),
        "f12" => (24, '~'),
        _ => return None,
    })
}

/// https://sw.kovidgoyal.net/kitty/keyboard-protocol/. Lone modifiers and the base layout
/// key aren't reported: GPUI gives neither.
fn kitty(ks: &Keystroke, mode: TermMode, action: KeyAction) -> Option<Vec<u8>> {
    let m = &ks.modifiers;
    let all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let action = match (mode.contains(TermMode::REPORT_EVENT_TYPES), action) {
        (false, KeyAction::Release) => return None,
        (false, _) => KeyAction::Press,
        (true, a) => a,
    };
    let mods = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.control as u8 + 8 * m.platform as u8;
    let event = match action {
        KeyAction::Press => "",
        KeyAction::Repeat => ":2",
        KeyAction::Release => ":3",
    };
    let key = ks.key.as_str();

    if let Some((n, fin)) = functional(key) {
        if !all && mods == 1 && action == KeyAction::Press {
            return legacy(ks, mode);
        }
        return Some(if mods == 1 && event.is_empty() {
            if fin == '~' { format!("\x1b[{n}~") } else { format!("\x1b[{fin}") }
        } else {
            format!("\x1b[{n};{mods}{event}{fin}")
        }
        .into_bytes());
    }

    let code = match key {
        "enter" => 13,
        "tab" => 9,
        "backspace" => 127,
        "escape" => 27,
        "space" => 32,
        k if k.chars().count() == 1 => k.chars().next()?.to_lowercase().next()? as u32,
        _ => return None,
    };
    let text: Option<String> = ks
        .key_char
        .clone()
        .or_else(|| (key == "space").then(|| " ".into()))
        .filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
        .filter(|_| !m.control && !m.alt && !m.platform);

    if !all {
        // Text, and plain enter/tab/backspace, go out as in legacy mode; they have no events
        let plain_legacy = mods == 1 && matches!(code, 13 | 9 | 127);
        if text.is_some() || plain_legacy {
            return match action {
                KeyAction::Release => None,
                _ => legacy(ks, mode),
            };
        }
    }

    let mut out = format!("\x1b[{code}");
    if mode.contains(TermMode::REPORT_ALTERNATE_KEYS) && m.shift {
        let shifted = text.as_deref().and_then(|t| t.chars().next()).map(|c| c as u32);
        if let Some(sh) = shifted.filter(|&sh| sh != code) {
            out += &format!(":{sh}");
        }
    }
    let text = text
        .filter(|_| mode.contains(TermMode::REPORT_ASSOCIATED_TEXT) && action != KeyAction::Release)
        .map(|t| t.chars().map(|c| (c as u32).to_string()).collect::<Vec<_>>().join(":"));
    match (&text, mods, event) {
        (None, 1, "") => {}
        (Some(_), 1, "") => out += ";",
        _ => out += &format!(";{mods}{event}"),
    }
    if let Some(t) = text {
        out += &format!(";{t}");
    }
    out.push('u');
    Some(out.into_bytes())
}

/// Pasted text: bracketed when the program asked for it, else newlines become CR.
pub fn paste(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let clean = text.replace('\x1b', "");
        format!("\x1b[200~{clean}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(s: &str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    fn enc(s: &str, mode: TermMode) -> Vec<u8> {
        encode(&ks(s), mode, KeyAction::Press).unwrap()
    }

    #[test]
    fn basics() {
        let m = TermMode::empty();
        assert_eq!(enc("ctrl-c", m), vec![3]);
        assert_eq!(enc("alt-enter", m), b"\x1b\r");
        assert_eq!(enc("up", m), b"\x1b[A");
        assert_eq!(enc("up", TermMode::APP_CURSOR), b"\x1bOA");
        assert_eq!(enc("ctrl-left", m), b"\x1b[1;5D");
        assert_eq!(enc("shift-tab", m), b"\x1b[Z");
        assert_eq!(enc("f5", m), b"\x1b[15~");
        assert_eq!(enc("backspace", m), vec![0x7f]);
    }

    #[test]
    fn kitty_disambiguate() {
        let m = TermMode::DISAMBIGUATE_ESC_CODES;
        assert_eq!(enc("shift-enter", m), b"\x1b[13;2u");
        assert_eq!(enc("enter", m), b"\r");
        assert_eq!(enc("escape", m), b"\x1b[27u");
        assert_eq!(enc("ctrl-c", m), b"\x1b[99;5u");
        assert_eq!(enc("alt-a", m), b"\x1b[97;3u");
        assert_eq!(enc("ctrl-up", m), b"\x1b[1;5A");
        assert_eq!(enc("shift-tab", m), b"\x1b[9;2u");
        assert_eq!(enc("f3", m), b"\x1bOR");
        assert_eq!(enc("ctrl-f3", m), b"\x1b[13;5~");
        let release = encode(&ks("ctrl-c"), m, KeyAction::Release);
        assert_eq!(release, None);
    }

    #[test]
    fn kitty_all_keys_and_events() {
        let m = TermMode::DISAMBIGUATE_ESC_CODES
            | TermMode::REPORT_ALL_KEYS_AS_ESC
            | TermMode::REPORT_EVENT_TYPES;
        assert_eq!(enc("enter", m), b"\x1b[13u");
        assert_eq!(enc("up", m), b"\x1b[A");
        let up = encode(&ks("ctrl-c"), m, KeyAction::Release).unwrap();
        assert_eq!(up, b"\x1b[99;5:3u");
        let held = encode(&ks("left"), m, KeyAction::Repeat).unwrap();
        assert_eq!(held, b"\x1b[1;1:2D");
    }
}
