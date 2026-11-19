//! One hc terminal in a GPUI view: alacritty_terminal keeps the grid, a canvas draws it.

use std::cell::{Cell as StdCell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::{Flags, Hyperlink};
use alacritty_terminal::term::{self, Term, TermMode};
use alacritty_terminal::vte::ansi::{self, CursorShape, NamedColor, Rgb};
use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{
    App, Bounds, ClipboardItem, Context, FocusHandle, Focusable, Font, FontFeatures, FontStyle,
    FontWeight, Hsla, KeyDownEvent, KeyUpEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    Pixels, Point, Render, ScrollWheelEvent, ShapedLine, SharedString, TextRun, Window, canvas,
    div, fill, font, outline, point, prelude::*, px, size,
};

use crate::hc;
use crate::ime::TermInput;
use crate::keys::{self, KeyAction};
use crate::theme::{self, hsla};

// Kitty's font when installed, else the bundled one.
const FAMILY: &str = "Hack Nerd Font Mono";
const BUNDLED: &str = "JetBrains Mono";
const PAD: f32 = 6.;
const BLINK: Duration = Duration::from_millis(530);

/// Font and cell size, fixed for the window's life.
#[derive(Clone)]
pub struct Metrics {
    pub font: Font,
    pub font_size: Pixels,
    pub cell_w: f32,
    pub cell_h: f32,
}

impl Metrics {
    pub fn new(font_size: f32, window: &Window) -> Self {
        let ts = window.text_system();
        let installed = ts.all_font_names().iter().any(|n| n == FAMILY);
        let family = if installed { FAMILY } else { BUNDLED };
        let font = Font {
            features: FontFeatures::disable_ligatures(),
            ..font(family)
        };
        let id = ts.resolve_font(&font);
        let size = px(font_size);
        let scale = window.scale_factor();
        let adv = ts
            .advance(id, size, 'm')
            .map(|s| f32::from(s.width))
            .unwrap_or(font_size * 0.6);
        let h = f32::from(ts.ascent(id, size)) + f32::from(ts.descent(id, size)).abs();
        // Whole device pixels, so every cell edge lands on a pixel
        let snap = |v: f32| (v * scale).round() / scale;
        let snap_up = |v: f32| (v * scale).ceil() / scale;
        Self {
            font,
            font_size: size,
            cell_w: snap(adv),
            // Line height rounded up before the 130%, as kitty does
            cell_h: snap(snap_up(h) * 1.3),
        }
    }

    pub fn grid_size(&self, w: f32, h: f32) -> (u16, u16) {
        let cols = ((w - 2. * PAD) / self.cell_w).floor().max(2.) as u16;
        let rows = ((h - 2. * PAD) / self.cell_h).floor().max(1.) as u16;
        (cols, rows)
    }
}

/// Events from the terminal parser. Runs on the reader thread with the term locked.
pub struct Listener {
    title: Arc<Mutex<Option<String>>>,
    debug: bool,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(t) => *self.title.lock().unwrap() = Some(t),
            Event::ResetTitle => *self.title.lock().unwrap() = Some("nerdr".into()),
            // The core already answers device queries (it runs its own Ghostty terminal and
            // got our colors and cell size in `display`). Answering here too would type the
            // reply twice, so these are only logged.
            Event::PtyWrite(s) if self.debug => eprintln!("nerdr: unanswered PtyWrite {s:?}"),
            Event::ColorRequest(i, _) if self.debug => eprintln!("nerdr: unanswered color {i}"),
            _ => {}
        }
    }
}

pub struct Shared {
    pub term: FairMutex<Term<Listener>>,
    pub title: Arc<Mutex<Option<String>>>,
    pub exited: Mutex<Option<String>>,
    /// Set by the reader, cleared by the UI: one wakeup per frame at most.
    dirty: AtomicBool,
    /// The core closed the stream for good (the program ended, or attach failed).
    ended: AtomicBool,
}

/// What a terminal view tells whoever holds it.
pub enum TermEvent {
    /// Its program ended, or the terminal is gone: the tab can close.
    Exited,
}

impl gpui::EventEmitter<TermEvent> for TerminalView {}

struct TermSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct TerminalView {
    shared: Arc<Shared>,
    stream: Rc<hc::Stream>,
    focus: FocusHandle,
    metrics: Metrics,
    /// Grid size and where the grid was last painted, from prepaint.
    size: Rc<StdCell<(u16, u16)>>,
    origin: Rc<StdCell<Point<Pixels>>>,
    scroll_acc: f32,
    selecting: bool,
    /// Keys whose press was sent, so their release can be too.
    pressed: HashSet<String>,
    title: String,
    /// The window's title as last set, shared by all tabs.
    window_title: Rc<RefCell<String>>,
    focused: bool,
    /// The OSC 8 link under the mouse: underlined, and a click opens it.
    hover_link: Option<Hyperlink>,
    marked: Option<String>,
    /// In the off half of a blink.
    blink_off: bool,
    /// A key was pressed: keep the cursor on through the next tick.
    blink_hold: bool,
    _tasks: Vec<gpui::Task<()>>,
    _subs: Vec<gpui::Subscription>,
}

impl TerminalView {
    /// Attaches a terminal on `client`. `attach` holds the `terminal.attach` params
    /// (terminal, display, history); cols and rows are filled in.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        client: &hc::Client,
        mut attach: serde_json::Value,
        cols: u16,
        rows: u16,
        metrics: Metrics,
        window_title: Rc<RefCell<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let debug = std::env::var_os("NERDR_DEBUG").is_some();
        let title = Arc::new(Mutex::new(None));
        let config = term::Config {
            scrolling_history: 10_000,
            kitty_keyboard: true,
            ..Default::default()
        };
        let term = Term::new(
            config,
            &TermSize {
                cols: cols as usize,
                rows: rows as usize,
            },
            Listener {
                title: title.clone(),
                debug,
            },
        );
        let shared = Arc::new(Shared {
            term: FairMutex::new(term),
            title,
            exited: Mutex::new(None),
            dirty: AtomicBool::new(false),
            ended: AtomicBool::new(false),
        });

        let (wake_tx, mut wake_rx) = unbounded::<()>();
        let sink = TermSink {
            shared: shared.clone(),
            parser: Mutex::new(ansi::Processor::new()),
            trace: std::env::var_os("NERDR_TRACE")
                .map(|p| Mutex::new(std::fs::File::create(p).expect("NERDR_TRACE file"))),
            stats: debug.then(|| Mutex::new(Stats::default())),
            wake: wake_tx,
        };
        let title = format!("nerdr — {}", attach["terminal"].as_str().unwrap_or("?"));
        attach["cols"] = cols.into();
        attach["rows"] = rows.into();
        let stream = Rc::new(client.attach(attach, Arc::new(sink)));
        let ui_shared = shared.clone();
        let wake = cx.spawn(async move |this, cx| {
            let mut told = false;
            while wake_rx.next().await.is_some() {
                ui_shared.dirty.store(false, Ordering::Release);
                let ended = ui_shared.ended.load(Ordering::Acquire);
                let tell = ended && !told;
                told = ended;
                let alive = this.update(cx, |this, cx| {
                    this.refresh_title();
                    cx.notify();
                    if tell {
                        cx.emit(TermEvent::Exited);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        let focus = cx.focus_handle();
        let focus_in = cx.on_focus_in(&focus, window, |this, _, _| {
            this.focused = true;
            this.stream.focus(true);
        });
        let focus_out = cx.on_focus_out(&focus, window, |this, _, _, _| {
            this.focused = false;
            this.stream.focus(false);
        });
        let blink = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK).await;
                if this.update(cx, |this, cx| this.blink(cx)).is_err() {
                    break;
                }
            }
        });

        Self {
            shared,
            stream,
            focus,
            metrics,
            size: Rc::new(StdCell::new((cols, rows))),
            origin: Rc::new(StdCell::new(point(px(PAD), px(PAD)))),
            scroll_acc: 0.,
            selecting: false,
            pressed: HashSet::new(),
            title,
            window_title,
            focused: false,
            hover_link: None,
            marked: None,
            blink_off: false,
            blink_hold: false,
            _tasks: vec![wake, blink],
            _subs: vec![focus_in, focus_out],
        }
    }

    /// Blinks only when the program asked for it and the terminal has focus.
    fn blink(&mut self, cx: &mut Context<Self>) {
        let wanted = self.focused && self.shared.term.lock().cursor_style().blinking;
        let off = wanted && !self.blink_hold && !self.blink_off;
        self.blink_hold = false;
        if off != self.blink_off {
            self.blink_off = off;
            cx.notify();
        }
    }

    fn send(&self, bytes: &[u8]) {
        self.stream.input(bytes);
    }

    fn mode(&self) -> TermMode {
        *self.shared.term.lock().mode()
    }

    fn on_key_down(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        let text_key = k
            .key_char
            .as_ref()
            .is_some_and(|t| !t.is_empty() && !t.chars().any(char::is_control))
            && !m.control
            // Option types characters on macOS ({[]}| on Norwegian layouts), as in kitty's default
            && (!m.alt || cfg!(target_os = "macos"))
            && !m.platform
            && !m.function;
        // Left to the input handler (ime.rs), so dead keys and IMEs can compose. Not when
        // the program wants every key as an escape code.
        if text_key && !self.mode().contains(TermMode::REPORT_ALL_KEYS_AS_ESC) {
            return;
        }
        cx.stop_propagation();
        if m.control && m.shift {
            match k.key.as_str() {
                "c" => {
                    if let Some(text) = self.shared.term.lock().selection_to_string() {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                    return;
                }
                "v" => {
                    self.paste(cx.read_from_clipboard());
                    return;
                }
                _ => {}
            }
        }
        if m.shift && !m.control && matches!(k.key.as_str(), "pageup" | "pagedown") {
            self.scroll_pages(if k.key == "pageup" { 1 } else { -1 }, cx);
            return;
        }
        let mode = self.mode();
        let action = if ev.is_held { KeyAction::Repeat } else { KeyAction::Press };
        if let Some(bytes) = keys::encode(k, mode, action) {
            self.pressed.insert(k.key.clone());
            self.type_bytes(&bytes, cx);
        }
    }

    /// Up into the scrollback by whole screens; negative goes down.
    pub fn scroll_pages(&mut self, pages: i32, cx: &mut Context<Self>) {
        let rows = self.size.get().1 as i32;
        self.shared.term.lock().scroll_display(Scroll::Delta(pages * rows));
        cx.notify();
    }

    /// A key on the program's behalf (the prefix pressed twice), encoded as if typed.
    pub fn type_keystroke(&mut self, ks: &gpui::Keystroke, cx: &mut Context<Self>) {
        if let Some(bytes) = keys::encode(ks, self.mode(), KeyAction::Press) {
            self.type_bytes(&bytes, cx);
        }
    }

    /// Input from the keyboard: back to the bottom, selection gone, cursor shown.
    fn type_bytes(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        self.blink_off = false;
        self.blink_hold = true;
        {
            let mut t = self.shared.term.lock();
            t.scroll_display(Scroll::Bottom);
            t.selection = None;
        }
        self.send(bytes);
        cx.notify();
    }

    /// Text being composed by the IME or a dead key, drawn at the cursor until committed.
    pub fn marked(&self) -> Option<&str> {
        self.marked.as_deref()
    }

    pub fn set_marked(&mut self, text: &str, cx: &mut Context<Self>) {
        self.marked = (!text.is_empty()).then(|| text.to_string());
        cx.notify();
    }

    pub fn commit_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.marked = None;
        if text.is_empty() {
            cx.notify();
        } else {
            self.type_bytes(text.as_bytes(), cx);
        }
    }

    /// The cursor cell in window coordinates, where the IME puts its candidate window.
    pub fn cursor_bounds(&self) -> Bounds<Pixels> {
        let term = self.shared.term.lock();
        let p = term.grid().cursor.point;
        let row = p.line.0 + term.grid().display_offset() as i32;
        let o = self.origin.get();
        let (cw, ch) = (self.metrics.cell_w, self.metrics.cell_h);
        Bounds::new(
            point(o.x + px(p.column.0 as f32 * cw), o.y + px(row as f32 * ch)),
            size(px(cw), px(ch)),
        )
    }

    /// Only with the kitty protocol's event types, and only for keys whose press went out.
    fn on_key_up(&mut self, ev: &KeyUpEvent, _: &mut Window, _: &mut Context<Self>) {
        let k = &ev.keystroke;
        if !self.pressed.remove(&k.key) {
            return;
        }
        if let Some(bytes) = keys::encode(k, self.mode(), KeyAction::Release) {
            self.send(&bytes);
        }
    }

    fn paste(&self, item: Option<ClipboardItem>) {
        if let Some(text) = item.and_then(|i| i.text()) {
            self.send(&keys::paste(&text, self.mode()));
        }
    }

    /// Window position to a grid point (accounting for scrollback) and the cell side.
    fn grid_point(&self, pos: Point<Pixels>) -> (GridPoint, Side, (usize, usize)) {
        let o = self.origin.get();
        let (cols, rows) = self.size.get();
        let x = f32::from(pos.x - o.x).max(0.);
        let y = f32::from(pos.y - o.y).max(0.);
        let col = ((x / self.metrics.cell_w) as usize).min(cols as usize - 1);
        let row = ((y / self.metrics.cell_h) as usize).min(rows as usize - 1);
        let side = if x % self.metrics.cell_w > self.metrics.cell_w / 2. {
            Side::Right
        } else {
            Side::Left
        };
        let offset = self.shared.term.lock().grid().display_offset() as i32;
        (
            GridPoint::new(Line(row as i32 - offset), Column(col)),
            side,
            (col, row),
        )
    }

    /// Mouse reporting to the program, xterm style: SGR when it asked for it.
    fn report_mouse(&self, button: u8, pressed: bool, cell: (usize, usize), mods: gpui::Modifiers) {
        let mode = self.mode();
        let mut b = button;
        if mods.shift {
            b += 4;
        }
        if mods.alt {
            b += 8;
        }
        if mods.control {
            b += 16;
        }
        let (x, y) = (cell.0 + 1, cell.1 + 1);
        if mode.contains(TermMode::SGR_MOUSE) {
            let c = if pressed { 'M' } else { 'm' };
            self.send(format!("\x1b[<{b};{x};{y}{c}").as_bytes());
        } else {
            let b = if pressed { b } else { 3 };
            let enc = |v: usize| (32 + v).min(255) as u8;
            self.send(&[0x1b, b'[', b'M', 32 + b, enc(x), enc(y)]);
        }
    }

    fn mouse_mode(&self, mods: &gpui::Modifiers) -> bool {
        !mods.shift && self.mode().intersects(TermMode::MOUSE_MODE)
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        let (p, side, cell) = self.grid_point(ev.position);
        if self.mouse_mode(&ev.modifiers) {
            let button = match ev.button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                _ => return,
            };
            self.report_mouse(button, true, cell, ev.modifiers);
            return;
        }
        if ev.button == MouseButton::Left {
            let ty = match ev.click_count {
                2 => SelectionType::Semantic,
                3.. => SelectionType::Lines,
                _ => SelectionType::Simple,
            };
            self.shared.term.lock().selection = Some(Selection::new(ty, p, side));
            self.selecting = true;
            cx.notify();
        }
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (p, side, cell) = self.grid_point(ev.position);
        if self.mouse_mode(&ev.modifiers) {
            let mode = self.mode();
            let button = match ev.pressed_button {
                Some(MouseButton::Left) => 0,
                Some(MouseButton::Middle) => 1,
                Some(MouseButton::Right) => 2,
                _ => 3,
            };
            let report = if button == 3 {
                mode.contains(TermMode::MOUSE_MOTION)
            } else {
                mode.intersects(TermMode::MOUSE_MOTION | TermMode::MOUSE_DRAG)
            };
            if report {
                self.report_mouse(32 + button, true, cell, ev.modifiers);
            }
            return;
        }
        if self.selecting && ev.pressed_button == Some(MouseButton::Left) {
            if let Some(sel) = self.shared.term.lock().selection.as_mut() {
                sel.update(p, side);
            }
            cx.notify();
        }
        let link = self.link_at(p);
        if link != self.hover_link {
            self.hover_link = link;
            cx.notify();
        }
    }

    /// The OSC 8 link at a grid point.
    fn link_at(&self, p: GridPoint) -> Option<Hyperlink> {
        self.shared.term.lock().grid()[p].hyperlink()
    }

    fn on_mouse_up(&mut self, ev: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (p, _, cell) = self.grid_point(ev.position);
        if self.mouse_mode(&ev.modifiers) {
            let button = match ev.button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                _ => return,
            };
            self.report_mouse(button, false, cell, ev.modifiers);
            return;
        }
        if self.selecting {
            self.selecting = false;
            let clicked = {
                let mut term = self.shared.term.lock();
                let empty = term.selection_to_string().is_none_or(|t| t.is_empty());
                if empty {
                    term.selection = None;
                }
                empty
            };
            // A click without a drag opens the link under it
            if clicked && let Some(link) = self.link_at(p) {
                cx.open_url(link.uri());
            }
            cx.notify();
        }
    }

    fn on_scroll(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ch = self.metrics.cell_h;
        self.scroll_acc += f32::from(ev.delta.pixel_delta(px(ch)).y) / ch;
        let lines = self.scroll_acc.trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_acc -= lines as f32;
        let mode = self.mode();
        if self.mouse_mode(&ev.modifiers) {
            let (_, _, cell) = self.grid_point(ev.position);
            let button = if lines > 0 { 64 } else { 65 };
            for _ in 0..lines.abs() {
                self.report_mouse(button, true, cell, ev.modifiers);
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let app = mode.contains(TermMode::APP_CURSOR);
            let seq: &[u8] = match (lines > 0, app) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            self.send(&seq.repeat(lines.unsigned_abs() as usize));
        } else {
            self.shared.term.lock().scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }
}

impl TerminalView {
    /// The program's title (OSC 0/2), or `nerdr — <address>` until it sets one.
    pub fn title(&self) -> &str {
        &self.title
    }

    fn refresh_title(&mut self) {
        if let Some(t) = self.shared.title.lock().unwrap().take() {
            self.title = t;
        }
    }
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_title();
        let exited = self.shared.exited.lock().unwrap().clone();
        let title = match &exited {
            Some(why) => format!("{} [{why}]", crate::TITLE),
            None => crate::TITLE.to_string(),
        };
        // Render runs on every output batch; setting the title each time makes macOS redraw the title bar
        if *self.window_title.borrow() != title {
            window.set_window_title(&title);
            *self.window_title.borrow_mut() = title;
        }

        let shared = self.shared.clone();
        let metrics = self.metrics.clone();
        let size_cell = self.size.clone();
        let origin = self.origin.clone();
        let stream = self.stream.clone();
        let focused = self.focus.is_focused(window);
        let hide_cursor = focused && self.blink_off;
        let hover_link = self.hover_link.clone();
        let marked = self.marked.clone();
        let input = (cx.entity(), self.focus.clone());

        div()
            .id("terminal")
            .size_full()
            .p(px(PAD))
            .bg(hsla(theme::BG))
            .when(hover_link.is_some(), |d| d.cursor_pointer())
            .track_focus(&self.focus)
            .key_context("Terminal")
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .child(
                canvas(
                    move |bounds, window, _cx| {
                        let (cols, rows) = metrics.grid_size(
                            f32::from(bounds.size.width) + 2. * PAD,
                            f32::from(bounds.size.height) + 2. * PAD,
                        );
                        origin.set(bounds.origin);
                        if size_cell.get() != (cols, rows) {
                            size_cell.set((cols, rows));
                            shared.term.lock().resize(TermSize {
                                cols: cols as usize,
                                rows: rows as usize,
                            });
                            stream.resize(cols, rows);
                        }
                        let state = FrameState {
                            focused,
                            hide_cursor: hide_cursor || marked.is_some(),
                            hover_link: hover_link.as_ref(),
                            marked: marked.as_deref(),
                        };
                        build_frame(&shared, &metrics, bounds, state, window)
                    },
                    move |_, frame, window, cx| {
                        let (view, focus) = input;
                        window.handle_input(&focus, TermInput(view), cx);
                        frame.paint(window, cx)
                    },
                )
                .size_full(),
            )
    }
}

/// The terminal's end of its hc stream: parses output into the term on the reader thread.
struct TermSink {
    shared: Arc<Shared>,
    parser: Mutex<ansi::Processor>,
    trace: Option<Mutex<std::fs::File>>,
    stats: Option<Mutex<Stats>>,
    wake: UnboundedSender<()>,
}

impl TermSink {
    fn notify(&self) {
        if !self.shared.dirty.swap(true, Ordering::AcqRel) {
            let _ = self.wake.unbounded_send(());
        }
    }
}

impl hc::Sink for TermSink {
    fn attached(&self, result: &serde_json::Value) {
        self.shared.ended.store(false, Ordering::Release);
        let again = self.shared.exited.lock().unwrap().take().is_some();
        if again {
            // A new paint replays the screen and history: start from an empty term so
            // the history isn't doubled.
            eprintln!("nerdr: attached again on channel {}", result["channel"]);
            let mut parser = self.parser.lock().unwrap();
            *parser = ansi::Processor::new();
            let mut term = self.shared.term.lock();
            parser.advance(&mut *term, b"\x1bc");
            term.grid_mut().clear_history();
        }
        self.notify();
    }

    fn data(&self, payloads: &[&[u8]]) {
        let mut parser = self.parser.lock().unwrap();
        let mut n = 0;
        {
            let mut term = self.shared.term.lock();
            for p in payloads {
                if let Some(t) = &self.trace {
                    use std::io::Write;
                    let _ = t.lock().unwrap().write_all(p);
                }
                parser.advance(&mut *term, p);
                n += p.len();
            }
        }
        if let Some(s) = &self.stats {
            s.lock().unwrap().add(n, payloads.len());
        }
        self.notify();
    }

    fn closed(&self, why: hc::Closed) {
        let why = match why {
            hc::Closed::Ended(w) => {
                eprintln!("nerdr: stream closed: {w}");
                self.shared.ended.store(true, Ordering::Release);
                w
            }
            hc::Closed::Lost(_) => "reconnecting…".to_string(),
        };
        *self.shared.exited.lock().unwrap() = Some(why);
        self.notify();
    }
}

/// Throughput, printed with NERDR_DEBUG when a burst ends.
#[derive(Default)]
struct Stats {
    start: Option<Instant>,
    last: Option<Instant>,
    bytes: usize,
    frames: usize,
}

impl Stats {
    fn add(&mut self, n: usize, frames: usize) {
        let now = Instant::now();
        if let Some(last) = self.last
            && now.duration_since(last).as_millis() > 500
            && let Some(start) = self.start
            && self.bytes > 100_000
        {
            let secs = last.duration_since(start).as_secs_f64().max(1e-6);
            eprintln!(
                "nerdr: burst {} KiB in {} frames, {:.0} ms, {:.1} MiB/s",
                self.bytes / 1024,
                self.frames,
                secs * 1000.,
                self.bytes as f64 / secs / 1048576.
            );
            self.start = None;
        }
        if self.start.is_none() || self.last.is_some_and(|l| now.duration_since(l).as_millis() > 500) {
            self.start = Some(now);
            self.bytes = 0;
            self.frames = 0;
        }
        self.bytes += n;
        self.frames += frames;
        self.last = Some(now);
    }
}

/// Everything one paint needs, built in prepaint.
struct Frame {
    rects: Vec<(Bounds<Pixels>, Hsla)>,
    paths: Vec<(gpui::Path<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    cursor: Option<(Bounds<Pixels>, Hsla, bool)>,
    /// IME text on top of everything: its background and where it goes.
    composing: Option<(Bounds<Pixels>, Point<Pixels>, ShapedLine)>,
    line_height: Pixels,
}

impl Frame {
    fn paint(self, window: &mut Window, cx: &mut App) {
        for (b, c) in self.rects {
            window.paint_quad(fill(b, c));
        }
        for (path, c) in self.paths {
            window.paint_path(path, c);
        }
        if let Some((b, c, hollow)) = &self.cursor
            && !hollow
        {
            window.paint_quad(fill(*b, *c));
        }
        for (origin, line) in &self.lines {
            let _ = line.paint(*origin, self.line_height, window, cx);
        }
        if let Some((b, c, true)) = self.cursor {
            window.paint_quad(outline(b, c, gpui::BorderStyle::Solid));
        }
        if let Some((bg, origin, line)) = &self.composing {
            window.paint_quad(fill(*bg, hsla(theme::BG)));
            let _ = line.paint(*origin, self.line_height, window, cx);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Rgb,
    bold: bool,
    italic: bool,
}

/// A piece of a row shaped as one line: narrow cells next to each other, or one wide cell.
struct Segment {
    row: usize,
    col: usize,
    text: String,
    runs: Vec<(usize, Style)>,
    /// Bytes up to the last non-space, so trailing blanks aren't shaped.
    used: usize,
    cells: usize,
    wide: bool,
}

/// What the view adds to the terminal's own state when drawing.
struct FrameState<'a> {
    focused: bool,
    hide_cursor: bool,
    hover_link: Option<&'a Hyperlink>,
    marked: Option<&'a str>,
}

fn build_frame(
    shared: &Shared,
    m: &Metrics,
    bounds: Bounds<Pixels>,
    state: FrameState,
    window: &mut Window,
) -> Frame {
    let FrameState {
        focused,
        hide_cursor,
        hover_link,
        marked,
    } = state;
    let (cw, ch) = (m.cell_w, m.cell_h);
    let o = bounds.origin;
    let cell_rect = |col: usize, row: usize, w: usize| {
        Bounds::new(
            point(o.x + px(col as f32 * cw), o.y + px(row as f32 * ch)),
            size(px(w as f32 * cw), px(ch)),
        )
    };
    let mut rects: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
    let mut paths = Vec::new();
    let mut segments: Vec<Segment> = Vec::new();
    let mut cursor = None;
    let cursor_cell;
    {
        let term = shared.term.lock();
        let content = term.renderable_content();
        let colors = content.colors;
        let offset = content.display_offset as i32;
        let cur = content.cursor;
        cursor_cell = (cur.point.column.0, (cur.point.line.0 + offset) as usize);
        let cursor_color = colors[NamedColor::Cursor].unwrap_or(theme::CURSOR);
        let default_bg = colors[NamedColor::Background].unwrap_or(theme::BG);
        let selection = content.selection;
        // A background run waiting to be merged with the next cell: (row, start col, end col, color)
        let mut bg_run: Option<(usize, usize, usize, Rgb)> = None;
        let flush_bg = |run: Option<(usize, usize, usize, Rgb)>, rects: &mut Vec<_>| {
            if let Some((row, a, b, c)) = run {
                rects.push((cell_rect(a, row, b - a), hsla(c)));
            }
        };

        for cell in content.display_iter {
            let flags = cell.flags;
            if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let row = (cell.point.line.0 + offset) as usize;
            let col = cell.point.column.0;
            let width = if flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };

            let mut fg = theme::resolve(cell.fg, colors);
            let mut bg = theme::resolve(cell.bg, colors);
            if flags.contains(Flags::DIM) {
                fg = theme::dim(fg, default_bg);
            }
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if flags.contains(Flags::HIDDEN) {
                fg = bg;
            }
            if selection.is_some_and(|s| s.contains(cell.point)) {
                fg = theme::SELECTION_FG;
                bg = theme::SELECTION;
            }
            let is_cursor =
                cell.point == cur.point && cur.shape != CursorShape::Hidden && !hide_cursor;
            if is_cursor {
                let w = width;
                let r = cell_rect(col, row, w);
                cursor = Some(match (focused, cur.shape) {
                    (true, CursorShape::Block) => {
                        fg = bg;
                        (r, hsla(cursor_color), false)
                    }
                    (true, CursorShape::Beam) => (
                        Bounds::new(r.origin, size(px(2.), r.size.height)),
                        hsla(cursor_color),
                        false,
                    ),
                    (true, CursorShape::Underline) => (
                        Bounds::new(
                            point(r.origin.x, r.origin.y + r.size.height - px(2.)),
                            size(r.size.width, px(2.)),
                        ),
                        hsla(cursor_color),
                        false,
                    ),
                    _ => (r, hsla(cursor_color), true),
                });
            }

            // Backgrounds, merged along the row
            if bg != default_bg {
                match &mut bg_run {
                    Some((r, _, end, c)) if *r == row && *end == col && *c == bg => *end += width,
                    _ => {
                        flush_bg(bg_run.take(), &mut rects);
                        bg_run = Some((row, col, col + width, bg));
                    }
                }
            } else {
                flush_bg(bg_run.take(), &mut rects);
            }
            let hovered = hover_link.is_some() && cell.hyperlink().as_ref() == hover_link;
            if flags.intersects(Flags::ALL_UNDERLINES) || hovered {
                let r = cell_rect(col, row, width);
                let y = r.origin.y + r.size.height - px(2.);
                rects.push((
                    Bounds::new(point(r.origin.x, y), size(r.size.width, px(1.))),
                    hsla(fg),
                ));
            }
            if flags.contains(Flags::STRIKEOUT) {
                let r = cell_rect(col, row, width);
                let y = r.origin.y + r.size.height / 2.;
                rects.push((
                    Bounds::new(point(r.origin.x, y), size(r.size.width, px(1.))),
                    hsla(fg),
                ));
            }

            // Text
            let style = Style {
                fg,
                bold: flags.contains(Flags::BOLD),
                italic: flags.contains(Flags::ITALIC),
            };
            let mut c = if cell.c == '\t' { ' ' } else { cell.c };
            // Block elements are drawn as rectangles so they meet across rows and columns
            if let Some(parts) = block_element(c) {
                let r = cell_rect(col, row, width);
                for &(x0, y0, x1, y1, alpha) in parts {
                    let (w, h) = (r.size.width, r.size.height);
                    let b = Bounds::new(
                        point(r.origin.x + w * x0, r.origin.y + h * y0),
                        size(w * (x1 - x0), h * (y1 - y0)),
                    );
                    rects.push((b, hsla(fg).opacity(alpha)));
                }
                c = ' ';
            } else if let Some(path) = rounded_corner(cell_rect(col, row, width), c) {
                paths.push((path, hsla(fg)));
                c = ' ';
            } else if let Some(arms) = box_arms(c) {
                for b in box_rects(cell_rect(col, row, width), arms) {
                    rects.push((b, hsla(fg)));
                }
                c = ' ';
            }
            let new_segment = width == 2
                || !segments.last().is_some_and(|s| {
                    s.row == row
                        && s.col + s.cells == col
                        && !s.wide
                        // GPUI 0.2.2's layout_line merges neighbouring runs of the same color
                        // even when their fonts differ, so a font change starts a new segment.
                        && s.runs.last().is_none_or(|(_, r)| {
                            (r.bold, r.italic) == (style.bold, style.italic)
                        })
                });
            if new_segment {
                segments.push(Segment {
                    row,
                    col,
                    text: String::new(),
                    runs: Vec::new(),
                    used: 0,
                    cells: 0,
                    wide: width == 2,
                });
            }
            let seg = segments.last_mut().unwrap();
            let before = seg.text.len();
            seg.text.push(c);
            if let Some(zw) = cell.zerowidth() {
                seg.text.extend(zw.iter());
            }
            let added = seg.text.len() - before;
            match seg.runs.last_mut() {
                Some((len, s)) if *s == style => *len += added,
                _ => seg.runs.push((added, style)),
            }
            if c != ' ' {
                seg.used = seg.text.len();
            }
            seg.cells += width;
        }
        flush_bg(bg_run.take(), &mut rects);
    }

    // Shape outside the lock, so the reader isn't held up
    let ts = window.text_system().clone();
    let mut lines = Vec::with_capacity(segments.len());
    for seg in segments {
        if seg.used == 0 {
            continue;
        }
        let mut text = seg.text;
        text.truncate(seg.used);
        let mut left = seg.used;
        let runs: Vec<TextRun> = seg
            .runs
            .iter()
            .filter_map(|&(len, s)| {
                let len = len.min(left);
                left -= len;
                (len > 0).then(|| TextRun {
                    len,
                    font: Font {
                        weight: if s.bold { FontWeight::BOLD } else { FontWeight::NORMAL },
                        style: if s.italic { FontStyle::Italic } else { FontStyle::Normal },
                        ..m.font.clone()
                    },
                    color: hsla(s.fg),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                })
            })
            .collect();
        let shaped = ts.shape_line(SharedString::from(text), m.font_size, &runs, Some(px(cw)));
        let origin = point(
            o.x + px(seg.col as f32 * cw),
            o.y + px(seg.row as f32 * ch),
        );
        lines.push((origin, shaped));
    }
    let mut composing = None;
    if let Some(text) = marked {
        let run = TextRun {
            len: text.len(),
            font: m.font.clone(),
            color: hsla(theme::FG),
            background_color: None,
            underline: Some(gpui::UnderlineStyle {
                thickness: px(1.),
                color: Some(hsla(theme::FG)),
                wavy: false,
            }),
            strikethrough: None,
        };
        let shaped = ts.shape_line(SharedString::from(text.to_string()), m.font_size, &[run], None);
        let origin = cell_rect(cursor_cell.0, cursor_cell.1, 1).origin;
        let bg = Bounds::new(origin, size(shaped.width, px(ch)));
        composing = Some((bg, origin, shaped));
    }
    Frame {
        rects,
        paths,
        lines,
        cursor,
        composing,
        line_height: px(ch),
    }
}

/// Box drawing (U+2500–257F, light and heavy lines) as arm weights: up, down, left, right.
/// 0 none, 1 light, 2 heavy.
fn box_arms(c: char) -> Option<[u8; 4]> {
    Some(match c {
        '─' => [0, 0, 1, 1],
        '━' => [0, 0, 2, 2],
        '│' => [1, 1, 0, 0],
        '┃' => [2, 2, 0, 0],
        '┌' => [0, 1, 0, 1],
        '┍' => [0, 1, 0, 2],
        '┎' => [0, 2, 0, 1],
        '┏' => [0, 2, 0, 2],
        '┐' => [0, 1, 1, 0],
        '┑' => [0, 1, 2, 0],
        '┒' => [0, 2, 1, 0],
        '┓' => [0, 2, 2, 0],
        '└' => [1, 0, 0, 1],
        '┕' => [1, 0, 0, 2],
        '┖' => [2, 0, 0, 1],
        '┗' => [2, 0, 0, 2],
        '┘' => [1, 0, 1, 0],
        '┙' => [1, 0, 2, 0],
        '┚' => [2, 0, 1, 0],
        '┛' => [2, 0, 2, 0],
        '├' => [1, 1, 0, 1],
        '┝' => [1, 1, 0, 2],
        '┞' => [2, 1, 0, 1],
        '┟' => [1, 2, 0, 1],
        '┠' => [2, 2, 0, 1],
        '┡' => [2, 1, 0, 2],
        '┢' => [1, 2, 0, 2],
        '┣' => [2, 2, 0, 2],
        '┤' => [1, 1, 1, 0],
        '┥' => [1, 1, 2, 0],
        '┦' => [2, 1, 1, 0],
        '┧' => [1, 2, 1, 0],
        '┨' => [2, 2, 1, 0],
        '┩' => [2, 1, 2, 0],
        '┪' => [1, 2, 2, 0],
        '┫' => [2, 2, 2, 0],
        '┬' => [0, 1, 1, 1],
        '┭' => [0, 1, 2, 1],
        '┮' => [0, 1, 1, 2],
        '┯' => [0, 1, 2, 2],
        '┰' => [0, 2, 1, 1],
        '┱' => [0, 2, 2, 1],
        '┲' => [0, 2, 1, 2],
        '┳' => [0, 2, 2, 2],
        '┴' => [1, 0, 1, 1],
        '┵' => [1, 0, 2, 1],
        '┶' => [1, 0, 1, 2],
        '┷' => [1, 0, 2, 2],
        '┸' => [2, 0, 1, 1],
        '┹' => [2, 0, 2, 1],
        '┺' => [2, 0, 1, 2],
        '┻' => [2, 0, 2, 2],
        '┼' => [1, 1, 1, 1],
        '┽' => [1, 1, 2, 1],
        '┾' => [1, 1, 1, 2],
        '┿' => [1, 1, 2, 2],
        '╀' => [2, 1, 1, 1],
        '╁' => [1, 2, 1, 1],
        '╂' => [2, 2, 1, 1],
        '╃' => [2, 1, 2, 1],
        '╄' => [2, 1, 1, 2],
        '╅' => [1, 2, 2, 1],
        '╆' => [1, 2, 1, 2],
        '╇' => [2, 1, 2, 2],
        '╈' => [1, 2, 2, 2],
        '╉' => [2, 2, 2, 1],
        '╊' => [2, 2, 1, 2],
        '╋' => [2, 2, 2, 2],
        '╴' => [0, 0, 1, 0],
        '╵' => [1, 0, 0, 0],
        '╶' => [0, 0, 0, 1],
        '╷' => [0, 1, 0, 0],
        '╸' => [0, 0, 2, 0],
        '╹' => [2, 0, 0, 0],
        '╺' => [0, 0, 0, 2],
        '╻' => [0, 2, 0, 0],
        '╼' => [0, 0, 1, 2],
        '╽' => [1, 2, 0, 0],
        '╾' => [0, 0, 2, 1],
        '╿' => [2, 1, 0, 0],
        _ => return None,
    })
}

/// Rectangles for a box-drawing cell. The arms reach the cell edges, so lines meet
/// across rows whatever the line height.
fn box_rects(r: Bounds<Pixels>, [up, down, left, right]: [u8; 4]) -> Vec<Bounds<Pixels>> {
    let thick = |w: u8| px(w as f32);
    let (x, y, w, h) = (r.origin.x, r.origin.y, r.size.width, r.size.height);
    // Whole pixels from the cell edge keep the lines sharp
    let cx = x + ((w - thick(up.max(down))) / 2.).floor();
    let cy = y + ((h - thick(left.max(right))) / 2.).floor();
    // Arms run past the center to cover the joint
    let over_v = thick(left.max(right));
    let over_h = thick(up.max(down));
    let mut out = Vec::new();
    if up > 0 {
        out.push(Bounds::new(point(cx, y), size(thick(up), cy - y + over_v)));
    }
    if down > 0 {
        out.push(Bounds::new(point(cx, cy), size(thick(down), y + h - cy)));
    }
    if left > 0 {
        out.push(Bounds::new(point(x, cy), size(cx - x + over_h, thick(left))));
    }
    if right > 0 {
        out.push(Bounds::new(point(cx, cy), size(x + w - cx, thick(right))));
    }
    out
}

/// ╭╮╯╰ as a light line that bends from one cell edge to the other, centered like `box_rects`.
fn rounded_corner(r: Bounds<Pixels>, c: char) -> Option<gpui::Path<Pixels>> {
    let (x, y, w, h) = (r.origin.x, r.origin.y, r.size.width, r.size.height);
    let cx = x + ((w - px(1.)) / 2.).floor() + px(0.5);
    let cy = y + ((h - px(1.)) / 2.).floor() + px(0.5);
    // Where the line leaves the cell vertically and horizontally
    let (vy, hx) = match c {
        '╭' => (y + h, x + w),
        '╮' => (y + h, x),
        '╯' => (y, x),
        '╰' => (y, x + w),
        _ => return None,
    };
    let radius = cx - x;
    let toward = |from: Pixels, to: Pixels| if to > from { from + radius } else { from - radius };
    let mut b = gpui::PathBuilder::stroke(px(1.));
    b.move_to(point(cx, vy));
    b.line_to(point(cx, toward(cy, vy)));
    b.curve_to(point(toward(cx, hx), cy), point(cx, cy));
    b.line_to(point(hx, cy));
    b.build().ok()
}

/// U+2580–U+259F as rectangles in cell fractions: (x0, y0, x1, y1, alpha).
fn block_element(c: char) -> Option<&'static [(f32, f32, f32, f32, f32)]> {
    const E: f32 = 1. / 8.;
    Some(match c {
        '▀' => &[(0., 0., 1., 0.5, 1.)],
        '▁' => &[(0., 7. * E, 1., 1., 1.)],
        '▂' => &[(0., 6. * E, 1., 1., 1.)],
        '▃' => &[(0., 5. * E, 1., 1., 1.)],
        '▄' => &[(0., 4. * E, 1., 1., 1.)],
        '▅' => &[(0., 3. * E, 1., 1., 1.)],
        '▆' => &[(0., 2. * E, 1., 1., 1.)],
        '▇' => &[(0., E, 1., 1., 1.)],
        '█' => &[(0., 0., 1., 1., 1.)],
        '▉' => &[(0., 0., 7. * E, 1., 1.)],
        '▊' => &[(0., 0., 6. * E, 1., 1.)],
        '▋' => &[(0., 0., 5. * E, 1., 1.)],
        '▌' => &[(0., 0., 4. * E, 1., 1.)],
        '▍' => &[(0., 0., 3. * E, 1., 1.)],
        '▎' => &[(0., 0., 2. * E, 1., 1.)],
        '▏' => &[(0., 0., E, 1., 1.)],
        '▐' => &[(0.5, 0., 1., 1., 1.)],
        '░' => &[(0., 0., 1., 1., 0.25)],
        '▒' => &[(0., 0., 1., 1., 0.5)],
        '▓' => &[(0., 0., 1., 1., 0.75)],
        '▔' => &[(0., 0., 1., E, 1.)],
        '▕' => &[(7. * E, 0., 1., 1., 1.)],
        '▖' => &[(0., 0.5, 0.5, 1., 1.)],
        '▗' => &[(0.5, 0.5, 1., 1., 1.)],
        '▘' => &[(0., 0., 0.5, 0.5, 1.)],
        '▙' => &[(0., 0., 0.5, 1., 1.), (0.5, 0.5, 1., 1., 1.)],
        '▚' => &[(0., 0., 0.5, 0.5, 1.), (0.5, 0.5, 1., 1., 1.)],
        '▛' => &[(0., 0., 1., 0.5, 1.), (0., 0.5, 0.5, 1., 1.)],
        '▜' => &[(0., 0., 1., 0.5, 1.), (0.5, 0.5, 1., 1., 1.)],
        '▝' => &[(0.5, 0., 1., 0.5, 1.)],
        '▞' => &[(0.5, 0., 1., 0.5, 1.), (0., 0.5, 0.5, 1., 1.)],
        '▟' => &[(0.5, 0., 1., 1., 1.), (0., 0.5, 0.5, 1., 1.)],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_lines_reach_the_cell_edges() {
        let cell = Bounds::new(point(px(0.), px(0.)), size(px(10.), px(25.)));
        let covers = |rects: &[Bounds<Pixels>], p: Point<Pixels>| rects.iter().any(|r| r.contains(&p));
        let v = box_rects(cell, box_arms('│').unwrap());
        assert!(covers(&v, point(px(4.5), px(0.))) && covers(&v, point(px(4.5), px(24.9))));
        let corner = box_rects(cell, box_arms('┌').unwrap());
        assert!(covers(&corner, point(px(9.9), px(12.5))) && covers(&corner, point(px(4.5), px(24.9))));
        assert!(!covers(&corner, point(px(4.5), px(0.))));
    }
}
