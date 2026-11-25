//! The window's content: the tasks in a sidebar and the active tab's panes beside it.
//! The model is `tasks.rs`; here it gets terminal views, keys and drawing. The layout is
//! saved in the hc document `nerdr`, so it survives restarts of the app; the terminals
//! themselves live in the core. Keys: `prefix.rs`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use anyhow::{Context as _, Result};
use futures::StreamExt;
use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, KeyDownEvent, Render, Subscription,
    Window, div, prelude::*, px, relative,
};
use jetdiff_ui::RepoView;
use serde_json::{Value, json};

use crate::seen::Seen;
use crate::hc;
use crate::panes::{Dir, Node, Side};
use crate::prefix::{self, Action, Binding, Keys};
use crate::repos;
use crate::tasks::{Addressed, Tab, Task, Tasks};
use crate::theme::{self, hsla};
use crate::view::{Metrics, TermEvent, TerminalView};
use crate::worktree;

pub const DOC: &str = "nerdr";
/// Every terminal nerdr starts carries this label.
pub const LABEL: &str = "nerdr";
const SIDEBAR_W: f32 = 220.;

/// The saved form of the layout: the `nerdr` document.
pub type Layout = Tasks<String>;

/// Our colors and cell size, so the core answers the program's display queries like we look.
pub fn display(m: &Metrics, window: &gpui::Window) -> Value {
    let s = window.scale_factor();
    json!({
        "fg": theme::hex_str(theme::FG),
        "bg": theme::hex_str(theme::BG),
        "cursor": theme::hex_str(theme::CURSOR),
        "cell": [(m.cell_w * s).round() as u32, (m.cell_h * s).round() as u32],
    })
}

/// `terminal.run` params for a new shell in `cwd`.
pub fn shell_params(cwd: &str, cols: u16, rows: u16, display: &Value) -> Value {
    json!({
        "labels": {LABEL: "1"},
        // Not a child of whatever terminal launched us: it should outlive us
        "no_parent": true,
        "no_notify": true,
        "cwd": cwd,
        "size": {"cols": cols, "rows": rows},
        "display": display,
        "env": {"COLORTERM": "truecolor", "TERM_PROGRAM": "nerdr"},
    })
}

pub fn home() -> String {
    std::env::var("HOME").unwrap_or_else(|_| "/".into())
}

/// A `~` path in full.
fn untilde(path: &str) -> String {
    match path.strip_prefix('~') {
        Some(rest) => format!("{}{rest}", home()),
        None => path.to_string(),
    }
}

/// A path with `~` for the home folder.
fn tilde(path: &str) -> String {
    let home = home();
    match path.strip_prefix(&home) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// The folder a terminal started in.
fn start_dir(info: &HashMap<String, Value>, address: &str) -> String {
    info.get(address)
        .and_then(|i| i["cwd"].as_str())
        .map(str::to_string)
        .unwrap_or_else(home)
}

/// The layout to open: the saved one with dead panes dropped, plus `address` if given.
/// Starts a shell when nothing is left. Blocking; runs before the window has content.
/// Returns the layout, the document version, and what the core knows about each live
/// terminal.
pub fn load(
    client: &hc::Client,
    address: Option<&str>,
    cols: u16,
    rows: u16,
    display: &Value,
) -> Result<(Layout, u64, HashMap<String, Value>)> {
    let doc = client.call_blocking("doc.get", json!({"name": DOC}))?;
    let version = doc["version"].as_u64().unwrap_or(0);
    let layout: Layout = match &doc["value"] {
        Value::Null => Layout::default(),
        v => serde_json::from_value(v.clone()).unwrap_or_else(|e| {
            eprintln!("nerdr: ignoring unreadable layout ({e}): {v}");
            Layout::default()
        }),
    };
    let list = client.call_blocking("terminal.list", json!({}))?;
    let mut info: HashMap<String, Value> = list["terminals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| Some((t["address"].as_str()?.to_string(), t.clone())))
        .collect();
    let live: HashSet<String> = info.keys().cloned().collect();
    let mut layout = layout.reconcile(&live);
    if let Some(a) = address {
        let a = resolve(client, a)?;
        if !layout.contains(&a) {
            layout.add(start_dir(&info, &a), a);
        }
    }
    if layout.is_empty() {
        let run = client.call_blocking("terminal.run", shell_params(&home(), cols, rows, display))?;
        let a = run["address"].as_str().context("terminal.run: no address")?;
        eprintln!("nerdr: started {a}");
        info.insert(a.to_string(), json!({"address": a, "cwd": home()}));
        layout.add_task(home(), None, a.to_string());
    }
    Ok((layout, version, info))
}

/// A name or address to the address.
fn resolve(client: &hc::Client, t: &str) -> Result<String> {
    let r = client.call_blocking("terminal.get", json!({"terminal": t}))?;
    let t = r.get("terminal").unwrap_or(&r);
    Ok(t["address"]
        .as_str()
        .context("terminal.get: no address")?
        .to_string())
}

struct Pane {
    address: String,
    view: Entity<TerminalView>,
    _subs: Vec<Subscription>,
}

impl Addressed for Pane {
    fn address(&self) -> &str {
        &self.address
    }
}

/// The new-task popup: what's typed, and the hit picked with the arrows.
#[derive(Default)]
struct Picker {
    query: String,
    selected: usize,
    /// The repos used before, most recent first, as `~` paths.
    used: Vec<String>,
    /// The name step, once a repo is picked.
    naming: Option<Naming>,
}

struct Naming {
    /// The picked repo, a `~` path.
    repo: String,
    name: String,
    /// The name is the generated one, untouched: typing replaces it.
    generated: bool,
    /// Start in the repo itself, not in a new worktree.
    in_repo: bool,
    /// The worktree is being made.
    busy: bool,
    /// What went wrong.
    error: Option<String>,
}

/// The popup's name step: the repo, the name, and new worktree or in the repo.
fn render_naming(n: &Naming) -> gpui::Div {
    let dim = hsla(theme::MUTED);
    let row = |label: &'static str| {
        div()
            .flex()
            .flex_row()
            .gap(px(8.))
            .px(px(8.))
            .child(div().w(px(56.)).text_color(dim).child(label))
    };
    // One text with the cursor, so the row's gap doesn't come between them
    let cursor = if n.busy { "" } else { "▏" };
    let selected = gpui::HighlightStyle {
        background_color: Some(hsla(theme::SIDEBAR_SELECTED)),
        ..Default::default()
    };
    let name = gpui::StyledText::new(format!("{}{cursor}", n.name))
        .with_highlights(n.generated.then(|| (0..n.name.len(), selected)));
    let mode = |label: &'static str, on: bool| div().when(!on, |d| d.text_color(dim)).child(label);
    let note = if n.busy {
        Some(("lager worktree …".to_string(), dim))
    } else if let Some(e) = &n.error {
        Some((e.clone(), hsla(theme::RED)))
    } else {
        // What the typed name becomes
        worktree::branch_name(&n.name)
            .filter(|b| *b != n.name && !n.in_repo)
            .map(|b| (format!("blir {b}"), dim))
    };
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(row("repo").child(n.repo.clone()))
        .child(
            row("start")
                .child(mode("worktree", !n.in_repo))
                .child(div().text_color(dim).child("|"))
                .child(mode("start i main", n.in_repo))
                .child(div().text_color(dim).child("(tab)")),
        )
        .when(!n.in_repo, |d| d.child(row("branch").child(div().whitespace_nowrap().child(name))))
        .children(note.map(|(t, c)| div().px(px(8.)).text_color(c).child(t)))
}

/// So many hits show in the popup.
const PICKER_ROWS: usize = 12;

pub struct Workspace {
    client: hc::Client,
    metrics: Metrics,
    display: Value,
    tasks: Tasks<Pane>,
    /// What the core says about each terminal (from the event feed): title, cwd, status.
    info: HashMap<String, Value>,
    doc_version: u64,
    saving: bool,
    save_again: bool,
    focus: FocusHandle,
    window_title: Rc<RefCell<String>>,
    keys: Keys,
    /// The prefix was pressed; the next key is a command.
    prefix_armed: bool,
    /// Until then a repeatable key works without the prefix.
    repeat_until: Option<Instant>,
    /// A line between panes being dragged: its tab and the path to its split.
    drag: Option<(u64, Vec<bool>)>,
    /// The scale `metrics` and `display` were made for.
    scale: f32,
    /// The popup for a new task, while it's open.
    picker: Option<Picker>,
    /// The repos a task can start from, as `~` paths, from the last search.
    repos: Vec<String>,
    searching: bool,
    /// The window has the system's focus: what shows in it is seen.
    window_active: bool,
    seen: Seen,
    /// Each task's diff overlay by task id, made the first time it opens.
    diffs: HashMap<String, Entity<RepoView>>,
    /// A message over the panes until the next key.
    notice: Option<String>,
    _tasks: Vec<gpui::Task<()>>,
    _subs: Vec<Subscription>,
}

impl Workspace {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: hc::Client,
        layout: Layout,
        doc_version: u64,
        info: HashMap<String, Value>,
        metrics: Metrics,
        display: Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut notices = client.notices();
        let feed = cx.spawn(async move |this, cx| {
            while let Some(n) = notices.next().await {
                if this.update(cx, |this, cx| this.on_notice(n, cx)).is_err() {
                    break;
                }
            }
        });
        let mut this = Self {
            client,
            metrics,
            display,
            tasks: Tasks::default(),
            info,
            doc_version,
            saving: false,
            save_again: false,
            focus: cx.focus_handle(),
            window_title: Rc::default(),
            keys: Keys::load(),
            prefix_armed: false,
            repeat_until: None,
            drag: None,
            scale: window.scale_factor(),
            picker: None,
            repos: Vec::new(),
            searching: false,
            window_active: window.is_window_active(),
            seen: Seen::default(),
            diffs: HashMap::new(),
            notice: None,
            _tasks: vec![feed],
            _subs: vec![
                cx.observe_window_bounds(window, Self::on_bounds),
                cx.observe_window_activation(window, |this, window, cx| {
                    this.window_active = window.is_window_active();
                    this.track_seen();
                    cx.notify();
                }),
            ],
        };
        this.tasks = layout.try_map(&mut |a: String| Some(this.open_pane(a, window, cx)));
        this.save(cx);
        this
    }

    /// A new scale (another display, or the first real one) changes the cell size in
    /// pixels. The core only takes `display` on attach, so every pane attaches again.
    fn on_bounds(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scale = window.scale_factor();
        if scale == self.scale {
            return;
        }
        self.scale = scale;
        self.metrics = Metrics::new(f32::from(self.metrics.font_size), window);
        self.display = display(&self.metrics, window);
        let tasks = std::mem::take(&mut self.tasks);
        self.tasks = tasks.try_map(&mut |p: Pane| Some(self.open_pane(p.address, window, cx)));
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Writes the layout to the `nerdr` document. One write at a time; changes made
    /// meanwhile are written after it. Another writer wins nothing: on a version conflict
    /// we read the current version and write ours over it.
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving {
            self.save_again = true;
            return;
        }
        self.saving = true;
        let client = self.client.clone();
        let value = serde_json::to_value(self.tasks.map(&|p: &Pane| p.address.clone())).unwrap();
        let version = self.doc_version;
        cx.spawn(async move |this, cx| {
            let params = |v: u64| json!({"name": DOC, "value": value, "version": v});
            let mut res = client.call("doc.set", params(version)).await;
            if res.is_err() {
                // Stale version (another nerdr, or a reconnect): take the current one
                if let Ok(cur) = client.call("doc.get", json!({"name": DOC})).await {
                    let v = cur["version"].as_u64().unwrap_or(0);
                    res = client.call("doc.set", params(v)).await;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match res {
                    Ok(r) => this.doc_version = r["version"].as_u64().unwrap_or(this.doc_version),
                    Err(e) => eprintln!("nerdr: saving layout: {e:#}"),
                }
                if std::mem::take(&mut this.save_again) {
                    this.save(cx);
                }
            });
        })
        .detach();
    }

    fn open_pane(&mut self, address: String, window: &mut Window, cx: &mut Context<Self>) -> Pane {
        let (cols, rows) = self.grid_size(window);
        let params = json!({"terminal": address, "display": self.display, "history": 5000});
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let window_title = self.window_title.clone();
        let view = cx.new(|cx| {
            TerminalView::new(&client, params, cols, rows, metrics, window_title, window, cx)
        });
        let a = address.clone();
        let exited = cx.subscribe_in(&view, window, move |this, _, ev, window, cx| match ev {
            TermEvent::Exited => this.on_exited(&a, window, cx),
        });
        let changed = cx.observe(&view, |_, _, cx| cx.notify());
        let a = address.clone();
        // Focus listeners run at the end of a draw, where a notify doesn't bring a new
        // frame, so the border would wait for the next key. Deferred, it runs after.
        let focused = cx.on_focus_in(&view.focus_handle(cx), window, move |_, window, cx| {
            let a = a.clone();
            cx.defer_in(window, move |this, _, cx| this.on_pane_focused(&a, cx));
        });
        Pane {
            address,
            view,
            _subs: vec![exited, changed, focused],
        }
    }

    /// Cells for a terminal filling the area beside the sidebar.
    fn grid_size(&self, window: &Window) -> (u16, u16) {
        let vp = window.viewport_size();
        self.metrics.grid_size(
            f32::from(vp.width) - SIDEBAR_W,
            f32::from(vp.height),
        )
    }

    pub fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(d) = self.shown_diff() {
            return window.focus(&d.focus_handle(cx));
        }
        match self.tasks.active_pane() {
            Some(p) => window.focus(&p.view.focus_handle(cx)),
            None => window.focus(&self.focus),
        }
    }

    /// A click (or focus_active) put the focus in a pane: it is the active one.
    fn on_pane_focused(&mut self, address: &str, cx: &mut Context<Self>) {
        if self.tasks.focus(address) {
            self.save(cx);
            self.track_seen();
            cx.notify();
        }
    }

    /// What's on screen now has been seen, so a done pane there isn't news.
    fn track_seen(&mut self) {
        let visible: HashSet<String> = if self.window_active {
            self.tasks.visible_panes().into_iter().map(|p| p.address.clone()).collect()
        } else {
            HashSet::new()
        };
        let done = self
            .tasks
            .tasks
            .iter()
            .flat_map(|t| &t.tabs)
            .flat_map(|tab| tab.panes.leaves())
            .filter(|p| self.state(&p.address) == Some("done"))
            .map(|p| p.address.clone())
            .collect();
        self.seen.update(done, &visible);
    }

    /// What a pane's program reports with OSC 7501, as hc has it: idle, working, done,
    /// blocked or error.
    fn state(&self, address: &str) -> Option<&str> {
        self.info.get(address)?["status"]["state"].as_str()
    }

    fn select_pane(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        // Focusing it does the rest
        if let Some(p) = self.tasks.neighbor(side) {
            window.focus(&p.view.focus_handle(cx));
        }
    }

    /// Where panes go: beside the sidebar, the whole height.
    fn pane_area(&self, window: &Window) -> (f32, f32) {
        let vp = window.viewport_size();
        (f32::from(vp.width) - SIDEBAR_W, f32::from(vp.height))
    }

    fn resize_pane(&mut self, side: Side, cells: u16, window: &mut Window, cx: &mut Context<Self>) {
        let area = self.pane_area(window);
        let cell = match side {
            Side::Left | Side::Right => self.metrics.cell_w,
            Side::Up | Side::Down => self.metrics.cell_h,
        };
        let Some(t) = self.tasks.active_tab_mut() else {
            return;
        };
        let active = t.active.clone();
        t.zoomed = false;
        if t.panes.resize(&|p: &Pane| p.address == active, side, cells as f32 * cell, area) {
            self.save(cx);
            cx.notify();
        }
    }

    /// The dragged line follows the mouse.
    fn on_drag(&mut self, ev: &gpui::MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, path)) = self.drag.clone() else {
            return;
        };
        if ev.pressed_button != Some(gpui::MouseButton::Left) {
            self.end_drag(cx);
            return;
        }
        let (w, h) = self.pane_area(window);
        let (fx, fy) = ((f32::from(ev.position.x) - SIDEBAR_W) / w, f32::from(ev.position.y) / h);
        let Some(t) = self.tasks.active_tab_mut().filter(|t| t.id == id) else {
            return;
        };
        let Some((dir, r)) = t.panes.split_at(&path) else {
            return;
        };
        let ratio = match dir {
            Dir::Horizontal => (fx - r.x) / r.w,
            Dir::Vertical => (fy - r.y) / r.h,
        };
        t.panes.set_ratio(&path, ratio);
        cx.notify();
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        if self.drag.take().is_some() {
            self.save(cx);
        }
    }

    fn toggle_zoom(&mut self, cx: &mut Context<Self>) {
        if self.tasks.toggle_zoom() {
            cx.notify();
        }
    }

    /// After a change to which task or tab shows.
    fn shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_active(window, cx);
        self.save(cx);
        self.track_seen();
        cx.notify();
    }

    fn select(&mut self, task: usize, tab: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.tasks.select(task, tab);
        self.shown(window, cx);
    }

    /// The active task's diff overlay, when it covers the tabs.
    fn shown_diff(&self) -> Option<&Entity<RepoView>> {
        let t = self.tasks.active_task().filter(|t| t.diff)?;
        self.diffs.get(&t.id)
    }

    /// After the diff overlay came or went. Not saved: it lives while the app runs.
    fn diff_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_active(window, cx);
        self.track_seen();
        cx.notify();
    }

    /// prefix+d: the active task's diff overlay, read from its origin the first time.
    fn toggle_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(t) = self.tasks.active_task() else {
            return;
        };
        if t.diff || self.diffs.contains_key(&t.id) {
            self.tasks.toggle_diff();
            return self.diff_changed(window, cx);
        }
        if !cx.has_global::<jetdiff_ui::Settings>() {
            self.notice = Some("jetdiff-configen kunne ikke leses, se feilen i terminalen".into());
            return cx.notify();
        }
        let id = t.id.clone();
        let origin = t.origin.clone();
        // On this thread: jetdiff's RepoSource can't be sent from another. Only the first
        // open reads; jetdiff runs its later reads in the background itself.
        match jetdiff_app::RepoSource::open(std::path::Path::new(&origin)) {
            Ok((source, commits)) => {
                let view = cx.new(|cx| RepoView::new(source, Some(commits), cx));
                self.diffs.insert(id, view);
                self.tasks.toggle_diff();
            }
            Err(e) => self.notice = Some(format!("ingen diff for {}: {e}", tilde(&origin))),
        }
        self.diff_changed(window, cx);
    }

    /// Back to the active task's tabs.
    fn hide_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tasks.hide_diff() {
            self.diff_changed(window, cx);
        }
    }

    /// Where a pane's program is, for panes split from it.
    fn cwd_of(&self, address: &str) -> String {
        self.info
            .get(address)
            .and_then(|i| i["foreground"]["cwd"].as_str().or(i["cwd"].as_str()))
            .map(str::to_string)
            .unwrap_or_else(home)
    }

    /// Starts a shell and opens it as a tab in the active task's origin.
    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = self.tasks.new_tab_dir().map_or_else(home, str::to_string);
        let (cols, rows) = self.grid_size(window);
        let run = self
            .client
            .call("terminal.run", shell_params(&cwd, cols, rows, &self.display));
        cx.spawn_in(window, async move |this, cx| {
            let address = match run.await {
                Ok(r) => r["address"].as_str().map(str::to_string),
                Err(e) => {
                    eprintln!("nerdr: new tab: {e:#}");
                    None
                }
            };
            let Some(address) = address else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                let pane = this.open_pane(address, window, cx);
                this.tasks.add(cwd, pane);
                this.shown(window, cx);
            });
        })
        .detach();
    }

    /// tmux's split-window: a shell in the active pane's directory takes half of it and
    /// becomes the active pane.
    fn split(&mut self, dir: Dir, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.tasks.active_pane() else {
            return;
        };
        let at = pane.address.clone();
        let cwd = self.cwd_of(&at);
        let (cols, rows) = pane.view.read(cx).grid();
        let (cols, rows) = match dir {
            Dir::Horizontal => (cols / 2, rows),
            Dir::Vertical => (cols, rows / 2),
        };
        let run = self
            .client
            .call("terminal.run", shell_params(&cwd, cols, rows, &self.display));
        cx.spawn_in(window, async move |this, cx| {
            let address = match run.await {
                Ok(r) => r["address"].as_str().map(str::to_string),
                Err(e) => {
                    eprintln!("nerdr: split: {e:#}");
                    None
                }
            };
            let Some(address) = address else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                let new = this.open_pane(address, window, cx);
                if this.tasks.split(&at, dir, new).is_ok() {
                    this.shown(window, cx);
                }
            });
        })
        .detach();
    }

    /// Ends a terminal's program and lets the core forget it.
    fn kill(&self, address: String, cx: &mut Context<Self>) {
        let client = self.client.clone();
        cx.spawn(async move |_, _| {
            let t = json!({"terminal": address});
            let _ = client.call("terminal.kill", t.clone()).await;
            let _ = client.call("terminal.rm", t).await;
        })
        .detach();
    }

    /// tmux's kill-pane: the last pane takes its tab with it.
    fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(address) = self.tasks.active_pane().map(|p| p.address.clone()) else {
            return;
        };
        self.remove_pane(&address, window, cx);
        self.kill(address, cx);
    }

    /// tmux's kill-window: the tab and every pane in it.
    fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tasks.active_tab() else {
            return;
        };
        let addresses: Vec<String> = tab.panes.leaves().iter().map(|p| p.address.clone()).collect();
        for a in addresses {
            self.remove_pane(&a, window, cx);
            self.kill(a, cx);
        }
    }

    /// The terminal's program ended: close its pane and let the core forget it.
    fn on_exited(&mut self, address: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.remove_pane(address, window, cx);
        let rm = self.client.call("terminal.rm", json!({"terminal": address}));
        cx.spawn(async move |_, _| {
            let _ = rm.await;
        })
        .detach();
    }

    /// Drops the pane (detaching its stream). The last pane of all closes the app.
    fn remove_pane(&mut self, address: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tasks.contains(address) {
            return;
        }
        self.tasks.remove_pane(address);
        let tasks = &self.tasks.tasks;
        self.diffs.retain(|id, _| tasks.iter().any(|t| t.id == *id));
        self.save(cx);
        if self.tasks.is_empty() {
            // Let the save go out before quitting
            cx.spawn(async move |_, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(200))
                    .await;
                let _ = cx.update(|cx| cx.quit());
            })
            .detach();
            return;
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    /// prefix+f: the popup takes the keys, and the repos are searched again meanwhile.
    fn open_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let used = repos::used(std::path::Path::new(&home())).iter().map(|u| tilde(u)).collect();
        self.picker = Some(Picker {
            used,
            ..Default::default()
        });
        window.focus(&self.focus);
        cx.notify();
        if self.searching {
            return;
        }
        self.searching = true;
        let roots = repos::roots(std::path::Path::new(&home()));
        let find = cx.background_spawn(async move { repos::find(&roots) });
        cx.spawn(async move |this, cx| {
            let found = find.await;
            let _ = this.update(cx, |this, cx| {
                this.repos = found.iter().map(|p| tilde(&p.to_string_lossy())).collect();
                this.searching = false;
                cx.notify();
            });
        })
        .detach();
    }

    fn close_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker = None;
        self.focus_active(window, cx);
        cx.notify();
    }

    /// The repos matching what's typed, best first, with the matched byte ranges.
    fn picker_hits(&self) -> Vec<(usize, Vec<std::ops::Range<usize>>)> {
        let Some(p) = &self.picker else {
            return Vec::new();
        };
        repos::rank(&p.query, &self.repos, &p.used)
    }

    fn picker_key(&mut self, k: &gpui::Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.as_ref().is_some_and(|p| p.naming.is_some()) {
            return self.naming_key(k, window, cx);
        }
        let hits = self.picker_hits();
        let Some(p) = &mut self.picker else {
            return;
        };
        let m = &k.modifiers;
        match (k.key.as_str(), m.control) {
            ("escape", _) => return self.close_picker(window, cx),
            ("enter", _) => {
                if let Some((i, _)) = hits.get(p.selected) {
                    let repo = self.repos[*i].clone();
                    self.pick_repo(repo, cx);
                }
                return;
            }
            ("up", _) | ("p" | "k", true) => p.selected = p.selected.saturating_sub(1),
            ("down", _) | ("n" | "j", true) => p.selected = (p.selected + 1).min(hits.len().saturating_sub(1)),
            ("backspace", _) => {
                p.query.pop();
                p.selected = 0;
            }
            _ => match &k.key_char {
                Some(t) if !m.control && !m.platform && !t.chars().any(char::is_control) => {
                    p.query.push_str(t);
                    p.selected = 0;
                }
                _ => {}
            },
        }
        cx.notify();
    }

    /// On to the name step, with a generated name.
    fn pick_repo(&mut self, repo: String, cx: &mut Context<Self>) {
        if let Some(p) = &mut self.picker {
            p.naming = Some(Naming {
                repo,
                name: worktree::generate_name(),
                generated: true,
                in_repo: false,
                busy: false,
                error: None,
            });
        }
        cx.notify();
    }

    fn naming_key(&mut self, k: &gpui::Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = &mut self.picker else {
            return;
        };
        let Some(n) = &mut p.naming else {
            return;
        };
        let m = &k.modifiers;
        match k.key.as_str() {
            // The worktree is still made, and its task started
            "escape" if n.busy => return self.close_picker(window, cx),
            "escape" => p.naming = None,
            _ if n.busy => return,
            "tab" => n.in_repo = !n.in_repo,
            "enter" if n.in_repo => {
                let repo = untilde(&n.repo);
                self.picker = None;
                return self.start_task(repo.clone(), repo, None, window, cx);
            }
            "enter" => match worktree::branch_name(&n.name) {
                Some(branch) => return self.make_worktree(branch, window, cx),
                None => n.error = Some("navnet blir ikke et gyldig branch-navn".into()),
            },
            "backspace" => {
                if std::mem::take(&mut n.generated) {
                    n.name.clear();
                } else {
                    n.name.pop();
                }
            }
            _ => match &k.key_char {
                Some(t) if !m.control && !m.platform && !t.chars().any(char::is_control) => {
                    if std::mem::take(&mut n.generated) {
                        n.name.clear();
                    }
                    n.name.push_str(t);
                    n.error = None;
                }
                _ => {}
            },
        }
        cx.notify();
    }

    /// Makes the worktree in the background, then starts the task in it. The popup shows
    /// what went wrong.
    fn make_worktree(&mut self, branch: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(n) = self.picker.as_mut().and_then(|p| p.naming.as_mut()) else {
            return;
        };
        n.busy = true;
        n.error = None;
        let repo = untilde(&n.repo);
        let root = worktree::root(std::path::Path::new(&home()));
        let from = std::path::PathBuf::from(&repo);
        let make = cx.background_spawn(async move { worktree::create(&from, &root, &branch) });
        cx.spawn_in(window, async move |this, cx| {
            let made = make.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let waiting = this.picker.as_mut().and_then(|p| p.naming.as_mut()).filter(|n| n.busy);
                match made {
                    Ok(made) => {
                        if waiting.is_some() {
                            this.picker = None;
                        }
                        let origin = made.path.to_string_lossy().into_owned();
                        this.start_task(origin, repo, Some(made.base), window, cx);
                    }
                    Err(e) => match waiting {
                        Some(n) => {
                            n.busy = false;
                            n.error = Some(format!("{e:#}"));
                        }
                        None => eprintln!("nerdr: new worktree: {e:#}"),
                    },
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// A new task in `origin` with the agent in its first pane. `repo` is the repo it's
    /// from, `base` as in `Task`.
    fn start_task(&mut self, origin: String, repo: String, base: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        repos::mark_used(std::path::Path::new(&home()), &repo);
        let (cols, rows) = self.grid_size(window);
        let mut params = shell_params(&origin, cols, rows, &self.display);
        params["argv"] = json!(["claude"]);
        let run = self.client.call("terminal.run", params);
        cx.spawn_in(window, async move |this, cx| {
            let address = match run.await {
                Ok(r) => r["address"].as_str().map(str::to_string),
                Err(e) => {
                    eprintln!("nerdr: new task: {e:#}");
                    None
                }
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if let Some(address) = address {
                    let pane = this.open_pane(address, window, cx);
                    this.tasks.add_task(origin, base, pane);
                }
                this.shown(window, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn render_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.picker.as_ref()?;
        if let Some(n) = &p.naming {
            return Some(self.popup(render_naming(n)));
        }
        let dim = hsla(theme::MUTED);
        let hits = self.picker_hits();
        // A window of rows that keeps the selected one in view
        let first = p.selected.saturating_sub(PICKER_ROWS - 1);
        let mut list = div().flex().flex_col();
        for (row, (i, ranges)) in hits.iter().enumerate().skip(first).take(PICKER_ROWS) {
            let text = self.repos[*i].clone();
            let lit = gpui::HighlightStyle {
                color: Some(hsla(theme::YELLOW)),
                ..Default::default()
            };
            let label = gpui::StyledText::new(text.clone()).with_highlights(ranges.iter().map(|r| (r.clone(), lit)));
            list = list.child(
                div()
                    .id(("repo", row))
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .when(row == p.selected, |d| d.bg(hsla(theme::SIDEBAR_SELECTED)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.pick_repo(text.clone(), cx))),
            );
        }
        let note = if self.searching {
            Some("leter etter repoer …")
        } else if hits.is_empty() {
            Some("ingen repoer")
        } else {
            None
        };
        let input = div()
            .flex()
            .flex_row()
            .gap(px(8.))
            .px(px(8.))
            .pb(px(6.))
            .child(div().text_color(dim).child("ny oppgave"))
            .child(div().child(format!("{}▏", p.query)));
        let body = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(input)
            .child(list)
            .children(note.map(|n| div().px(px(8.)).text_color(dim).child(n)));
        Some(self.popup(body))
    }

    fn render_notice(&self) -> Option<AnyElement> {
        let n = self.notice.as_ref()?;
        let hint = div().text_color(hsla(theme::MUTED)).child("en tast lukker");
        Some(self.popup(div().px(px(8.)).flex().flex_col().gap(px(4.)).child(n.clone()).child(hint)))
    }

    /// A panel over the panes, near the top.
    fn popup(&self, body: gpui::Div) -> AnyElement {
        let panel = div()
            .occlude()
            .w(px(640.))
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(2.))
            .bg(hsla(theme::SIDEBAR_BG))
            .border_1()
            .border_color(hsla(theme::PANE_BORDER))
            .rounded(px(6.))
            .font_family(self.metrics.font.family.clone())
            .text_size(px(14.))
            .text_color(hsla(theme::FG))
            .child(body);
        div()
            .absolute()
            .top(px(60.))
            .left_0()
            .right_0()
            .flex()
            .justify_center()
            .child(panel)
            .into_any_element()
    }

    fn on_notice(&mut self, n: hc::Notice, cx: &mut Context<Self>) {
        match n {
            hc::Notice::Snapshot { terminals, .. } => {
                self.info = terminals
                    .into_iter()
                    .filter_map(|t| Some((t["address"].as_str()?.to_string(), t)))
                    .collect();
            }
            hc::Notice::Event(e) => {
                let Some(addr) = e["terminal"].as_str() else {
                    return;
                };
                let data = &e["data"];
                match e["type"].as_str().unwrap_or("") {
                    "created" => {
                        self.info.insert(addr.to_string(), data.clone());
                    }
                    "removed" => {
                        self.info.remove(addr);
                    }
                    ty => {
                        let Some(i) = self.info.get_mut(addr) else {
                            return;
                        };
                        match ty {
                            "title" => i["title"] = data["title"].clone(),
                            "foreground" => i["foreground"] = data.clone(),
                            // The record itself, or wrapped
                            "status" => {
                                i["status"] = if data.get("state").is_some() {
                                    data.clone()
                                } else {
                                    data["status"].clone()
                                }
                            }
                            "finished" => i["state"] = "finished".into(),
                            _ => return,
                        }
                    }
                }
            }
            hc::Notice::Connected { .. } | hc::Notice::Disconnected(_) => {}
        }
        self.track_seen();
        cx.notify();
    }

    /// tmux-style: the prefix arms, the next key is the command. After a repeatable one
    /// (`bind -r`) more of them work without the prefix for `repeat-time`. Root-table keys
    /// (`bind -n`) act straight away. Every other key goes to the terminal.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        // A lone modifier (shift for `&`) doesn't end a wait
        if matches!(k.key.as_str(), "shift" | "control" | "alt" | "platform" | "function") {
            return;
        }
        if self.picker.is_some() {
            cx.stop_propagation();
            self.picker_key(k, window, cx);
            return;
        }
        if self.notice.take().is_some() {
            cx.stop_propagation();
            cx.notify();
            return;
        }
        let name = prefix::key_name(k);
        // In the diff overlay only the prefix is ours; the rest are jetdiff's
        if !self.prefix_armed && self.shown_diff().is_some() {
            if self.keys.prefix.matches(k) {
                self.prefix_armed = true;
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        if !self.prefix_armed {
            let repeating = self.repeat_until.take().is_some_and(|t| Instant::now() < t);
            let binding = self.keys.action(&name).filter(|b| b.repeat && repeating);
            if let Some(b) = binding {
                cx.stop_propagation();
                self.run_binding(b, window, cx);
            } else if self.keys.prefix.matches(k) {
                self.prefix_armed = true;
                cx.stop_propagation();
                cx.notify();
            } else if let Some(b) = self.keys.root(&name) {
                let program = self
                    .tasks
                    .active_pane()
                    .and_then(|p| self.info.get(&p.address)?["foreground"]["name"].as_str());
                if !b.passes_to(program) {
                    let action = b.action;
                    cx.stop_propagation();
                    self.run(action, window, cx);
                }
            }
            return;
        }
        self.prefix_armed = false;
        cx.stop_propagation();
        cx.notify();
        if self.keys.prefix.matches(k) {
            self.run(Action::SendPrefix, window, cx);
        } else if let Some(b) = self.keys.action(&name) {
            self.run_binding(b, window, cx);
        }
    }

    fn run_binding(&mut self, b: Binding, window: &mut Window, cx: &mut Context<Self>) {
        if b.repeat {
            self.repeat_until = Some(Instant::now() + self.keys.repeat_time);
        }
        self.run(b.action, window, cx);
    }

    fn run(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if action.on_terminals() {
            self.hide_diff(window, cx);
        }
        match action {
            Action::NewTab => self.new_tab(window, cx),
            Action::NewTask => self.open_picker(window, cx),
            Action::NextTab => {
                self.tasks.cycle_tab(1);
                self.shown(window, cx)
            }
            Action::PrevTab => {
                self.tasks.cycle_tab(-1);
                self.shown(window, cx)
            }
            Action::SelectTab(t) => self.select(self.tasks.active, Some(t), window, cx),
            Action::LastTab => {
                self.tasks.last_tab();
                self.shown(window, cx)
            }
            Action::CloseTab => self.close_tab(window, cx),
            Action::NextTask => {
                self.tasks.cycle_task(1);
                self.shown(window, cx)
            }
            Action::PrevTask => {
                self.tasks.cycle_task(-1);
                self.shown(window, cx)
            }
            Action::Split(dir) => self.split(dir, window, cx),
            Action::ClosePane => self.close_pane(window, cx),
            Action::SelectPane(side) => self.select_pane(side, window, cx),
            Action::Zoom => self.toggle_zoom(cx),
            Action::Resize(side, cells) => self.resize_pane(side, cells, window, cx),
            Action::ScrollPageUp => {
                if let Some(p) = self.tasks.active_pane() {
                    p.view.update(cx, |v, cx| v.scroll_pages(1, cx));
                }
            }
            Action::SendPrefix => {
                let ks = self.keys.prefix.keystroke();
                if let Some(p) = self.tasks.active_pane() {
                    p.view.update(cx, |v, cx| v.type_keystroke(&ks, cx));
                }
            }
            Action::Diff => self.toggle_diff(window, cx),
            Action::Quit => cx.quit(),
        }
    }

    /// The sidebar label for a tab, from its active pane, as tmux names a window: the
    /// title, else the program's name, else the address.
    fn tab_label(&self, t: &Tab<Pane>, cx: &gpui::App) -> String {
        let Some(p) = t.active_pane() else {
            return String::new();
        };
        let i = self.info.get(&p.address);
        let title = i.and_then(|i| i["title"].as_str()).filter(|s| !s.is_empty());
        let view_title = p.view.read(cx).title();
        title
            .map(str::to_string)
            .or_else(|| (!view_title.starts_with("nerdr — ")).then(|| view_title.to_string()))
            .or_else(|| i.and_then(|i| i["foreground"]["name"].as_str()).map(str::to_string))
            .unwrap_or_else(|| p.address.clone())
    }

    /// Panes as tmux draws them with lines between: the lines next to the active pane are
    /// lit, and a line can be dragged. `path` leads from the tab's root to `node`.
    fn render_panes(&self, tab: &Tab<Pane>, node: &Node<Pane>, path: Vec<bool>, cx: &mut Context<Self>) -> AnyElement {
        let (split, ratio, a, b) = match node {
            Node::Leaf(p) => {
                let leaf = div().relative().size_full().overflow_hidden().child(p.view.clone());
                return match (p.address == tab.active).then(|| active_frame(tab)).flatten() {
                    Some(frame) => leaf.child(frame).into_any_element(),
                    None => leaf.into_any_element(),
                };
            }
            Node::Split { split, ratio, a, b } => (*split, *ratio, a, b),
        };
        let line = hsla(theme::PANE_BORDER);
        let child = |second: bool| {
            let mut p = path.clone();
            p.push(second);
            p
        };
        let first = self.render_panes(tab, a, child(false), cx);
        let second = self.render_panes(tab, b, child(true), cx);
        let first = div().flex_none().overflow_hidden().child(first);
        let second = div().flex_1().min_w(px(0.)).min_h(px(0.)).overflow_hidden().child(second);
        // The line is 1 px; the grab area reaches 4 px into the first pane, which is painted
        // before the line, so the area stays on top
        let grab = div().absolute().id(gpui::ElementId::Name(format!("line-{}-{path:?}", tab.id).into()));
        let grab = match split {
            Dir::Horizontal => grab.top_0().bottom_0().left(px(-4.)).w(px(5.)).cursor_col_resize(),
            Dir::Vertical => grab.left_0().right_0().top(px(-4.)).h(px(5.)).cursor_row_resize(),
        };
        let id = tab.id;
        let grab = grab.on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                this.drag = Some((id, path.clone()));
                cx.stop_propagation();
            }),
        );
        let row = div().size_full().flex();
        match split {
            Dir::Horizontal => row
                .flex_row()
                .child(first.h_full().w(relative(ratio)))
                .child(div().relative().flex_none().w(px(1.)).h_full().bg(line).child(grab))
                .child(second.h_full()),
            Dir::Vertical => row
                .flex_col()
                .child(first.w_full().h(relative(ratio)))
                .child(div().relative().flex_none().h(px(1.)).w_full().bg(line).child(grab))
                .child(second.w_full()),
        }
        .into_any_element()
    }

    /// The most urgent status among a tab's panes, as a color. Done is green only until seen.
    fn status_color(&self, tab: &Tab<Pane>) -> Option<gpui::Hsla> {
        let rank = |state: &str, unseen: bool| match state {
            "blocked" | "error" => (3, theme::RED),
            "done" if unseen => (2, theme::GREEN),
            "working" => (1, theme::YELLOW),
            _ => (0, theme::MUTED),
        };
        tab.panes
            .leaves()
            .iter()
            .filter_map(|p| Some(rank(self.state(&p.address)?, self.seen.is_unseen(&p.address))))
            .max_by_key(|(r, _)| *r)
            .map(|(_, c)| hsla(c))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dim = hsla(theme::MUTED);
        let fg = hsla(theme::FG);
        let mut col = div()
            .id("sidebar")
            .w(px(SIDEBAR_W))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .py(px(8.))
            .bg(hsla(theme::SIDEBAR_BG))
            .font_family(self.metrics.font.family.clone())
            .text_size(px(13.));
        for (si, s) in self.tasks.tasks.iter().enumerate() {
            let active_space = si == self.tasks.active;
            col = col.child(
                div()
                    .id(("task", si))
                    .px(px(10.))
                    .pt(px(if si == 0 { 0. } else { 10. }))
                    .pb(px(2.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(if active_space { fg } else { dim })
                    .child(format!("{}  {}", si + 1, task_label(s)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select(si, None, window, cx)
                    })),
            );
            for (ti, t) in s.tabs.iter().enumerate() {
                let selected = active_space && ti == s.active;
                let dot = self.status_color(t);
                let row = div()
                    .id(("tab", si * 1000 + ti))
                    .mx(px(4.))
                    .pl(px(18.))
                    .pr(px(6.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .text_color(if selected { fg } else { dim })
                    .when(selected, |d| d.bg(hsla(theme::SIDEBAR_SELECTED)))
                    .child(
                        div()
                            .size(px(7.))
                            .flex_none()
                            .rounded(px(4.))
                            .when_some(dot, |d, c| d.bg(c)),
                    )
                    .child(div().flex_none().child((self.keys.base_index + ti).to_string()))
                    .child(
                        div()
                            .flex_1()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            // tmux's window_zoomed_flag, as in the user's status line
                            .child(if t.zoomed {
                                format!("{} 🔍", self.tab_label(t, cx))
                            } else {
                                self.tab_label(t, cx)
                            }),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select(si, Some(ti), window, cx)
                    }));
                col = col.child(row);
            }
        }
        // Red while the prefix waits for its key, like tmux's status bar with client_prefix
        let armed = self.prefix_armed;
        let badge = div()
            .px(px(6.))
            .rounded(px(4.))
            .text_color(if armed { hsla(theme::SIDEBAR_BG) } else { dim })
            .when(armed, |d| d.bg(hsla(theme::RED)))
            .child(self.keys.prefix.name());
        col.child(div().flex_1())
            .child(div().flex_none().flex().mx(px(10.)).mt(px(10.)).child(badge))
    }
}

/// The last part of the task's origin, `~` for the home folder.
fn task_label(t: &Task<Pane>) -> String {
    if t.origin == home() {
        return "~".into();
    }
    t.origin.rsplit('/').find(|p| !p.is_empty()).unwrap_or(&t.origin).to_string()
}

/// A yellow frame over the dividers around the active pane. Deferred so it paints on top
/// of the dividers and outside the pane's clip; none on the tab's outer edges.
fn active_frame(tab: &Tab<Pane>) -> Option<AnyElement> {
    let rects = tab.panes.rects();
    if rects.len() < 2 {
        return None;
    }
    let (_, r) = rects.into_iter().find(|(p, _)| p.address == tab.active)?;
    let (left, top) = (r.x > 1e-4, r.y > 1e-4);
    let (right, bottom) = (r.x + r.w < 1. - 1e-4, r.y + r.h < 1. - 1e-4);
    let out = |on: bool| px(if on { -1. } else { 0. });
    let mut frame = div()
        .absolute()
        .left(out(left))
        .top(out(top))
        .right(out(right))
        .bottom(out(bottom))
        .border_color(hsla(theme::YELLOW));
    if left {
        frame = frame.border_l_1();
    }
    if top {
        frame = frame.border_t_1();
    }
    if right {
        frame = frame.border_r_1();
    }
    if bottom {
        frame = frame.border_b_1();
    }
    Some(gpui::deferred(frame).into_any_element())
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panes = match self.shown_diff() {
            Some(d) => Some(
                div()
                    .size_full()
                    .child(d.clone())
                    // jetdiff leaves its quit keys to whoever shows it
                    .on_action(cx.listener(|this, _: &jetdiff_ui::Quit, window, cx| this.hide_diff(window, cx)))
                    .into_any_element(),
            ),
            None => self.tasks.active_tab().map(|t| match t.active_pane() {
                Some(p) if t.zoomed => div().size_full().child(p.view.clone()).into_any_element(),
                _ => self.render_panes(t, &t.panes, Vec::new(), cx),
            }),
        };
        let main = div()
            .relative()
            .flex_1()
            .h_full()
            .overflow_hidden()
            .children(panes)
            .children(self.render_picker(cx))
            .children(self.render_notice());
        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(hsla(theme::BG))
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::on_key))
            .on_mouse_move(cx.listener(Self::on_drag))
            .on_mouse_up(gpui::MouseButton::Left, cx.listener(|this, _, _, cx| this.end_drag(cx)))
            .child(self.render_sidebar(cx))
            .child(main)
    }
}
