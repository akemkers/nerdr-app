//! Tasks, their tabs and panes, without GPUI. A task is one piece of work started from a
//! fixed origin (see GLOSSARY.md), a tab is a tree of panes (`panes.rs`), and a pane is
//! one hc terminal. Generic over the pane, so the saved layout (addresses) and the live
//! one (views) are the same model. The workspace drives it and draws it.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::panes::{Dir, Node, Side};

/// A pane knows its terminal's address.
pub trait Addressed {
    fn address(&self) -> &str;
}

impl Addressed for String {
    fn address(&self) -> &str {
        self
    }
}

/// Also the saved form of the layout: the `nerdr` document, with addresses as panes.
/// A document in another shape reads as no tasks.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Tasks<P> {
    #[serde(default)]
    pub tasks: Vec<Task<P>>,
    #[serde(default)]
    pub active: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Task<P> {
    pub id: String,
    /// The folder the task starts from: new tabs start here.
    pub origin: String,
    /// When the origin is a worktree nerdr made: the commit its branch was made from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub tabs: Vec<Tab<P>>,
    #[serde(default)]
    pub active: usize,
    /// The tab active before this one, for tmux's last-window.
    #[serde(skip)]
    last: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Tab<P> {
    /// Stays the same while panes come and go.
    #[serde(skip, default = "tab_id")]
    pub id: u64,
    pub panes: Node<P>,
    /// The active pane's address.
    pub active: String,
    /// The panes active before it, the most recent last: where select-pane goes among
    /// equals, and where the focus goes back to when the active pane closes.
    #[serde(skip)]
    pub recent: Vec<String>,
    /// tmux's zoom: only the active pane shows.
    #[serde(skip)]
    pub zoomed: bool,
}

fn tab_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

fn task_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("s{ms:x}{}", N.fetch_add(1, Ordering::Relaxed))
}

impl<P> Default for Tasks<P> {
    fn default() -> Self {
        Self {
            tasks: Vec::new(),
            active: 0,
        }
    }
}

impl<P: Addressed> Tab<P> {
    pub fn new(pane: P) -> Self {
        Self {
            id: tab_id(),
            active: pane.address().to_string(),
            panes: Node::Leaf(pane),
            recent: Vec::new(),
            zoomed: false,
        }
    }

    pub fn pane(&self, address: &str) -> Option<&P> {
        self.panes.find(&|p: &P| p.address() == address)
    }

    pub fn active_pane(&self) -> Option<&P> {
        self.pane(&self.active)
    }

    fn set_active(&mut self, address: String) {
        let old = std::mem::replace(&mut self.active, address);
        self.recent.retain(|a| *a != old && *a != self.active);
        self.recent.push(old);
    }

    /// The panes `f` keeps or replaces; `None` when none are left.
    fn try_map<Q: Addressed>(self, f: &mut impl FnMut(P) -> Option<Q>) -> Option<Tab<Q>> {
        let panes = self.panes.try_map(f)?;
        let alive = |a: &String| panes.contains(&|p: &Q| p.address() == a);
        let mut recent = self.recent;
        recent.retain(alive);
        let active = if alive(&self.active) {
            self.active
        } else {
            recent.pop().unwrap_or_else(|| panes.leaves()[0].address().to_string())
        };
        Some(Tab {
            id: self.id,
            panes,
            active,
            recent,
            zoomed: self.zoomed,
        })
    }
}

impl<P: Addressed> Task<P> {
    pub fn new(origin: String, base: Option<String>, pane: P) -> Self {
        Self {
            id: task_id(),
            origin,
            base,
            tabs: vec![Tab::new(pane)],
            active: 0,
            last: None,
        }
    }

    pub fn active_tab(&self) -> Option<&Tab<P>> {
        self.tabs.get(self.active)
    }
}

impl<P: Addressed> Tasks<P> {
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn active_task(&self) -> Option<&Task<P>> {
        self.tasks.get(self.active)
    }

    pub fn active_tab(&self) -> Option<&Tab<P>> {
        self.active_task()?.active_tab()
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab<P>> {
        let t = self.tasks.get_mut(self.active)?;
        t.tabs.get_mut(t.active)
    }

    pub fn active_pane(&self) -> Option<&P> {
        self.active_tab()?.active_pane()
    }

    /// The panes on screen: the active tab's, or only its active pane when zoomed.
    pub fn visible_panes(&self) -> Vec<&P> {
        match self.active_tab() {
            Some(t) if t.zoomed => t.active_pane().into_iter().collect(),
            Some(t) => t.panes.leaves(),
            None => Vec::new(),
        }
    }

    pub fn contains(&self, address: &str) -> bool {
        self.find_pane(address).is_some()
    }

    /// Where a pane is: task and tab index.
    pub fn find_pane(&self, address: &str) -> Option<(usize, usize)> {
        self.tasks.iter().enumerate().find_map(|(ti, t)| {
            let i = t.tabs.iter().position(|tab| tab.pane(address).is_some())?;
            Some((ti, i))
        })
    }

    /// Where a new tab in the active task starts.
    pub fn new_tab_dir(&self) -> Option<&str> {
        Some(&self.active_task()?.origin)
    }

    /// The focus went to a pane: it is its tab's active pane. False if it already was.
    pub fn focus(&mut self, address: &str) -> bool {
        let Some((ti, i)) = self.find_pane(address) else {
            return false;
        };
        let tab = &mut self.tasks[ti].tabs[i];
        if tab.active == address {
            return false;
        }
        tab.set_active(address.to_string());
        tab.zoomed = false;
        true
    }

    /// Shows a task, and one of its tabs if given.
    pub fn select(&mut self, task: usize, tab: Option<usize>) {
        let Some(t) = self.tasks.get_mut(task) else {
            return;
        };
        if let Some(i) = tab
            && i < t.tabs.len()
            && i != t.active
        {
            t.last = t.active_tab().map(|t| t.id);
            t.active = i;
        }
        self.active = task;
    }

    pub fn cycle_tab(&mut self, d: isize) {
        if let Some(t) = self.active_task() {
            let n = t.tabs.len() as isize;
            let i = (t.active as isize + d).rem_euclid(n.max(1)) as usize;
            self.select(self.active, Some(i));
        }
    }

    pub fn last_tab(&mut self) {
        let last = self
            .active_task()
            .and_then(|t| t.tabs.iter().position(|tab| Some(tab.id) == t.last));
        if let Some(i) = last {
            self.select(self.active, Some(i));
        }
    }

    pub fn cycle_task(&mut self, d: isize) {
        let n = self.tasks.len() as isize;
        if n > 0 {
            let t = (self.active as isize + d).rem_euclid(n) as usize;
            self.select(t, None);
        }
    }

    /// A new tab in the active task, shown. The pane comes back when there's no task.
    pub fn add_tab(&mut self, pane: P) -> Result<(), P> {
        let Some(t) = self.tasks.get(self.active) else {
            return Err(pane);
        };
        let i = t.tabs.len();
        self.tasks[self.active].tabs.push(Tab::new(pane));
        self.select(self.active, Some(i));
        Ok(())
    }

    /// A new task with one tab, shown. `base` as in `Task`.
    pub fn add_task(&mut self, origin: String, base: Option<String>, pane: P) {
        self.tasks.push(Task::new(origin, base, pane));
        self.select(self.tasks.len() - 1, None);
    }

    /// A tab in the active task, or a task with that origin when there is none.
    pub fn add(&mut self, origin: String, pane: P) {
        if let Err(pane) = self.add_tab(pane) {
            self.add_task(origin, None, pane);
        }
    }

    /// tmux's split-window: `new` takes the second half of the pane at `at` and becomes
    /// active. It comes back when `at` is gone.
    pub fn split(&mut self, at: &str, dir: Dir, new: P) -> Result<(), P> {
        let Some((ti, i)) = self.find_pane(at) else {
            return Err(new);
        };
        let address = new.address().to_string();
        let tab = self.tasks[ti].tabs.remove(i);
        let (panes, rest) = tab.panes.split(&|p: &P| p.address() == at, dir, new);
        let mut tab = Tab { panes, ..tab };
        if rest.is_none() {
            tab.set_active(address);
            tab.zoomed = false;
        }
        self.tasks[ti].tabs.insert(i, tab);
        rest.map_or(Ok(()), Err)
    }

    /// Takes a pane out; its sibling takes the space, and the focus goes back to the
    /// last pane, as in tmux. Then empty tabs and tasks go.
    pub fn remove_pane(&mut self, address: &str) {
        let Some((ti, i)) = self.find_pane(address) else {
            return;
        };
        let id = self.tasks[ti].tabs[i].id;
        *self = std::mem::take(self).try_map(&mut |p: P| (p.address() != address).then_some(p));
        if let Some(tab) = self.tasks.iter_mut().flat_map(|t| &mut t.tabs).find(|tab| tab.id == id) {
            tab.zoomed = false;
        }
    }

    /// The pane on that side of the active one, for select-pane.
    pub fn neighbor(&self, side: Side) -> Option<&P> {
        let t = self.active_tab()?;
        let recent = |p: &P| {
            t.recent
                .iter()
                .position(|a| a == p.address())
                .map_or(0, |i| i + 1)
        };
        t.panes.neighbor(&|p: &P| p.address() == t.active, side, &recent)
    }

    /// tmux's resize-pane -Z. False when the tab has one pane.
    pub fn toggle_zoom(&mut self) -> bool {
        match self.active_tab_mut() {
            Some(t) if matches!(t.panes, Node::Split { .. }) => {
                t.zoomed = !t.zoomed;
                true
            }
            _ => false,
        }
    }

    /// Drops panes whose terminal isn't live any more, then empty tabs and tasks.
    pub fn reconcile(self, live: &HashSet<String>) -> Self {
        self.try_map(&mut |p: P| live.contains(p.address()).then_some(p))
    }

    /// The panes `f` keeps or replaces. Tabs and tasks left empty go; the active ones
    /// stay active while they exist.
    pub fn try_map<Q: Addressed>(self, f: &mut impl FnMut(P) -> Option<Q>) -> Tasks<Q> {
        let active_task = self.active_task().map(|t| t.id.clone());
        let tasks: Vec<Task<Q>> = self
            .tasks
            .into_iter()
            .filter_map(|t| {
                let active_tab = t.active_tab().map(|t| t.id);
                let tabs: Vec<Tab<Q>> = t.tabs.into_iter().filter_map(|tab| tab.try_map(f)).collect();
                if tabs.is_empty() {
                    return None;
                }
                let active = active_tab
                    .and_then(|id| tabs.iter().position(|tab| tab.id == id))
                    .unwrap_or_else(|| t.active.min(tabs.len() - 1));
                Some(Task {
                    id: t.id,
                    origin: t.origin,
                    base: t.base,
                    tabs,
                    active,
                    last: t.last,
                })
            })
            .collect();
        let active = active_task
            .and_then(|id| tasks.iter().position(|t| t.id == id))
            .unwrap_or_else(|| self.active.min(tasks.len().saturating_sub(1)));
        Tasks { tasks, active }
    }

    /// The same tasks with panes made by `f`, as for saving.
    pub fn map<Q>(&self, f: &impl Fn(&P) -> Q) -> Tasks<Q> {
        Tasks {
            tasks: self
                .tasks
                .iter()
                .map(|t| Task {
                    id: t.id.clone(),
                    origin: t.origin.clone(),
                    base: t.base.clone(),
                    tabs: t
                        .tabs
                        .iter()
                        .map(|tab| Tab {
                            id: tab.id,
                            panes: tab.panes.map(f),
                            active: tab.active.clone(),
                            recent: tab.recent.clone(),
                            zoomed: tab.zoomed,
                        })
                        .collect(),
                    active: t.active,
                    last: t.last,
                })
                .collect(),
            active: self.active,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tasks(spec: &[(&str, &[&str], usize)], active: usize) -> Tasks<String> {
        let mut out = Tasks::default();
        for (origin, tabs, a) in spec {
            out.add_task(origin.to_string(), None, tabs[0].to_string());
            for t in &tabs[1..] {
                out.add_tab(t.to_string()).unwrap();
            }
            out.select(out.tasks.len() - 1, Some(*a));
        }
        out.select(active, None);
        out
    }

    /// Each task's origin, its tabs' active panes, and the active indexes.
    fn shape(t: &Tasks<String>) -> (Vec<(String, Vec<String>, usize)>, usize) {
        let tasks = t
            .tasks
            .iter()
            .map(|t| {
                let tabs = t.tabs.iter().map(|tab| tab.active.clone()).collect();
                (t.origin.clone(), tabs, t.active)
            })
            .collect();
        (tasks, t.active)
    }

    fn live(ts: &[&str]) -> HashSet<String> {
        ts.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn new_tabs_start_in_the_origin() {
        let mut t = tasks(&[("/repo", &["t1"], 0)], 0);
        assert_eq!(t.new_tab_dir(), Some("/repo"));
        t.add_task("/other".into(), None, "t2".into());
        assert_eq!(t.new_tab_dir(), Some("/other"));
        // A tab added to a task keeps that task's origin
        t.add_tab("t3".into()).unwrap();
        assert_eq!(shape(&t).0[1], ("/other".into(), vec!["t2".into(), "t3".into()], 1));
    }

    #[test]
    fn add_goes_to_the_active_task_or_a_new_one() {
        let mut t = Tasks::default();
        assert_eq!(t.add_tab("t0".to_string()), Err("t0".to_string()));
        t.add("/a".into(), "t1".to_string());
        t.add("/b".into(), "t2".to_string());
        assert_eq!(shape(&t), (vec![("/a".into(), vec!["t1".into(), "t2".into()], 1)], 0));
        assert!(t.contains("t2") && !t.contains("t3"));
    }

    #[test]
    fn reconcile_keeps_active_tab_and_task() {
        let t = tasks(&[("/a", &["t1", "t2", "t3"], 2), ("/b", &["t4"], 0)], 0);
        let t = t.reconcile(&live(&["t2", "t3", "t4"]));
        let want = (vec![("/a".into(), vec!["t2".into(), "t3".into()], 1), ("/b".into(), vec!["t4".into()], 0)], 0);
        assert_eq!(shape(&t), want);
    }

    #[test]
    fn reconcile_drops_empty_tasks() {
        let t = tasks(&[("/a", &["t1"], 0), ("/b", &["t4"], 0), ("/c", &["t5"], 0)], 2);
        let t = t.reconcile(&live(&["t5"]));
        assert_eq!(shape(&t), (vec![("/c".into(), vec!["t5".into()], 0)], 0));
    }

    #[test]
    fn reconcile_active_tab_gone_falls_back() {
        let t = tasks(&[("/a", &["t1", "t2"], 1)], 0);
        let t = t.reconcile(&live(&["t1"]));
        assert_eq!(shape(&t), (vec![("/a".into(), vec!["t1".into()], 0)], 0));
    }

    #[test]
    fn split_makes_the_new_pane_active_and_remove_goes_back() {
        let mut t = tasks(&[("/a", &["t1"], 0)], 0);
        t.split("t1", Dir::Horizontal, "t2".into()).unwrap();
        assert_eq!(t.active_tab().unwrap().active, "t2");
        assert_eq!(t.split("t9", Dir::Vertical, "t3".into()), Err("t3".into()));
        assert!(t.toggle_zoom());
        assert!(t.focus("t1"));
        assert!(!t.active_tab().unwrap().zoomed);
        assert!(!t.focus("t1"));
        t.split("t1", Dir::Vertical, "t3".into()).unwrap();
        // Closing a pane ends the zoom
        t.toggle_zoom();
        // t3 closes: back to t1, the pane active before it
        t.remove_pane("t3");
        assert_eq!(t.active_tab().unwrap().active, "t1");
        assert!(!t.active_tab().unwrap().zoomed);
        assert_eq!(t.active_tab().unwrap().panes.leaves(), ["t1", "t2"]);
        t.remove_pane("t1");
        t.remove_pane("t2");
        assert!(t.is_empty());
    }

    #[test]
    fn neighbor_moves_within_the_active_tab() {
        let mut t = tasks(&[("/a", &["t1"], 0)], 0);
        t.split("t1", Dir::Horizontal, "t2".into()).unwrap();
        assert_eq!(t.neighbor(Side::Left).map(String::as_str), Some("t1"));
        assert_eq!(t.neighbor(Side::Right), None);
    }

    #[test]
    fn last_tab_goes_back() {
        let mut t = tasks(&[("/a", &["t1", "t2", "t3"], 0)], 0);
        t.select(0, Some(2));
        t.cycle_tab(1);
        assert_eq!(t.active_tab().unwrap().active, "t1");
        t.last_tab();
        assert_eq!(t.active_tab().unwrap().active, "t3");
        t.cycle_task(1);
        assert_eq!(t.active, 0);
    }

    #[test]
    fn saved_form_round_trips_origin_and_splits() {
        let mut t = tasks(&[("/a", &["t1"], 0), ("/b", &["t2"], 0)], 1);
        t.tasks[1].base = Some("abc123".into());
        t.split("t2", Dir::Vertical, "t3".into()).unwrap();
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["tasks"][1]["origin"], "/b");
        assert_eq!(v["tasks"][1]["base"], "abc123");
        assert!(v["tasks"][0].get("base").is_none());
        assert_eq!(v["tasks"][1]["tabs"][0], json!({"panes": {"split": "vertical", "ratio": 0.5, "a": "t2", "b": "t3"}, "active": "t3"}));
        let back: Tasks<String> = serde_json::from_value(v).unwrap();
        assert_eq!(shape(&back), shape(&t));
        assert_eq!(back.tasks[1].base.as_deref(), Some("abc123"));
    }

    #[test]
    fn visible_panes_are_the_active_tabs() {
        let mut t = tasks(&[("/a", &["t1", "t2"], 0)], 0);
        t.split("t1", Dir::Horizontal, "t3".into()).unwrap();
        assert_eq!(t.visible_panes(), ["t1", "t3"]);
        t.toggle_zoom();
        assert_eq!(t.visible_panes(), ["t3"]);
        t.cycle_tab(1);
        assert_eq!(t.visible_panes(), ["t2"]);
    }

    #[test]
    fn a_document_from_before_tasks_reads_as_none() {
        let old = json!({"spaces": [{"id": "x", "tabs": ["t1"]}], "active": 0});
        let t: Tasks<String> = serde_json::from_value(old).unwrap();
        assert!(t.is_empty());
    }
}
