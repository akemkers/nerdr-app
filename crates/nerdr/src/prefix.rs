//! tmux-style keys: the prefix, then one key. Spaces are tmux's sessions, tabs its windows.

use gpui::Keystroke;

/// The prefix key, from tmux's name for it (`C-b`, `C-f`, `M-a`, `C-Space`).
#[derive(Clone, Debug, PartialEq)]
pub struct Prefix {
    key: String,
    ctrl: bool,
    alt: bool,
}

impl Default for Prefix {
    fn default() -> Self {
        Prefix::parse("C-b").unwrap()
    }
}

impl Prefix {
    pub fn parse(name: &str) -> Option<Prefix> {
        let (mut ctrl, mut alt, mut rest) = (false, false, name.trim());
        loop {
            if let Some(r) = rest.strip_prefix("C-") {
                (ctrl, rest) = (true, r);
            } else if let Some(r) = rest.strip_prefix("M-") {
                (alt, rest) = (true, r);
            } else {
                break;
            }
        }
        let key = match rest {
            "Space" => "space".to_string(),
            r if r.chars().count() == 1 => r.to_lowercase(),
            _ => return None,
        };
        Some(Prefix { key, ctrl, alt })
    }

    pub fn matches(&self, ks: &Keystroke) -> bool {
        let m = &ks.modifiers;
        ks.key == self.key && m.control == self.ctrl && m.alt == self.alt && !m.platform
    }

    /// As tmux writes it, for the sidebar.
    pub fn name(&self) -> String {
        let key = if self.key == "space" { "Space" } else { &self.key };
        format!("{}{}{key}", if self.ctrl { "C-" } else { "" }, if self.alt { "M-" } else { "" })
    }

    /// The prefix as a keystroke, to send it on to the program.
    pub fn keystroke(&self) -> Keystroke {
        let mut s = String::new();
        if self.ctrl {
            s += "ctrl-";
        }
        if self.alt {
            s += "alt-";
        }
        s += &self.key;
        Keystroke::parse(&s).expect("the prefix is a valid keystroke")
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    NewTab,
    NextTab,
    PrevTab,
    /// By position in the space, from 0.
    SelectTab(usize),
    LastTab,
    CloseTab,
    NewSpace,
    NextSpace,
    PrevSpace,
    ScrollPageUp,
    SendPrefix,
    Quit,
}

/// The key after the prefix as tmux names it: the character it typed when it typed one
/// (so `&` is `&` on any layout), else the key's name. Control is written `C-`.
pub fn key_name(ks: &Keystroke) -> String {
    let m = &ks.modifiers;
    let typed = ks
        .key_char
        .as_ref()
        .filter(|t| t.chars().count() == 1 && !t.chars().any(char::is_control));
    let base = match (typed, ks.key.as_str()) {
        (_, "pageup") => "PPage".to_string(),
        (_, "pagedown") => "NPage".to_string(),
        (_, "space") => "Space".to_string(),
        (Some(t), _) if !m.control => t.clone(),
        (_, k) => k.to_string(),
    };
    if m.control { format!("C-{base}") } else { base }
}

/// tmux's default prefix table, for what nerdr has. `base` is tmux's `base-index`.
pub fn default_action(key: &str, base: usize) -> Option<Action> {
    Some(match key {
        "c" => Action::NewTab,
        "n" => Action::NextTab,
        "p" => Action::PrevTab,
        "l" => Action::LastTab,
        "&" => Action::CloseTab,
        ")" => Action::NextSpace,
        "(" => Action::PrevSpace,
        "PPage" => Action::ScrollPageUp,
        "d" => Action::Quit,
        // nerdr's own, as in the tmux nerdr: a new space where tmux would make a session
        "f" => Action::NewSpace,
        d if d.len() == 1 && d.as_bytes()[0].is_ascii_digit() => {
            let n = (d.as_bytes()[0] - b'0') as usize;
            Action::SelectTab(n.checked_sub(base)?)
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ks(s: &str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    #[test]
    fn prefix_from_tmux_names() {
        let p = Prefix::parse("C-f").unwrap();
        assert!(p.matches(&ks("ctrl-f")));
        assert!(!p.matches(&ks("f")));
        assert!(!p.matches(&ks("ctrl-alt-f")));
        assert_eq!(p.name(), "C-f");
        assert_eq!(p.keystroke(), ks("ctrl-f"));
        assert!(Prefix::parse("C-Space").unwrap().matches(&ks("ctrl-space")));
        assert_eq!(Prefix::parse("C-Left"), None);
    }

    #[test]
    fn keys_after_the_prefix() {
        let mut amp = ks("shift-6");
        amp.key_char = Some("&".into());
        assert_eq!(key_name(&amp), "&");
        assert_eq!(key_name(&ks("pageup")), "PPage");
        assert_eq!(key_name(&ks("ctrl-o")), "C-o");
        assert_eq!(default_action("&", 0), Some(Action::CloseTab));
        assert_eq!(default_action("1", 1), Some(Action::SelectTab(0)));
        assert_eq!(default_action("0", 1), None);
        assert_eq!(default_action("0", 0), Some(Action::SelectTab(0)));
    }
}
