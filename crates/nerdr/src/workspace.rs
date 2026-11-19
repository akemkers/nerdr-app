//! Spaces, tabs and panes, as tmux's sessions, windows and panes. A space is a group of
//! tabs (one project, say), a tab is a tree of panes (`panes.rs`), and a pane is one hc
//! terminal. The layout is saved in the hc document `nerdr`, so it survives restarts of
//! the app; the terminals themselves live in the core. Keys: `prefix.rs`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use anyhow::{Context as _, Result};
use futures::StreamExt;
use gpui::{
    AnyElement, Context, Entity, FocusHandle, Focusable, KeyDownEvent, Render, Subscription,
    Task, Window, div, prelude::*, px, relative,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::hc;
use crate::panes::{Dir, Node, Side};
use crate::prefix::{self, Action, Binding, Keys};
use crate::theme::{self, hsla};
use crate::view::{Metrics, TermEvent, TerminalView};

pub const DOC: &str = "nerdr";
/// Every terminal nerdr starts carries this label.
pub const LABEL: &str = "nerdr";
const SIDEBAR_W: f32 = 220.;

/// The saved form of the layout: the `nerdr` document.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Layout {
    #[serde(default)]
    pub spaces: Vec<SpaceDoc>,
    #[serde(default)]
    pub active: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SpaceDoc {
    pub id: String,
    /// Shown instead of the active tab's directory.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub tabs: Vec<TabDoc>,
    #[serde(default)]
    pub active: usize,
}

/// A tab: its pane tree of terminal addresses, and the active pane.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(from = "TabRepr")]
pub struct TabDoc {
    pub panes: Node<String>,
    pub active: String,
}

/// Before splits a tab was saved as its terminal's address.
#[derive(Deserialize)]
#[serde(untagged)]
enum TabRepr {
    Address(String),
    Tab {
        panes: Node<String>,
        #[serde(default)]
        active: Option<String>,
    },
}

impl From<TabRepr> for TabDoc {
    fn from(r: TabRepr) -> Self {
        match r {
            TabRepr::Address(a) => TabDoc::new(a),
            TabRepr::Tab { panes, active } => {
                let active = active.unwrap_or_else(|| panes.leaves()[0].clone());
                TabDoc { panes, active }
            }
        }
    }
}

impl TabDoc {
    pub fn new(address: String) -> Self {
        Self {
            active: address.clone(),
            panes: Node::Leaf(address),
        }
    }
}

impl Layout {
    /// Drops panes whose terminal isn't live any more, then empty tabs and spaces, and
    /// keeps the active indexes in range.
    pub fn reconcile(&mut self, live: &HashSet<String>) {
        for s in &mut self.spaces {
            let before = s.tabs.get(s.active).map(|t| t.active.clone());
            s.tabs = std::mem::take(&mut s.tabs)
                .into_iter()
                .filter_map(|t| {
                    let panes = t.panes.retain(&|a: &String| live.contains(a))?;
                    let active = if panes.contains(&|a: &String| *a == t.active) {
                        t.active
                    } else {
                        panes.leaves()[0].clone()
                    };
                    Some(TabDoc { panes, active })
                })
                .collect();
            s.active = before
                .and_then(|b| s.tabs.iter().position(|t| t.panes.contains(&|a: &String| *a == b)))
                .unwrap_or(0);
        }
        let before = self.spaces.get(self.active).map(|s| s.id.clone());
        self.spaces.retain(|s| !s.tabs.is_empty());
        self.active = before
            .and_then(|b| self.spaces.iter().position(|s| s.id == b))
            .unwrap_or(0);
    }

    pub fn contains(&self, address: &str) -> bool {
        self.spaces
            .iter()
            .any(|s| s.tabs.iter().any(|t| t.panes.contains(&|a: &String| a == address)))
    }

    /// Adds a terminal as the active tab of the active space (a new space if none).
    pub fn add(&mut self, address: String) {
        if self.spaces.is_empty() {
            self.spaces.push(SpaceDoc::new(address));
            self.active = 0;
            return;
        }
        let s = &mut self.spaces[self.active];
        s.tabs.push(TabDoc::new(address));
        s.active = s.tabs.len() - 1;
    }
}

impl SpaceDoc {
    pub fn new(first_tab: String) -> Self {
        Self {
            id: new_id(),
            name: None,
            tabs: vec![TabDoc::new(first_tab)],
            active: 0,
        }
    }
}

fn new_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("s{ms:x}{}", N.fetch_add(1, Ordering::Relaxed))
}

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

/// The layout to open: the saved one with dead tabs dropped, plus `address` if given.
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
    let mut layout: Layout = match &doc["value"] {
        Value::Null => Layout::default(),
        v => serde_json::from_value(v.clone()).unwrap_or_else(|e| {
            eprintln!("nerdr: ignoring unreadable layout ({e}): {v}");
            Layout::default()
        }),
    };
    let list = client.call_blocking("terminal.list", json!({}))?;
    let info: HashMap<String, Value> = list["terminals"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| Some((t["address"].as_str()?.to_string(), t.clone())))
        .collect();
    let live: HashSet<String> = info.keys().cloned().collect();
    layout.reconcile(&live);
    if let Some(a) = address {
        let a = resolve(client, a)?;
        if !layout.contains(&a) {
            layout.add(a);
        }
    }
    if layout.spaces.is_empty() {
        let run = client.call_blocking("terminal.run", shell_params(&home(), cols, rows, display))?;
        let a = run["address"].as_str().context("terminal.run: no address")?;
        eprintln!("nerdr: started {a}");
        layout.add(a.to_string());
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

struct Space {
    id: String,
    name: Option<String>,
    tabs: Vec<Tab>,
    active: usize,
    /// The tab active before this one, for tmux's last-window.
    last: Option<u64>,
}

struct Tab {
    /// Stays the same while panes come and go.
    id: u64,
    panes: Node<Pane>,
    /// The active pane's address.
    active: String,
    /// The pane active before it: where the focus goes back to when the active pane closes.
    last: Option<String>,
    /// tmux's zoom: only the active pane shows.
    zoomed: bool,
}

struct Pane {
    address: String,
    view: Entity<TerminalView>,
    _subs: Vec<Subscription>,
}

impl Tab {
    fn pane(&self, address: &str) -> Option<&Pane> {
        self.panes.find(&|p: &Pane| p.address == address)
    }

    fn active_pane(&self) -> Option<&Pane> {
        self.pane(&self.active)
    }
}

fn tab_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    N.fetch_add(1, Ordering::Relaxed)
}

pub struct Workspace {
    client: hc::Client,
    metrics: Metrics,
    display: Value,
    spaces: Vec<Space>,
    active: usize,
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
    _tasks: Vec<Task<()>>,
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
            spaces: Vec::new(),
            active: layout.active,
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
            _tasks: vec![feed],
            _subs: vec![cx.observe_window_bounds(window, Self::on_bounds)],
        };
        for s in layout.spaces {
            let tabs = s
                .tabs
                .into_iter()
                .filter_map(|t| this.open_tab(t, window, cx))
                .collect();
            this.spaces.push(Space {
                id: s.id,
                name: s.name,
                tabs,
                active: s.active,
                last: None,
            });
        }
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
        for si in 0..self.spaces.len() {
            for ti in 0..self.spaces[si].tabs.len() {
                self.edit_panes(si, ti, |this, panes| {
                    panes.try_map(&mut |p: Pane| Some(this.open_pane(p.address, window, cx)))
                });
            }
        }
        self.focus_active(window, cx);
        cx.notify();
    }

    /// Runs `f` on a tab's pane tree, which it owns meanwhile. `None` back drops the tab.
    fn edit_panes(
        &mut self,
        si: usize,
        ti: usize,
        f: impl FnOnce(&mut Self, Node<Pane>) -> Option<Node<Pane>>,
    ) -> bool {
        let mut tab = self.spaces[si].tabs.remove(ti);
        match f(self, tab.panes) {
            Some(panes) => {
                tab.panes = panes;
                self.spaces[si].tabs.insert(ti, tab);
                true
            }
            None => false,
        }
    }

    fn layout(&self) -> Layout {
        Layout {
            spaces: self
                .spaces
                .iter()
                .map(|s| SpaceDoc {
                    id: s.id.clone(),
                    name: s.name.clone(),
                    tabs: s
                        .tabs
                        .iter()
                        .map(|t| TabDoc {
                            panes: t.panes.map(&|p: &Pane| p.address.clone()),
                            active: t.active.clone(),
                        })
                        .collect(),
                    active: s.active,
                })
                .collect(),
            active: self.active,
        }
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
        let value = serde_json::to_value(self.layout()).unwrap();
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

    fn open_tab(&mut self, doc: TabDoc, window: &mut Window, cx: &mut Context<Self>) -> Option<Tab> {
        let panes = doc
            .panes
            .try_map(&mut |a: String| Some(self.open_pane(a, window, cx)))?;
        Some(Tab {
            id: tab_id(),
            panes,
            active: doc.active,
            last: None,
            zoomed: false,
        })
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
        let focused = cx.on_focus_in(&view.focus_handle(cx), window, move |this, _, cx| {
            this.on_pane_focused(&a, cx)
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

    fn active_tab(&self) -> Option<&Tab> {
        let s = self.spaces.get(self.active)?;
        s.tabs.get(s.active)
    }

    fn active_pane(&self) -> Option<&Pane> {
        self.active_tab()?.active_pane()
    }

    /// Where a pane is: space and tab index.
    fn find_pane(&self, address: &str) -> Option<(usize, usize)> {
        self.spaces.iter().enumerate().find_map(|(si, s)| {
            let ti = s.tabs.iter().position(|t| t.pane(address).is_some())?;
            Some((si, ti))
        })
    }

    pub fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active_pane() {
            Some(p) => window.focus(&p.view.focus_handle(cx)),
            None => window.focus(&self.focus),
        }
    }

    /// A click (or focus_active) put the focus in a pane: it is the active one.
    fn on_pane_focused(&mut self, address: &str, cx: &mut Context<Self>) {
        let Some((si, ti)) = self.find_pane(address) else {
            return;
        };
        let tab = &mut self.spaces[si].tabs[ti];
        if tab.active != address {
            tab.last = Some(std::mem::replace(&mut tab.active, address.to_string()));
            tab.zoomed = false;
            self.save(cx);
            cx.notify();
        }
    }

    /// Makes a pane of the active tab the active one; focusing it does the rest.
    fn activate_pane(&mut self, address: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = address.and_then(|a| Some(self.active_tab()?.pane(&a)?.view.clone())) else {
            return;
        };
        window.focus(&p.focus_handle(cx));
    }

    fn select_pane(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        let next = self.active_tab().and_then(|t| {
            let p = t.panes.neighbor(&|p: &Pane| p.address == t.active, side)?;
            Some(p.address.clone())
        });
        self.activate_pane(next, window, cx);
    }

    /// Where panes go: beside the sidebar, the whole height.
    fn pane_area(&self, window: &Window) -> (f32, f32) {
        let vp = window.viewport_size();
        (f32::from(vp.width) - SIDEBAR_W, f32::from(vp.height))
    }

    fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        let s = self.spaces.get_mut(self.active)?;
        s.tabs.get_mut(s.active)
    }

    fn resize_pane(&mut self, side: Side, cells: u16, window: &mut Window, cx: &mut Context<Self>) {
        let area = self.pane_area(window);
        let cell = match side {
            Side::Left | Side::Right => self.metrics.cell_w,
            Side::Up | Side::Down => self.metrics.cell_h,
        };
        let Some(t) = self.active_tab_mut() else {
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
        let Some(t) = self.active_tab_mut().filter(|t| t.id == id) else {
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
        let Some(s) = self.spaces.get_mut(self.active) else {
            return;
        };
        if let Some(t) = s.tabs.get_mut(s.active)
            && matches!(t.panes, Node::Split { .. })
        {
            t.zoomed = !t.zoomed;
            cx.notify();
        }
    }

    fn select(&mut self, space: usize, tab: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.spaces.get_mut(space) else {
            return;
        };
        if let Some(t) = tab
            && t < s.tabs.len()
            && t != s.active
        {
            s.last = s.tabs.get(s.active).map(|t| t.id);
            s.active = t;
        }
        self.active = space;
        self.focus_active(window, cx);
        self.save(cx);
        cx.notify();
    }

    fn cycle_tab(&mut self, d: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(s) = self.spaces.get(self.active) {
            let n = s.tabs.len() as isize;
            let t = (s.active as isize + d).rem_euclid(n.max(1)) as usize;
            self.select(self.active, Some(t), window, cx);
        }
    }

    fn last_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.spaces.get(self.active) else {
            return;
        };
        let last = s.last.and_then(|id| s.tabs.iter().position(|t| t.id == id));
        if let Some(t) = last {
            self.select(self.active, Some(t), window, cx);
        }
    }

    fn cycle_space(&mut self, d: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.spaces.len() as isize;
        if n > 0 {
            let s = (self.active as isize + d).rem_euclid(n) as usize;
            self.select(s, None, window, cx);
        }
    }

    /// Where a pane's program is, for new tabs and panes next to it.
    fn cwd_of(&self, address: &str) -> String {
        self.info
            .get(address)
            .and_then(|i| i["foreground"]["cwd"].as_str().or(i["cwd"].as_str()))
            .map(str::to_string)
            .unwrap_or_else(home)
    }

    fn active_cwd(&self) -> String {
        self.active_pane()
            .map(|p| self.cwd_of(&p.address))
            .unwrap_or_else(home)
    }

    /// Starts a shell and opens it as a tab: in the active space, or in a new one.
    fn new_tab(&mut self, new_space: bool, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = if new_space { home() } else { self.active_cwd() };
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
                let Some(tab) = this.open_tab(TabDoc::new(address), window, cx) else {
                    return;
                };
                if new_space || this.spaces.is_empty() {
                    this.spaces.push(Space {
                        id: new_id(),
                        name: None,
                        tabs: vec![tab],
                        active: 0,
                        last: None,
                    });
                    let last = this.spaces.len() - 1;
                    this.select(last, Some(0), window, cx);
                } else {
                    let s = &mut this.spaces[this.active];
                    s.tabs.push(tab);
                    let last = s.tabs.len() - 1;
                    this.select(this.active, Some(last), window, cx);
                }
            });
        })
        .detach();
    }

    /// tmux's split-window: a shell in the active pane's directory takes half of it and
    /// becomes the active pane.
    fn split(&mut self, dir: Dir, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.active_pane() else {
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
                let Some((si, ti)) = this.find_pane(&at) else {
                    return;
                };
                let new = this.open_pane(address.clone(), window, cx);
                this.edit_panes(si, ti, |_, panes| {
                    Some(panes.split(&|p: &Pane| p.address == at, dir, new).0)
                });
                let tab = &mut this.spaces[si].tabs[ti];
                tab.last = Some(std::mem::replace(&mut tab.active, address));
                tab.zoomed = false;
                this.focus_active(window, cx);
                this.save(cx);
                cx.notify();
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
        let Some(address) = self.active_pane().map(|p| p.address.clone()) else {
            return;
        };
        self.remove_pane(&address, window, cx);
        self.kill(address, cx);
    }

    /// tmux's kill-window: the tab and every pane in it.
    fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.active_tab() else {
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

    /// Drops the pane (detaching its stream); its sibling takes the space. Then empty tabs
    /// and spaces go. The last pane of all closes the app.
    fn remove_pane(&mut self, address: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((si, ti)) = self.find_pane(address) else {
            return;
        };
        let kept = self.edit_panes(si, ti, |_, panes| panes.remove(&|p: &Pane| p.address == address));
        if kept {
            let tab = &mut self.spaces[si].tabs[ti];
            tab.zoomed = false;
            if tab.active == address {
                // As tmux: back to the last pane when it's still there
                tab.active = match tab.last.take() {
                    Some(l) if tab.pane(&l).is_some() => l,
                    _ => tab.panes.leaves()[0].address.clone(),
                };
            } else if tab.last.as_deref() == Some(address) {
                tab.last = None;
            }
        } else {
            let s = &mut self.spaces[si];
            if s.active > ti || s.active >= s.tabs.len() {
                s.active = s.active.saturating_sub(1);
            }
            if s.tabs.is_empty() {
                self.spaces.remove(si);
                if self.active > si || self.active >= self.spaces.len() {
                    self.active = self.active.saturating_sub(1);
                }
            }
        }
        self.save(cx);
        if self.spaces.is_empty() {
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
        let name = prefix::key_name(k);
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
        match action {
            Action::NewTab => self.new_tab(false, window, cx),
            Action::NewSpace => self.new_tab(true, window, cx),
            Action::NextTab => self.cycle_tab(1, window, cx),
            Action::PrevTab => self.cycle_tab(-1, window, cx),
            Action::SelectTab(t) => self.select(self.active, Some(t), window, cx),
            Action::LastTab => self.last_tab(window, cx),
            Action::CloseTab => self.close_tab(window, cx),
            Action::NextSpace => self.cycle_space(1, window, cx),
            Action::PrevSpace => self.cycle_space(-1, window, cx),
            Action::Split(dir) => self.split(dir, window, cx),
            Action::ClosePane => self.close_pane(window, cx),
            Action::SelectPane(side) => self.select_pane(side, window, cx),
            Action::Zoom => self.toggle_zoom(cx),
            Action::Resize(side, cells) => self.resize_pane(side, cells, window, cx),
            Action::ScrollPageUp => {
                if let Some(p) = self.active_pane() {
                    p.view.update(cx, |v, cx| v.scroll_pages(1, cx));
                }
            }
            Action::SendPrefix => {
                let ks = self.keys.prefix.keystroke();
                if let Some(p) = self.active_pane() {
                    p.view.update(cx, |v, cx| v.type_keystroke(&ks, cx));
                }
            }
            Action::Quit => cx.quit(),
        }
    }

    /// The sidebar label for a tab, from its active pane, as tmux names a window: the
    /// title, else the program's name, else the address.
    fn tab_label(&self, t: &Tab, cx: &gpui::App) -> String {
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

    /// The space's name, else the last part of its active pane's directory.
    fn space_label(&self, s: &Space) -> String {
        if let Some(n) = &s.name {
            return n.clone();
        }
        s.tabs
            .get(s.active)
            .and_then(|t| t.active_pane())
            .and_then(|p| self.info.get(&p.address))
            .and_then(|i| i["foreground"]["cwd"].as_str().or(i["cwd"].as_str()))
            .map(|c| {
                if c == home() {
                    "~".to_string()
                } else {
                    c.rsplit('/').next().unwrap_or(c).to_string()
                }
            })
            .unwrap_or_else(|| "space".into())
    }

    /// Panes as tmux draws them with lines between: the lines next to the active pane are
    /// lit, and a line can be dragged. `path` leads from the tab's root to `node`.
    fn render_panes(&self, tab: &Tab, node: &Node<Pane>, path: Vec<bool>, cx: &mut Context<Self>) -> AnyElement {
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

    /// The most urgent agent status (OSC 7501) among a tab's panes, as a color.
    fn status_color(&self, tab: &Tab) -> Option<gpui::Hsla> {
        let rank = |state: &str| match state {
            "blocked" | "error" => (3, theme::RED),
            "done" => (2, theme::GREEN),
            "working" => (1, theme::YELLOW),
            _ => (0, theme::MUTED),
        };
        tab.panes
            .leaves()
            .iter()
            .filter_map(|p| self.info.get(&p.address)?["status"]["state"].as_str().map(rank))
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
        for (si, s) in self.spaces.iter().enumerate() {
            let active_space = si == self.active;
            col = col.child(
                div()
                    .id(("space", si))
                    .px(px(10.))
                    .pt(px(if si == 0 { 0. } else { 10. }))
                    .pb(px(2.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(if active_space { fg } else { dim })
                    .child(format!("{}  {}", si + 1, self.space_label(s)))
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

/// A yellow frame over the dividers around the active pane. Deferred so it paints on top
/// of the dividers and outside the pane's clip; none on the tab's outer edges.
fn active_frame(tab: &Tab) -> Option<AnyElement> {
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
        let panes = self.active_tab().map(|t| match t.active_pane() {
            Some(p) if t.zoomed => div().size_full().child(p.view.clone()).into_any_element(),
            _ => self.render_panes(t, &t.panes, Vec::new(), cx),
        });
        let main = div().flex_1().h_full().overflow_hidden().children(panes);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(spaces: &[(&str, &[&str], usize)], active: usize) -> Layout {
        Layout {
            spaces: spaces
                .iter()
                .map(|(id, tabs, a)| SpaceDoc {
                    id: id.to_string(),
                    name: None,
                    tabs: tabs.iter().map(|t| TabDoc::new(t.to_string())).collect(),
                    active: *a,
                })
                .collect(),
            active,
        }
    }

    fn live(ts: &[&str]) -> HashSet<String> {
        ts.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn reconcile_keeps_active_tab_and_space() {
        let mut l = layout(&[("a", &["t1", "t2", "t3"], 2), ("b", &["t4"], 0)], 0);
        l.reconcile(&live(&["t2", "t3", "t4"]));
        assert_eq!(l, layout(&[("a", &["t2", "t3"], 1), ("b", &["t4"], 0)], 0));
    }

    #[test]
    fn reconcile_drops_empty_spaces() {
        let mut l = layout(&[("a", &["t1"], 0), ("b", &["t4"], 0), ("c", &["t5"], 0)], 2);
        l.reconcile(&live(&["t5"]));
        assert_eq!(l, layout(&[("c", &["t5"], 0)], 0));
    }

    #[test]
    fn reconcile_active_tab_gone_falls_back_to_first() {
        let mut l = layout(&[("a", &["t1", "t2"], 1)], 0);
        l.reconcile(&live(&["t1"]));
        assert_eq!(l, layout(&[("a", &["t1"], 0)], 0));
    }

    #[test]
    fn reconcile_closes_up_split_tabs() {
        let panes = Node::Leaf("t1".to_string())
            .split(&|a: &String| a == "t1", Dir::Horizontal, "t2".to_string())
            .0;
        let mut l = layout(&[("a", &["x"], 0)], 0);
        l.spaces[0].tabs[0] = TabDoc { panes, active: "t2".into() };
        l.reconcile(&live(&["t1"]));
        assert_eq!(l, layout(&[("a", &["t1"], 0)], 0));
    }

    #[test]
    fn add_goes_to_active_space_or_a_new_one() {
        let mut l = Layout::default();
        l.add("t1".into());
        assert_eq!(l.spaces.len(), 1);
        l.add("t2".into());
        assert_eq!(l.spaces[0].tabs, [TabDoc::new("t1".into()), TabDoc::new("t2".into())]);
        assert_eq!(l.spaces[0].active, 1);
        assert!(l.contains("t2") && !l.contains("t3"));
    }

    #[test]
    fn layout_reads_documents_from_before_splits() {
        let l: Layout =
            serde_json::from_value(json!({"spaces":[{"id":"x","tabs":["t1"]}]})).unwrap();
        assert_eq!(l, layout(&[("x", &["t1"], 0)], 0));
    }

    #[test]
    fn layout_round_trips_splits() {
        let panes = Node::Leaf("t1".to_string())
            .split(&|a: &String| a == "t1", Dir::Vertical, "t2".to_string())
            .0;
        let mut l = layout(&[("a", &["x"], 0)], 0);
        l.spaces[0].tabs[0] = TabDoc { panes, active: "t2".into() };
        let back: Layout = serde_json::from_value(serde_json::to_value(&l).unwrap()).unwrap();
        assert_eq!(back, l);
    }
}
