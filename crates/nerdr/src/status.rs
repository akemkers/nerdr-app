//! The status the sidebar and the tab bar show: the most urgent among the panes, and how
//! long it has been so. The words are hc's, from the programs' OSC 7501 reports.

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Quiet,
    Working,
    /// Done and not seen yet (`seen.rs`).
    Done,
    Blocked,
}

impl Level {
    /// hc's word: idle, working, done, blocked or error.
    pub fn of(state: &str, unseen: bool) -> Self {
        match state {
            "blocked" | "error" => Self::Blocked,
            "done" if unseen => Self::Done,
            "working" => Self::Working,
            _ => Self::Quiet,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub level: Level,
    /// When the word last changed, ms since the epoch.
    pub since_ms: Option<u64>,
}

/// The most urgent; among equals the one that has been so the longest.
pub fn most_urgent(all: impl IntoIterator<Item = Status>) -> Option<Status> {
    let since = |s: &Status| s.since_ms.unwrap_or(u64::MAX);
    all.into_iter()
        .max_by(|a, b| a.level.cmp(&b.level).then(since(b).cmp(&since(a))))
}

/// How long ago, short: `<1m`, `4m`, `2t`, `3d`.
pub fn age(since_ms: u64, now_ms: u64) -> String {
    let m = now_ms.saturating_sub(since_ms) / 60_000;
    match m {
        0 => "<1m".into(),
        1..60 => format!("{m}m"),
        60..1440 => format!("{}t", m / 60),
        _ => format!("{}d", m / 1440),
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(level: Level, since_ms: Option<u64>) -> Status {
        Status { level, since_ms }
    }

    #[test]
    fn done_counts_only_until_seen() {
        assert_eq!(Level::of("done", true), Level::Done);
        assert_eq!(Level::of("done", false), Level::Quiet);
        assert_eq!(Level::of("error", false), Level::Blocked);
        assert_eq!(Level::of("idle", true), Level::Quiet);
    }

    #[test]
    fn the_most_urgent_wins_and_then_the_oldest() {
        let all = [s(Level::Working, Some(10)), s(Level::Blocked, Some(30)), s(Level::Done, Some(5))];
        assert_eq!(most_urgent(all), Some(s(Level::Blocked, Some(30))));
        let working = [s(Level::Working, Some(20)), s(Level::Working, None), s(Level::Working, Some(10))];
        assert_eq!(most_urgent(working), Some(s(Level::Working, Some(10))));
        assert_eq!(most_urgent([]), None);
    }

    #[test]
    fn ages_are_short() {
        let min = 60_000;
        assert_eq!(age(0, 59_999), "<1m");
        assert_eq!(age(0, 4 * min), "4m");
        assert_eq!(age(0, 150 * min), "2t");
        assert_eq!(age(0, 3 * 1440 * min), "3d");
        // A clock that went back
        assert_eq!(age(10, 0), "<1m");
    }
}
