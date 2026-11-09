//! Spaces and tabs. A space is a group of tabs (one project, say); a tab is one hc
//! terminal. The layout is saved in the hc document `nerdr`, so it survives restarts of
//! the app; the terminals themselves live in the core.
//!
//! Keys (taken before the terminal sees them):
//!
//! | key | action |
//! |---|---|
//! | ctrl+shift+t | new tab in this space, in the current tab's directory |
//! | ctrl+shift+w | close tab (ends its program) |
//! | ctrl+tab, ctrl+shift+tab | next, previous tab |
//! | alt+1 … alt+9 | tab 1 … 9 |
//! | ctrl+shift+n | new space |
//! | ctrl+shift+down, ctrl+shift+up | next, previous space |

use std::collections::{HashMap, HashSet};

use anyhow::{Context as _, Result};
use futures::StreamExt;
use gpui::{
    Context, Entity, FocusHandle, Focusable, KeyDownEvent, Render, Subscription, Task, Window,
    div, prelude::*, px,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::hc;
use crate::theme::{self, hsla};
use crate::view::{FAMILY, Metrics, TermEvent, TerminalView};

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
    /// Terminal addresses, in tab order.
    #[serde(default)]
    pub tabs: Vec<String>,
    #[serde(default)]
    pub active: usize,
}

impl Layout {
    /// Drops tabs whose terminal isn't live any more, then empty spaces, and keeps the
    /// active indexes in range.
    pub fn reconcile(&mut self, live: &HashSet<String>) {
        for s in &mut self.spaces {
            let before = s.tabs.get(s.active).cloned();
            s.tabs.retain(|t| live.contains(t));
            s.active = before
                .and_then(|b| s.tabs.iter().position(|t| *t == b))
                .unwrap_or(0);
        }
        let before = self.spaces.get(self.active).map(|s| s.id.clone());
        self.spaces.retain(|s| !s.tabs.is_empty());
        self.active = before
            .and_then(|b| self.spaces.iter().position(|s| s.id == b))
            .unwrap_or(0);
    }

    pub fn contains(&self, address: &str) -> bool {
        self.spaces.iter().any(|s| s.tabs.iter().any(|t| t == address))
    }

    /// Adds a terminal as the active tab of the active space (a new space if none).
    pub fn add(&mut self, address: String) {
        if self.spaces.is_empty() {
            self.spaces.push(SpaceDoc::new(address));
            self.active = 0;
            return;
        }
        let s = &mut self.spaces[self.active];
        s.tabs.push(address);
        s.active = s.tabs.len() - 1;
    }
}

impl SpaceDoc {
    pub fn new(first_tab: String) -> Self {
        Self {
            id: new_id(),
            name: None,
            tabs: vec![first_tab],
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
}

struct Tab {
    address: String,
    view: Entity<TerminalView>,
    _subs: Vec<Subscription>,
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
    _tasks: Vec<Task<()>>,
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
            _tasks: vec![feed],
        };
        for s in layout.spaces {
            let tabs = s
                .tabs
                .into_iter()
                .map(|a| this.open_tab(a, window, cx))
                .collect();
            this.spaces.push(Space {
                id: s.id,
                name: s.name,
                tabs,
                active: s.active,
            });
        }
        this.save(cx);
        this
    }

    fn layout(&self) -> Layout {
        Layout {
            spaces: self
                .spaces
                .iter()
                .map(|s| SpaceDoc {
                    id: s.id.clone(),
                    name: s.name.clone(),
                    tabs: s.tabs.iter().map(|t| t.address.clone()).collect(),
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

    fn open_tab(&mut self, address: String, window: &mut Window, cx: &mut Context<Self>) -> Tab {
        let (cols, rows) = self.grid_size(window);
        let params = json!({"terminal": address, "display": self.display, "history": 5000});
        let client = self.client.clone();
        let metrics = self.metrics.clone();
        let view = cx.new(|cx| TerminalView::new(&client, params, cols, rows, metrics, window, cx));
        let a = address.clone();
        let exited = cx.subscribe_in(&view, window, move |this, _, ev, window, cx| match ev {
            TermEvent::Exited => this.on_exited(&a, window, cx),
        });
        let changed = cx.observe(&view, |_, _, cx| cx.notify());
        Tab {
            address,
            view,
            _subs: vec![exited, changed],
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

    pub fn focus_active(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active_tab() {
            Some(t) => window.focus(&t.view.focus_handle(cx)),
            None => window.focus(&self.focus),
        }
    }

    fn select(&mut self, space: usize, tab: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.spaces.get_mut(space) else {
            return;
        };
        if let Some(t) = tab
            && t < s.tabs.len()
        {
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

    fn cycle_space(&mut self, d: isize, window: &mut Window, cx: &mut Context<Self>) {
        let n = self.spaces.len() as isize;
        if n > 0 {
            let s = (self.active as isize + d).rem_euclid(n) as usize;
            self.select(s, None, window, cx);
        }
    }

    /// The directory a new tab should start in: where the active tab's program is.
    fn active_cwd(&self) -> String {
        self.active_tab()
            .and_then(|t| self.info.get(&t.address))
            .and_then(|i| i["foreground"]["cwd"].as_str().or(i["cwd"].as_str()))
            .map(str::to_string)
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
                let tab = this.open_tab(address, window, cx);
                if new_space || this.spaces.is_empty() {
                    this.spaces.push(Space {
                        id: new_id(),
                        name: None,
                        tabs: vec![tab],
                        active: 0,
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

    /// Closes the active tab and ends its program.
    fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(address) = self.active_tab().map(|t| t.address.clone()) else {
            return;
        };
        self.remove_tab(&address, window, cx);
        let client = self.client.clone();
        cx.spawn(async move |_, _| {
            let t = json!({"terminal": address});
            let _ = client.call("terminal.kill", t.clone()).await;
            let _ = client.call("terminal.rm", t).await;
        })
        .detach();
    }

    /// The terminal's program ended: close its tab and let the core forget it.
    fn on_exited(&mut self, address: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.remove_tab(address, window, cx);
        let rm = self.client.call("terminal.rm", json!({"terminal": address}));
        cx.spawn(async move |_, _| {
            let _ = rm.await;
        })
        .detach();
    }

    /// Drops the tab (detaching its stream), then empty spaces. The last tab closes the app.
    fn remove_tab(&mut self, address: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(si) = self
            .spaces
            .iter()
            .position(|s| s.tabs.iter().any(|t| t.address == address))
        else {
            return;
        };
        let s = &mut self.spaces[si];
        let ti = s.tabs.iter().position(|t| t.address == address).unwrap();
        s.tabs.remove(ti);
        if s.active > ti || s.active >= s.tabs.len() {
            s.active = s.active.saturating_sub(1);
        }
        if s.tabs.is_empty() {
            self.spaces.remove(si);
            if self.active > si || self.active >= self.spaces.len() {
                self.active = self.active.saturating_sub(1);
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

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        let key = k.key.as_str();
        let handled = match (m.control, m.shift, m.alt, key) {
            (true, true, false, "t") => {
                self.new_tab(false, window, cx);
                true
            }
            (true, true, false, "n") => {
                self.new_tab(true, window, cx);
                true
            }
            (true, true, false, "w") => {
                self.close_tab(window, cx);
                true
            }
            (true, false, false, "tab") => {
                self.cycle_tab(1, window, cx);
                true
            }
            (true, true, false, "tab") => {
                self.cycle_tab(-1, window, cx);
                true
            }
            (true, true, false, "down") => {
                self.cycle_space(1, window, cx);
                true
            }
            (true, true, false, "up") => {
                self.cycle_space(-1, window, cx);
                true
            }
            (false, false, true, d) if d.len() == 1 && ("1"..="9").contains(&d) => {
                let t = d.parse::<usize>().unwrap() - 1;
                self.select(self.active, Some(t), window, cx);
                true
            }
            _ => false,
        };
        if handled {
            cx.stop_propagation();
        }
    }

    /// The sidebar label for a tab: its title, else the program's name, else the address.
    fn tab_label(&self, t: &Tab, cx: &gpui::App) -> String {
        let i = self.info.get(&t.address);
        let title = i.and_then(|i| i["title"].as_str()).filter(|s| !s.is_empty());
        let view_title = t.view.read(cx).title();
        title
            .map(str::to_string)
            .or_else(|| (!view_title.starts_with("nerdr — ")).then(|| view_title.to_string()))
            .or_else(|| i.and_then(|i| i["foreground"]["name"].as_str()).map(str::to_string))
            .unwrap_or_else(|| t.address.clone())
    }

    /// The space's name, else the last part of its active tab's directory.
    fn space_label(&self, s: &Space) -> String {
        if let Some(n) = &s.name {
            return n.clone();
        }
        s.tabs
            .get(s.active)
            .and_then(|t| self.info.get(&t.address))
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

    /// The agent status (OSC 7501) of a terminal as a color, if it reported one.
    fn status_color(&self, address: &str) -> Option<gpui::Hsla> {
        let state = self.info.get(address)?["status"]["state"].as_str()?;
        let c = match state {
            "working" => 0xe0af68,
            "blocked" => 0xf7768e,
            "error" => 0xf7768e,
            "done" => 0x9ece6a,
            _ => 0x565f89,
        };
        Some(gpui::rgb(c).into())
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dim: gpui::Hsla = gpui::rgb(0x565f89).into();
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
            .bg(gpui::rgb(0x16161e))
            .font_family(FAMILY)
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
                let dot = self.status_color(&t.address);
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
                    .when(selected, |d| d.bg(hsla(theme::SELECTION)))
                    .child(
                        div()
                            .size(px(7.))
                            .flex_none()
                            .rounded(px(4.))
                            .when_some(dot, |d, c| d.bg(c)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(self.tab_label(t, cx)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select(si, Some(ti), window, cx)
                    }));
                col = col.child(row);
            }
        }
        col
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let main = div().flex_1().h_full().overflow_hidden().children(
            self.active_tab().map(|t| t.view.clone()),
        );
        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(hsla(theme::BG))
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::on_key))
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
                    tabs: tabs.iter().map(|t| t.to_string()).collect(),
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
    fn add_goes_to_active_space_or_a_new_one() {
        let mut l = Layout::default();
        l.add("t1".into());
        assert_eq!(l.spaces.len(), 1);
        l.add("t2".into());
        assert_eq!(l.spaces[0].tabs, ["t1", "t2"]);
        assert_eq!(l.spaces[0].active, 1);
        assert!(l.contains("t2") && !l.contains("t3"));
    }

    #[test]
    fn layout_reads_partial_documents() {
        let l: Layout =
            serde_json::from_value(json!({"spaces":[{"id":"x","tabs":["t1"]}]})).unwrap();
        assert_eq!(l, layout(&[("x", &["t1"], 0)], 0));
    }
}
