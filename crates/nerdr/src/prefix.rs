//! tmux-style keys: the prefix, then one key. Spaces are tmux's sessions, tabs its windows.
//! The prefix, `base-index` and the bindings come from a running tmux, so the user's
//! `tmux.conf` applies; without one, tmux's defaults.

use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

use gpui::Keystroke;
use regex::Regex;

use crate::panes::{Dir, Side};

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
    /// tmux's split-window: `-h` side by side, else one above the other.
    Split(Dir),
    ClosePane,
    SelectPane(Side),
    /// tmux's resize-pane -Z: the active pane fills the tab, and back.
    Zoom,
    /// tmux's resize-pane -L/-R/-U/-D: that many cells.
    Resize(Side, u16),
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
    table: Option<Table>,
    /// tmux's root table: keys that act without the prefix (`bind -n`).
    root: Table,
    /// tmux's `repeat-time`: how long a repeatable key (`bind -r`) works without the prefix.
    pub repeat_time: Duration,
}

type Table = HashMap<String, Binding>;

#[derive(Clone, Debug)]
pub struct Binding {
    pub action: Action,
    /// tmux's `-r`: works again without the prefix within `repeat-time`.
    pub repeat: bool,
    /// The key goes to the program instead when the pane's program matches, as with
    /// vim-tmux-navigator's `if-shell "ps … | grep -iqE '<pattern>'" "send-keys <key>" …`.
    pub unless: Option<Regex>,
}

impl Binding {
    fn new(action: Action, repeat: bool) -> Self {
        Binding {
            action,
            repeat,
            unless: None,
        }
    }

    /// Whether the key is the program's: `name` is the pane's foreground program.
    pub fn passes_to(&self, name: Option<&str>) -> bool {
        // The pattern is written for `ps -o state= -o comm=` lines; a running process
        // fills in the state
        match (&self.unless, name) {
            (Some(re), Some(name)) => re.is_match(&format!("S {name}")),
            _ => false,
        }
    }
}

impl Keys {
    /// From the running tmux server, if any. A server isn't started for this: that would
    /// run the user's tmux plugins (session restore among them).
    pub fn load() -> Keys {
        let tmux = |args: &[&str]| {
            let out = Command::new("tmux").args(args).output().ok()?;
            out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
        };
        let mut keys = Keys {
            repeat_time: Duration::from_millis(500),
            ..Keys::default()
        };
        if let Some(p) = tmux(&["show-options", "-gv", "prefix"]).and_then(|p| Prefix::parse(&p)) {
            keys.prefix = p;
        }
        if let Some(b) = tmux(&["show-options", "-gv", "base-index"]).and_then(|b| b.trim().parse().ok()) {
            keys.base_index = b;
        }
        if let Some(ms) = tmux(&["show-options", "-gv", "repeat-time"]).and_then(|m| m.trim().parse().ok()) {
            keys.repeat_time = Duration::from_millis(ms);
        }
        if let Some(list) = tmux(&["list-keys", "-T", "prefix"]) {
            keys.table = Some(parse_table(&list, keys.base_index));
        }
        if let Some(list) = tmux(&["list-keys", "-T", "root"]) {
            keys.root = parse_table(&list, keys.base_index);
        }
        if let Some(p) = std::env::var(PREFIX_VAR).ok().and_then(|p| Prefix::parse(&p)) {
            keys.prefix = p;
        }
        if std::env::var_os("NERDR_DEBUG").is_some() {
            let sorted = |t: &Table| {
                let mut v: Vec<_> = t
                    .iter()
                    .map(|(k, b)| (k.clone(), b.action, b.repeat, b.unless.as_ref().map(|r| r.to_string())))
                    .collect();
                v.sort_by(|a, b| a.0.cmp(&b.0));
                v
            };
            eprintln!(
                "nerdr: prefix {}, base-index {}, repeat-time {:?}, tmux prefix table {:?}, root table {:?}",
                keys.prefix.name(),
                keys.base_index,
                keys.repeat_time,
                keys.table.as_ref().map(sorted),
                sorted(&keys.root),
            );
        }
        keys
    }

    /// What the key after the prefix does: nerdr's own keys, else the tmux table.
    pub fn action(&self, key: &str) -> Option<Binding> {
        if key == "f" {
            return Some(Binding::new(Action::NewSpace, false));
        }
        match &self.table {
            Some(table) => table.get(key).cloned(),
            // tmux's defaults repeat the arrows, and resizing with C- and M-arrows
            None => default_action(key, self.base_index)
                .map(|a| Binding::new(a, matches!(a, Action::SelectPane(_) | Action::Resize(..)))),
        }
    }

    /// What a key does without the prefix, from tmux's root table.
    pub fn root(&self, key: &str) -> Option<&Binding> {
        self.root.get(key)
    }
}

/// `tmux list-keys -T prefix` to actions. Lines whose command nerdr has no action for
/// (panes, menus, prompts, chains) are left out, so those keys do nothing yet.
fn parse_table(list: &str, base: usize) -> Table {
    let mut table = HashMap::new();
    for line in list.lines() {
        let words = tokens(line);
        let mut words = words.iter().map(String::as_str);
        if words.next() != Some("bind-key") {
            continue;
        }
        let mut key = None;
        let mut repeat = false;
        while let Some(w) = words.next() {
            match w {
                "-r" => repeat = true,
                "-n" => {}
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
        let command: Vec<&str> = words.collect();
        if let Some(b) = vim_navigator(key, &command, base) {
            table.insert(key.to_string(), b);
        } else if let Some(action) = command_action(&command, base) {
            table.insert(key.to_string(), Binding::new(action, repeat));
        }
    }
    table
}

/// Splits a tmux command line into words as tmux does: `"…"` with backslash escapes,
/// `'…'` as is, and a backslash outside quotes escaping the next character (`\;`, `\#`).
fn tokens(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => out.extend(word.take()),
            '\\' => word.get_or_insert_default().extend(chars.next()),
            '"' => {
                let w = word.get_or_insert_default();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => w.extend(chars.next()),
                        c => w.push(c),
                    }
                }
            }
            '\'' => {
                let w = word.get_or_insert_default();
                w.extend(chars.by_ref().take_while(|&c| c != '\''));
            }
            c => word.get_or_insert_default().push(c),
        }
    }
    out.extend(word);
    out
}

/// vim-tmux-navigator's root binding: `if-shell "ps … | grep -iqE '<pattern>'"
/// "send-keys <the key>" "<command>"`. The command, unless the pane runs a program the
/// pattern matches (vim, fzf), which then gets the key.
fn vim_navigator(key: &str, command: &[&str], base: usize) -> Option<Binding> {
    let ["if-shell", test, then, otherwise] = command else {
        return None;
    };
    let pattern = test.split_once("grep -iqE '")?.1.split('\'').next()?;
    let then = tokens(then);
    if then.len() != 2 || then[0] != "send-keys" || then[1] != key {
        return None;
    }
    let otherwise = tokens(otherwise);
    let otherwise: Vec<&str> = otherwise.iter().map(String::as_str).collect();
    let action = command_action(&otherwise, base)?;
    let unless = Regex::new(&format!("(?i){pattern}")).ok()?;
    Some(Binding {
        action,
        repeat: false,
        unless: Some(unless),
    })
}

fn command_action(command: &[&str], base: usize) -> Option<Action> {
    let (name, args) = command.split_first()?;
    // tmux 3.8's defaults for C- and M-Up/Left: `if-shell -F "#{?floating_pane_flag}"
    // { floating } { tiled }`. nerdr has no floating panes, so the second block holds.
    if *name == "if-shell" && args.first() == Some(&"-F") && args.get(1) == Some(&"#{?floating_pane_flag}") {
        let rest = &args[2..];
        let close = rest.iter().position(|a| *a == "}")?;
        let tiled = rest.get(close + 1..)?.strip_prefix(&["{"])?.strip_suffix(&["}"])?;
        return command_action(tiled, base);
    }
    if args.iter().any(|a| *a == ";" || a.starts_with('{')) {
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
        "kill-pane" => Action::ClosePane,
        // tmux's defaults for `&` and `x` ask first; nerdr closes right away
        "confirm-before" if args.last() == Some(&"kill-window") => Action::CloseTab,
        "confirm-before" if args.last() == Some(&"kill-pane") => Action::ClosePane,
        // Always in the active pane's directory, as with `-c "#{pane_current_path}"`
        "split-window" if has("-h") => Action::Split(Dir::Horizontal),
        "split-window" => Action::Split(Dir::Vertical),
        "select-pane" if has("-L") => Action::SelectPane(Side::Left),
        "select-pane" if has("-R") => Action::SelectPane(Side::Right),
        "select-pane" if has("-U") => Action::SelectPane(Side::Up),
        "select-pane" if has("-D") => Action::SelectPane(Side::Down),
        "resize-pane" if has("-Z") => Action::Zoom,
        "resize-pane" => {
            let (i, side) = args.iter().enumerate().find_map(|(i, a)| {
                let side = match *a {
                    "-L" => Side::Left,
                    "-R" => Side::Right,
                    "-U" => Side::Up,
                    "-D" => Side::Down,
                    _ => return None,
                };
                Some((i, side))
            })?;
            let n = args.get(i + 1).and_then(|n| n.parse().ok()).unwrap_or(1);
            Action::Resize(side, n)
        }
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
        "x" => Action::ClosePane,
        "%" => Action::Split(Dir::Horizontal),
        "\"" => Action::Split(Dir::Vertical),
        "z" => Action::Zoom,
        "Left" => Action::SelectPane(Side::Left),
        "Right" => Action::SelectPane(Side::Right),
        "Up" => Action::SelectPane(Side::Up),
        "Down" => Action::SelectPane(Side::Down),
        "C-Left" => Action::Resize(Side::Left, 1),
        "C-Right" => Action::Resize(Side::Right, 1),
        "C-Up" => Action::Resize(Side::Up, 1),
        "C-Down" => Action::Resize(Side::Down, 1),
        "M-Left" => Action::Resize(Side::Left, 5),
        "M-Right" => Action::Resize(Side::Right, 5),
        "M-Up" => Action::Resize(Side::Up, 5),
        "M-Down" => Action::Resize(Side::Down, 5),
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
        let list = r##"bind-key    -T prefix (       switch-client -p
bind-key    -T prefix \;      last-pane
bind-key    -T prefix 1       select-window -t :=1
bind-key    -T prefix &       confirm-before -p "kill-window #W? (y/n)" kill-window
bind-key -r -T prefix C-Left  resize-pane -L
bind-key    -T prefix X       kill-window
bind-key    -T prefix x       kill-pane
bind-key    -T prefix o       select-pane -t :.+
bind-key    -T prefix Up      select-pane -U
bind-key    -T prefix z       resize-pane -Z
bind-key -r -T prefix C-Up    resize-pane -U
bind-key -r -T prefix M-Down  resize-pane -D 5
bind-key -r -T prefix M-Up    if-shell -F "#{?floating_pane_flag}" { resize-pane -D -5 } { resize-pane -U 5 }
bind-key    -T prefix v       split-window -h -c "#{pane_current_path}"
bind-key    -T prefix h       split-window -v -c "#{pane_current_path}"
bind-key    -T prefix f       command-prompt { find-window -Z "%%" }
bind-key    -T prefix C-f     send-prefix
bind-key    -T prefix PPage   copy-mode -u
bind-key    -T prefix r       source-file ~/.tmux.conf \; display-message "reloaded"
"##;
        let t = parse_table(list, 1);
        let act = |t: &Table, k: &str| t.get(k).map(|b| b.action);
        assert_eq!(act(&t, "("), Some(Action::PrevSpace));
        assert_eq!(act(&t, "1"), Some(Action::SelectTab(0)));
        assert_eq!(act(&t, "&"), Some(Action::CloseTab));
        assert_eq!(act(&t, "X"), Some(Action::CloseTab));
        assert_eq!(act(&t, "x"), Some(Action::ClosePane));
        assert_eq!(act(&t, "v"), Some(Action::Split(Dir::Horizontal)));
        assert_eq!(act(&t, "h"), Some(Action::Split(Dir::Vertical)));
        assert_eq!(act(&t, "C-f"), Some(Action::SendPrefix));
        assert_eq!(act(&t, "PPage"), Some(Action::ScrollPageUp));
        assert_eq!(act(&t, ";"), None);
        assert_eq!(act(&t, "o"), None);
        assert_eq!(act(&t, "Up"), Some(Action::SelectPane(Side::Up)));
        assert_eq!(act(&t, "z"), Some(Action::Zoom));
        assert_eq!(act(&t, "M-Up"), Some(Action::Resize(Side::Up, 5)));
        assert_eq!(act(&t, "f"), None);
        assert_eq!(act(&t, "r"), None);
        assert_eq!(act(&t, "C-Up"), Some(Action::Resize(Side::Up, 1)));
        assert_eq!(act(&t, "M-Down"), Some(Action::Resize(Side::Down, 5)));
        assert!(t["C-Up"].repeat && !t["X"].repeat);
        let keys = Keys { base_index: 1, table: Some(t), ..Keys::default() };
        assert_eq!(keys.action("f").map(|b| b.action), Some(Action::NewSpace));
        assert!(keys.action("c").is_none());
    }

    #[test]
    fn root_table_acts_without_the_prefix() {
        let list = r##"bind-key  -T root M-Left    resize-pane -L 5
bind-key  -T root C-h       if-shell "ps -o state= -t '#{pane_tty}'" "send-keys 'C-h'" "select-pane -L"
bind-key  -T root MouseDown1Pane select-pane -t = \; send-keys -M
"##;
        let keys = Keys { root: parse_table(list, 0), ..Keys::default() };
        let root = |k: &str| keys.root(k).map(|b| b.action);
        assert_eq!(root("M-Left"), Some(Action::Resize(Side::Left, 5)));
        assert_eq!(root("C-h"), None);
        assert_eq!(root("MouseDown1Pane"), None);
    }

    #[test]
    fn tokens_as_tmux_quotes_them() {
        assert_eq!(tokens(r#"a "b \"c\" d" 'e f' \; \#"#), ["a", r#"b "c" d"#, "e f", ";", "#"]);
    }

    #[test]
    fn vim_tmux_navigator_passes_keys_to_vim() {
        // As `tmux list-keys -T root` prints the plugin's binding
        let list = r##"bind-key  -T root C-h                       if-shell "ps -o state= -o comm= -t '#{pane_tty}'       | grep -iqE '^[^TXZ ]+ +(\\S+/)?g?\\.?(view|l?n?vim?x?|fzf)(diff)?(-wrapped)?$'" "send-keys 'C-h'" "select-pane -L"
"##;
        let keys = Keys { root: parse_table(list, 0), ..Keys::default() };
        let b = keys.root("C-h").unwrap();
        assert_eq!(b.action, Action::SelectPane(Side::Left));
        assert!(b.passes_to(Some("nvim")));
        assert!(b.passes_to(Some("/opt/homebrew/bin/nvim")));
        assert!(b.passes_to(Some("fzf")));
        assert!(!b.passes_to(Some("zsh")));
        assert!(!b.passes_to(None));
    }
}
