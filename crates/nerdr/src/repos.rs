//! The git repos a new task can start from: found under the folders in `NERDR_MAPPER`
//! (as in the tmux nerdr, default `~`), fuzzy-searched, with the repos used before first.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The folders searched, colon-separated like `PATH`.
pub const ROOTS_VAR: &str = "NERDR_MAPPER";
/// Built, downloaded or test data, and never a repo you start from.
const SKIP: [&str; 9] = ["node_modules", "target", "build", "dist", "out", "vendor", "__pycache__", "Library", "Applications"];
/// How deep under a root repos are looked for.
const DEPTH: usize = 6;
/// So many used repos are remembered.
const USED: usize = 100;

pub fn roots(home: &Path) -> Vec<PathBuf> {
    let var = std::env::var(ROOTS_VAR).unwrap_or_default();
    let roots: Vec<PathBuf> = var
        .split(':')
        .filter(|r| !r.is_empty())
        .map(|r| match r.strip_prefix('~') {
            Some(rest) => home.join(rest.trim_start_matches('/')),
            None => PathBuf::from(r),
        })
        .collect();
    if roots.is_empty() { vec![home.to_path_buf()] } else { roots }
}

/// Every folder with a `.git` folder under the roots. A worktree (a `.git` file) isn't a
/// repo of its own, hidden folders are skipped, and a repo's own folders aren't searched.
pub fn find(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for r in roots {
        walk(r, 0, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if dir.join(".git").is_dir() {
        out.push(dir.to_path_buf());
        return;
    }
    if depth == DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        // Not following links: a link to a parent would loop
        if name.starts_with('.') || SKIP.contains(&name.as_ref()) || !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        walk(&e.path(), depth + 1, out);
    }
}

/// How `query` matches `text`, case-insensitively as a subsequence: a score (higher is
/// better) and the byte ranges of the matched characters. None when it doesn't match.
pub fn score(query: &str, text: &str) -> Option<(i32, Vec<std::ops::Range<usize>>)> {
    let query: Vec<char> = query.chars().flat_map(char::to_lowercase).filter(|c| !c.is_whitespace()).collect();
    // The last path part counts most, so match there first when it can hold the query
    let base = text.rfind('/').map_or(0, |i| i + 1);
    let (from, in_base) = match matches(&query, text, base) {
        Some(_) => (base, true),
        None => (0, false),
    };
    let hits = matches(&query, text, from)?;
    let mut score = if in_base { 20 } else { 0 };
    let mut prev: Option<usize> = None;
    let mut ranges: Vec<std::ops::Range<usize>> = Vec::new();
    for &(i, c) in &hits {
        let before = text[..i].chars().next_back();
        score += 1;
        if before.is_none_or(|b| "/-_. ".contains(b)) {
            score += 3;
        }
        if prev.is_some_and(|p| text[p..i].chars().count() == 1) {
            score += 3;
        }
        match ranges.last_mut() {
            Some(r) if r.end == i => r.end = i + c.len_utf8(),
            _ => ranges.push(i..i + c.len_utf8()),
        }
        prev = Some(i);
    }
    // The name starts with what's typed
    if in_base && ranges.len() == 1 && ranges[0].start == base {
        score += 10;
    }
    Some((score, ranges))
}

/// The query's characters found in order in `text` from byte `from`: their byte index and
/// character.
fn matches(query: &[char], text: &str, from: usize) -> Option<Vec<(usize, char)>> {
    let mut want = query.iter().peekable();
    let mut hits = Vec::new();
    for (i, c) in text[from..].char_indices() {
        let Some(&&q) = want.peek() else { break };
        if c.to_lowercase().eq(std::iter::once(q)) {
            hits.push((from + i, c));
            want.next();
        }
    }
    want.peek().is_none().then_some(hits)
}

/// The texts matching `query`, best first, with their matched ranges. Among equal scores
/// the most recently used comes first, then the shorter and the alphabetically first.
pub fn rank(query: &str, texts: &[String], used: &[String]) -> Vec<(usize, Vec<std::ops::Range<usize>>)> {
    let recency: HashMap<&str, usize> = used.iter().enumerate().map(|(i, u)| (u.as_str(), i)).collect();
    let mut hits: Vec<(i32, usize, Vec<std::ops::Range<usize>>)> = texts
        .iter()
        .enumerate()
        .filter_map(|(i, t)| score(query, t).map(|(s, r)| (s, i, r)))
        .collect();
    hits.sort_by(|(sa, a, _), (sb, b, _)| {
        let key = |i: &usize| {
            let t = &texts[*i];
            (recency.get(t.as_str()).copied().unwrap_or(usize::MAX), t.len(), t.clone())
        };
        sb.cmp(sa).then_with(|| key(a).cmp(&key(b)))
    });
    hits.into_iter().map(|(_, i, r)| (i, r)).collect()
}

/// Where the used repos are kept, most recent first.
fn used_file(home: &Path) -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"));
    state.join("nerdr/repos")
}

pub fn used(home: &Path) -> Vec<String> {
    std::fs::read_to_string(used_file(home))
        .map(|s| s.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// `repo` was just used: it goes first.
pub fn mark_used(home: &Path, repo: &str) {
    let mut list = used(home);
    list.retain(|r| r != repo);
    list.insert(0, repo.to_string());
    list.truncate(USED);
    let file = used_file(home);
    let wrote = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(&file, list.join("\n") + "\n"));
    if let Err(e) = wrote {
        eprintln!("nerdr: {}: {e}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(ts: &[&str]) -> Vec<String> {
        ts.iter().map(|t| t.to_string()).collect()
    }

    fn ranked(query: &str, ts: &[&str], used: &[&str]) -> Vec<String> {
        let ts = texts(ts);
        rank(query, &ts, &texts(used)).into_iter().map(|(i, _)| ts[i].clone()).collect()
    }

    #[test]
    fn matches_in_order_ignoring_case() {
        assert!(score("NRA", "~/git/nerdr-app").is_some());
        assert!(score("arn", "~/git/nerdr-app").is_none());
        assert_eq!(score("nerdr", "~/git/nerdr-app").unwrap().1, vec![6..11]);
    }

    #[test]
    fn the_last_path_part_and_word_starts_win() {
        let ts = ["~/git/jetdiff/crates/app", "~/git/aksjehandel/api-probroker", "~/apps/x"];
        assert_eq!(ranked("app", &ts, &[])[0], "~/git/jetdiff/crates/app");
        assert_eq!(ranked("apr", &ts, &[])[0], "~/git/aksjehandel/api-probroker");
    }

    #[test]
    fn used_repos_come_first_among_equals() {
        let ts = ["~/git/api-a", "~/git/api-b"];
        assert_eq!(ranked("api", &ts, &[]), ["~/git/api-a", "~/git/api-b"]);
        assert_eq!(ranked("api", &ts, &["~/git/api-b"]), ["~/git/api-b", "~/git/api-a"]);
        // An empty query lists everything, used first
        assert_eq!(ranked("", &ts, &["~/git/api-b"])[0], "~/git/api-b");
    }

    #[test]
    fn find_takes_repos_but_not_worktrees_hidden_or_build_folders() {
        let root = std::env::temp_dir().join(format!("nerdr-repos-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["a/.git", "a/sub/.git", "b/c/.git", ".hidden/d/.git", "node_modules/e/.git", "wt"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("wt/.git"), "gitdir: elsewhere").unwrap();
        let found = find(std::slice::from_ref(&root));
        assert_eq!(found, [root.join("a"), root.join("b/c")]);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
