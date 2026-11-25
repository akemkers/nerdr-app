//! The worktrees nerdr makes for new tasks: a branch from fresh `origin/main` in a folder
//! nerdr owns, `<root>/<repo folder name>/<name>`. Wraps `git`. Also the task names: a
//! generated one, and a typed one turned into a valid branch name.

use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};

/// The folder the worktrees go in. Default `$XDG_DATA_HOME/nerdr/worktrees`
/// (`~/.local/share`).
pub const ROOT_VAR: &str = "NERDR_WORKTREES";

pub fn root(home: &Path) -> PathBuf {
    let expand = |p: &str| match p.strip_prefix('~') {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(p),
    };
    match std::env::var(ROOT_VAR) {
        Ok(r) if !r.is_empty() => expand(&r),
        _ => std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("nerdr/worktrees"),
    }
}

/// A worktree made for a task.
#[derive(Debug)]
pub struct Made {
    pub path: PathBuf,
    /// The commit the branch was made from.
    pub base: String,
}

/// Fetches, then adds a worktree for branch `name` under `root`: a new branch from
/// `origin/main`, or the branch with that name if there is one. Blocking.
pub fn create(repo: &Path, root: &Path, name: &str) -> Result<Made> {
    let folder = repo.file_name().context("the repo has no folder name")?;
    let path = root.join(folder).join(name);
    git(repo, &["fetch", "-q", "origin", "main"])?;
    let path_arg = path.to_str().context("the worktree path isn't UTF-8")?;
    if git(repo, &["rev-parse", "-q", "--verify", &format!("refs/heads/{name}")]).is_ok() {
        git(repo, &["worktree", "add", "-q", path_arg, name])?;
    } else {
        // No upstream: origin/main isn't where the branch is pushed to
        git(repo, &["worktree", "add", "-q", "--no-track", "-b", name, path_arg, "origin/main"])?;
    }
    let base = git(&path, &["merge-base", "HEAD", "origin/main"])?;
    Ok(Made { path, base })
}

fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        // A password prompt would hang: nobody sees it
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .output()
        .context("running git")?;
    if !out.status.success() {
        bail!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

const ADJECTIVES: [&str; 32] = [
    "amber", "bold", "brave", "brisk", "calm", "clever", "crisp", "dusky", "eager", "fair", "fancy", "fuzzy",
    "gentle", "glad", "golden", "happy", "jolly", "keen", "lively", "lucky", "mellow", "nimble", "proud", "quick",
    "quiet", "rapid", "shiny", "silent", "steady", "sunny", "swift", "witty",
];
const NOUNS: [&str; 32] = [
    "badger", "beaver", "birch", "brook", "cedar", "comet", "crane", "falcon", "fjord", "fox", "glacier", "harbor",
    "heron", "island", "lynx", "maple", "meadow", "moose", "otter", "owl", "pebble", "pine", "raven", "reef",
    "river", "salmon", "spruce", "stone", "tern", "valley", "walrus", "willow",
];

/// A name for a new task: word-word-hex.
pub fn generate_name() -> String {
    let n = std::collections::hash_map::RandomState::new().build_hasher().finish();
    let adjective = ADJECTIVES[(n % 32) as usize];
    let noun = NOUNS[((n >> 5) % 32) as usize];
    format!("{adjective}-{noun}-{:04x}", (n >> 16) & 0xffff)
}

/// What's typed as a valid branch name: lowercase, `æøå` as `ae o a`, anything but
/// letters, digits, `.`, `_` and `/` as `-`, and none of what git refuses (`..`, a part
/// starting with `.` or ending with `.lock`). None when nothing is left.
pub fn branch_name(typed: &str) -> Option<String> {
    let mut flat = String::new();
    for c in typed.trim().chars().flat_map(char::to_lowercase) {
        match c {
            'æ' => flat.push_str("ae"),
            'ø' => flat.push('o'),
            'å' => flat.push('a'),
            c if c.is_ascii_alphanumeric() || "._/".contains(c) => flat.push(c),
            _ => flat.push('-'),
        }
    }
    let parts: Vec<String> = flat
        .split('/')
        .filter_map(|part| {
            let mut p = part.to_string();
            while p.contains("..") {
                p = p.replace("..", ".");
            }
            while p.contains("--") {
                p = p.replace("--", "-");
            }
            let mut p = p.trim_matches(|c| c == '-' || c == '.').to_string();
            while let Some(rest) = p.strip_suffix(".lock") {
                p = rest.trim_end_matches(['-', '.']).to_string();
            }
            (!p.is_empty()).then_some(p)
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_names_are_word_word_hex_and_valid_branches() {
        for _ in 0..50 {
            let name = generate_name();
            let parts: Vec<&str> = name.split('-').collect();
            assert_eq!(parts.len(), 3, "{name}");
            assert!(ADJECTIVES.contains(&parts[0]) && NOUNS.contains(&parts[1]), "{name}");
            assert!(parts[2].len() == 4 && parts[2].chars().all(|c| c.is_ascii_hexdigit()), "{name}");
            assert_eq!(branch_name(&name).as_deref(), Some(name.as_str()));
        }
        assert_ne!(generate_name(), generate_name());
    }

    #[test]
    fn typed_names_become_branch_names() {
        let cases = [
            ("Fiks login-feil", "fiks-login-feil"),
            ("  rød  knapp på  forsiden ", "rod-knapp-pa-forsiden"),
            ("Ærlig talt", "aerlig-talt"),
            ("feature/Ny side", "feature/ny-side"),
            ("a..b", "a.b"),
            ("-.skjult.", "skjult"),
            ("x.lock", "x"),
            ("dobbel//skråstrek/", "dobbel/skrastrek"),
            ("hva? *[nå]* ~^:\\@{", "hva-na"),
        ];
        for (typed, want) in cases {
            assert_eq!(branch_name(typed).as_deref(), Some(want), "{typed:?}");
        }
        assert_eq!(branch_name(""), None);
        assert_eq!(branch_name(" ?! / "), None);
    }

    #[test]
    fn git_accepts_the_branch_names() {
        for typed in ["Fiks login-feil", "-.a..b.lock/", "@{x}~^:", "x/.y/z.lock.lock", "ÆØÅ/æøå"] {
            let name = branch_name(typed).unwrap();
            let ok = Command::new("git").args(["check-ref-format", "--branch", &name]).output().unwrap();
            assert!(ok.status.success(), "{typed:?} -> {name:?}");
        }
    }

    /// A folder with a bare `origin.git`, a `seed` clone that pushes to it, and `repo`,
    /// cloned from it: the repo a task starts from.
    struct Repos(PathBuf);

    impl Repos {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("nerdr-worktree-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let r = Self(dir);
            r.git(".", &["init", "-q", "--bare", "-b", "main", "origin.git"]);
            r.git(".", &["clone", "-q", "origin.git", "seed"]);
            r.commit("seed", "a.txt", "a\n");
            r.git("seed", &["push", "-q", "origin", "HEAD:main"]);
            r.git(".", &["clone", "-q", "origin.git", "repo"]);
            r
        }

        fn path(&self, dir: &str) -> PathBuf {
            self.0.join(dir)
        }

        fn git(&self, dir: &str, args: &[&str]) -> String {
            // The user's git config can sign commits or run hooks
            let base = ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"];
            let out = Command::new("git").args(base).args(args).current_dir(self.path(dir)).output().unwrap();
            assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim_end().to_string()
        }

        /// The hash of the new commit.
        fn commit(&self, dir: &str, file: &str, text: &str) -> String {
            std::fs::write(self.path(dir).join(file), text).unwrap();
            self.git(dir, &["add", "-A"]);
            self.git(dir, &["commit", "-q", "-m", "c"]);
            self.git(dir, &["rev-parse", "HEAD"])
        }
    }

    impl Drop for Repos {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_new_branch_starts_from_fresh_origin_main() {
        let r = Repos::new("fresh");
        // origin/main moves on after the clone, so only a fetch sees it
        let fresh = r.commit("seed", "b.txt", "b\n");
        r.git("seed", &["push", "-q", "origin", "HEAD:main"]);
        let root = r.path("worktrees");
        let made = create(&r.path("repo"), &root, "swift-otter-1a2b").unwrap();
        assert_eq!(made.path, root.join("repo/swift-otter-1a2b"));
        assert_eq!(made.base, fresh);
        let wt = "worktrees/repo/swift-otter-1a2b";
        assert_eq!(r.git(wt, &["rev-parse", "HEAD"]), fresh);
        assert_eq!(r.git(wt, &["branch", "--show-current"]), "swift-otter-1a2b");
        // No upstream set
        let upstream = Command::new("git").args(["rev-parse", "--abbrev-ref", "@{u}"]).current_dir(r.path(wt)).output().unwrap();
        assert!(!upstream.status.success());
    }

    #[test]
    fn an_existing_branch_is_reused() {
        let r = Repos::new("reuse");
        let main = r.git("repo", &["rev-parse", "HEAD"]);
        r.git("repo", &["checkout", "-q", "-b", "my-work"]);
        let work = r.commit("repo", "w.txt", "w\n");
        r.git("repo", &["checkout", "-q", "main"]);
        let made = create(&r.path("repo"), &r.path("worktrees"), "my-work").unwrap();
        assert_eq!(r.git("worktrees/repo/my-work", &["rev-parse", "HEAD"]), work);
        // Made from the commit it branched off main at
        assert_eq!(made.base, main);
    }

    #[test]
    fn a_repo_without_origin_fails() {
        let r = Repos::new("no-origin");
        r.git("repo", &["remote", "remove", "origin"]);
        assert!(create(&r.path("repo"), &r.path("worktrees"), "x").is_err());
        assert!(!r.path("worktrees/repo/x").exists());
    }
}
