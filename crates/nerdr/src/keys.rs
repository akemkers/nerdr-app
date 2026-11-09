//! GPUI keystrokes to the bytes a legacy xterm-style terminal sends.

use alacritty_terminal::term::TermMode;
use gpui::Keystroke;

/// `None` when the key sends nothing (a lone modifier, or a key we don't know).
pub fn encode(ks: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
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

    #[test]
    fn basics() {
        let m = TermMode::empty();
        assert_eq!(encode(&ks("ctrl-c"), m).unwrap(), vec![3]);
        assert_eq!(encode(&ks("alt-enter"), m).unwrap(), b"\x1b\r");
        assert_eq!(encode(&ks("up"), m).unwrap(), b"\x1b[A");
        assert_eq!(encode(&ks("up"), TermMode::APP_CURSOR).unwrap(), b"\x1bOA");
        assert_eq!(encode(&ks("ctrl-left"), m).unwrap(), b"\x1b[1;5D");
        assert_eq!(encode(&ks("shift-tab"), m).unwrap(), b"\x1b[Z");
        assert_eq!(encode(&ks("f5"), m).unwrap(), b"\x1b[15~");
        assert_eq!(encode(&ks("backspace"), m).unwrap(), vec![0x7f]);
    }
}
