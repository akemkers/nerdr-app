//! Done as news: a pane whose program got done (OSC 7501, from hc) shows it until the pane
//! has been on screen while the window is active.

use std::collections::HashSet;

#[derive(Default)]
pub struct Seen {
    done: HashSet<String>,
    unseen: HashSet<String>,
}

impl Seen {
    /// `done`: every pane that's done now. `visible`: the panes seen right now.
    pub fn update(&mut self, done: HashSet<String>, visible: &HashSet<String>) {
        for a in &done {
            if !self.done.contains(a) && !visible.contains(a) {
                self.unseen.insert(a.clone());
            }
        }
        self.unseen.retain(|a| done.contains(a) && !visible.contains(a));
        self.done = done;
    }

    pub fn is_unseen(&self, address: &str) -> bool {
        self.unseen.contains(address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_is_news_until_seen() {
        let set = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<HashSet<_>>();
        let mut s = Seen::default();
        // t1 gets done behind the scenes, t2 on screen
        s.update(set(&["t1", "t2"]), &set(&["t2"]));
        assert!(s.is_unseen("t1") && !s.is_unseen("t2"));
        // Away from t2 it stays seen
        s.update(set(&["t1", "t2"]), &set(&[]));
        assert!(!s.is_unseen("t2"));
        // Looking at t1
        s.update(set(&["t1", "t2"]), &set(&["t1"]));
        assert!(!s.is_unseen("t1"));
        // t1 works and is done again while away: news again
        s.update(set(&["t2"]), &set(&[]));
        s.update(set(&["t1", "t2"]), &set(&[]));
        assert!(s.is_unseen("t1"));
    }
}
