//! tmux-style keys: the prefix, then one key. Spaces are tmux's sessions, tabs its windows.
//! The prefix, `base-index` and the bindings come from a running tmux, so the user's
//! `tmux.conf` applies; without one, tmux's defaults.

use std::collections::HashMap;
use std::process::Command;

use gpui::Keystroke;

/// Overrides the prefix; the same variable as the tmux-based nerdr.
const PREFIX_VAR: &str = "NERDR_PREFIKS";

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
    let named = match ks.key.as_str() {
        "pageup" => Some("PPage"),
        "pagedown" => Some("NPage"),
        "space" => Some("Space"),
        "up" => Some("Up"),
        "down" => Some("Down"),
        "left" => Some("Left"),
        "right" => Some("Right"),
        "home" => Some("Home"),
        "end" => Some("End"),
        "tab" => Some("Tab"),
        "enter" => Some("Enter"),
        "escape" => Some("Escape"),
        "backspace" => Some("BSpace"),
        "delete" => Some("DC"),
        "insert" => Some("IC"),
        _ => None,
    };
    let base = match (named, typed) {
        (Some(n), _) => n.to_string(),
        (None, Some(t)) if !m.control && !m.alt => t.clone(),
        (None, _) if ks.key.starts_with('f') && ks.key.len() > 1 => ks.key.to_uppercase(),
        (None, _) => ks.key.clone(),
    };
    let mut name = String::new();
    if m.control {
        name += "C-";
    }
    if m.alt {
        name += "M-";
    }
    // Shift is in the character for typed keys (`X`, `&`), tmux writes it only for the rest
    if m.shift && named.is_some() {
        name += "S-";
    }
    name + &base
}

/// The prefix, numbering and bindings in use.
#[derive(Default)]
pub struct Keys {
    pub prefix: Prefix,
    /// tmux's `base-index`: the number of the first tab.
    pub base_index: usize,
    /// tmux's prefix table, mapped to what nerdr can do. `None`: no tmux, use the defaults.
    table: Option<HashMap<String, Action>>,
}

impl Keys {
    /// From the running tmux server, if any. A server isn't started for this: that would
    /// run the user's tmux plugins (session restore among them).
    pub fn load() -> Keys {
        let tmux = |args: &[&str]| {
            let out = Command::new("tmux").args(args).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        };
        let mut keys = Keys::default();
        if let Some(p) = tmux(&["show-options", "-gv", "prefix"]).and_then(|p| Prefix::parse(&p)) {
            keys.prefix = p;
        }
        if let Some(b) = tmux(&["show-options", "-gv", "base-index"]).and_then(|b| b.trim().parse().ok()) {
            keys.base_index = b;
        }
        if let Some(list) = tmux(&["list-keys", "-T", "prefix"]) {
            keys.table = Some(parse_table(&list, keys.base_index));
        }
        if let Some(p) = std::env::var(PREFIX_VAR).ok().and_then(|p| Prefix::parse(&p)) {
            keys.prefix = p;
        }
        if std::env::var_os("NERDR_DEBUG").is_some() {
            let mut bound: Vec<_> = keys.table.iter().flatten().collect();
            bound.sort_by(|a, b| a.0.cmp(b.0));
            eprintln!(
                "nerdr: prefix {}, base-index {}, tmux bindings {bound:?}",
                keys.prefix.name(),
                keys.base_index
            );
        }
        keys
    }

    /// What the key after the prefix does: nerdr's own keys, else the tmux table.
    pub fn action(&self, key: &str) -> Option<Action> {
        if key == "f" {
            return Some(Action::NewSpace);
        }
        match &self.table {
            Some(table) => table.get(key).copied(),
            None => default_action(key, self.base_index),
        }
    }
}

/// `tmux list-keys -T prefix` to actions. Lines whose command nerdr has no action for
/// (panes, menus, prompts, chains) are left out, so those keys do nothing yet.
fn parse_table(list: &str, base: usize) -> HashMap<String, Action> {
    let mut table = HashMap::new();
    for line in list.lines() {
        let mut words = line.split_whitespace();
        if words.next() != Some("bind-key") {
            continue;
        }
        let mut key = None;
        while let Some(w) = words.next() {
            match w {
                "-r" | "-n" => {}
                "-T" | "-N" => {
                    words.next();
                }
                k => {
                    key = Some(k);
                    break;
                }
            }
        }
        let Some(key) = key else { continue };
        // list-keys escapes # $ ' ; " ~ { } with a backslash
        let key = key.strip_prefix('\\').filter(|k| !k.is_empty()).unwrap_or(key);
        let command: Vec<&str> = words.collect();
        if let Some(a) = command_action(&command, base) {
            table.insert(key.to_string(), a);
        }
    }
    table
}

fn command_action(command: &[&str], base: usize) -> Option<Action> {
    let (name, args) = command.split_first()?;
    if args.iter().any(|a| *a == "\\;" || a.starts_with('{')) {
        return None;
    }
    let has = |flag: &str| args.contains(&flag);
    Some(match *name {
        "new-window" => Action::NewTab,
        "new-session" => Action::NewSpace,
        "next-window" => Action::NextTab,
        "previous-window" => Action::PrevTab,
        "last-window" => Action::LastTab,
        "kill-window" => Action::CloseTab,
        // tmux's default `&` asks first; nerdr closes the tab right away
        "confirm-before" if args.last() == Some(&"kill-window") => Action::CloseTab,
        "switch-client" if has("-n") => Action::NextSpace,
        "switch-client" if has("-p") => Action::PrevSpace,
        "detach-client" => Action::Quit,
        "copy-mode" if has("-u") => Action::ScrollPageUp,
        "send-prefix" => Action::SendPrefix,
        "select-window" => {
            let t = args.iter().position(|a| *a == "-t").and_then(|i| args.get(i + 1))?;
            let n: usize = t.trim_start_matches([':', '=']).parse().ok()?;
            Action::SelectTab(n.checked_sub(base)?)
        }
        _ => return None,
    })
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
        assert_eq!(key_name(&ks("shift-up")), "S-Up");
        assert_eq!(key_name(&ks("alt-left")), "M-Left");
    }

    #[test]
    fn table_from_list_keys() {
        let list = r#"bind-key    -T prefix (       switch-client -p
bind-key    -T prefix \;      last-pane
bind-key    -T prefix 1       select-window -t :=1
bind-key    -T prefix &       confirm-before -p "kill-window #W? (y/n)" kill-window
bind-key -r -T prefix C-Left  resize-pane -L
bind-key    -T prefix X       kill-window
bind-key    -T prefix f       command-prompt { find-window -Z "%%" }
bind-key    -T prefix C-f     send-prefix
bind-key    -T prefix PPage   copy-mode -u
bind-key    -T prefix r       source-file ~/.tmux.conf \; display-message "reloaded"
"#;
        let t = parse_table(list, 1);
        assert_eq!(t.get("("), Some(&Action::PrevSpace));
        assert_eq!(t.get("1"), Some(&Action::SelectTab(0)));
        assert_eq!(t.get("&"), Some(&Action::CloseTab));
        assert_eq!(t.get("X"), Some(&Action::CloseTab));
        assert_eq!(t.get("C-f"), Some(&Action::SendPrefix));
        assert_eq!(t.get("PPage"), Some(&Action::ScrollPageUp));
        assert_eq!(t.get(";"), None);
        assert_eq!(t.get("f"), None);
        assert_eq!(t.get("r"), None);
        let keys = Keys { prefix: Prefix::default(), base_index: 1, table: Some(t) };
        assert_eq!(keys.action("f"), Some(Action::NewSpace));
        assert_eq!(keys.action("c"), None);
    }
}
