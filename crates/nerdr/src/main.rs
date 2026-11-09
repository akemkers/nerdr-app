//! nerdr spike: one GPUI window with one hc terminal in it.
//!
//! `nerdr [ADDRESS]` attaches to that terminal. Without one it reuses the live terminal
//! labelled `nerdr.spike=1`, or starts a new shell with that label.

mod hc;
mod keys;
mod theme;
mod view;

use std::borrow::Cow;

use anyhow::{Context as _, Result};
use gpui::{
    App, AppContext, Application, Bounds, Focusable, WindowBounds, WindowOptions, px, size,
};
use serde_json::{Value, json};

use crate::view::{Attached, Metrics, TerminalView};

const FONTS: [&[u8]; 4] = [
    include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../fonts/JetBrainsMono-BoldItalic.ttf"),
];

const SPIKE_LABEL: &str = "nerdr.spike";

fn main() {
    let address = std::env::args().nth(1);
    let font_size: f32 = std::env::var("NERDR_FONT_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(14.);

    Application::new().run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(FONTS.map(Cow::Borrowed).to_vec())
            .expect("the built-in font should load");
        let bounds = Bounds::centered(None, size(px(1000.), px(680.)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("nerdr".into()),
                    ..Default::default()
                }),
                app_id: Some("nerdr".into()),
                focus: true,
                ..Default::default()
            },
            move |window, cx| {
                let metrics = Metrics::new(font_size, window);
                let vp = window.viewport_size();
                let (cols, rows) = metrics.grid_size(f32::from(vp.width), f32::from(vp.height));
                let attached = attach(address.as_deref(), cols, rows, &metrics, window)
                    .unwrap_or_else(|e| {
                        eprintln!("nerdr: {e:#}");
                        std::process::exit(1);
                    });
                let view = cx.new(|cx| TerminalView::new(attached, cols, rows, metrics, window, cx));
                window.focus(&view.focus_handle(cx));
                view
            },
        );
        opened.expect("could not open window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}

/// Our colors and cell size, so the core answers the program's display queries like we look.
fn display(m: &Metrics, window: &gpui::Window) -> Value {
    let s = window.scale_factor();
    json!({
        "fg": theme::hex_str(theme::FG),
        "bg": theme::hex_str(theme::BG),
        "cursor": theme::hex_str(theme::CURSOR),
        "cell": [(m.cell_w * s).round() as u32, (m.cell_h * s).round() as u32],
    })
}

fn attach(
    address: Option<&str>,
    cols: u16,
    rows: u16,
    m: &Metrics,
    window: &gpui::Window,
) -> Result<Attached> {
    let mut conn = hc::Conn::connect()?;
    let display = display(m, window);
    let address = match address {
        Some(a) => a.to_string(),
        None => {
            let list = conn.call(
                "terminal.list",
                json!({"labels": {SPIKE_LABEL: "1"}}),
            )?;
            let live = list["terminals"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|t| t["state"] == "live")
                .and_then(|t| t["address"].as_str());
            match live {
                Some(a) => {
                    eprintln!("nerdr: reusing {a}");
                    a.to_string()
                }
                None => {
                    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
                    let run = conn.call(
                        "terminal.run",
                        json!({
                            "labels": {SPIKE_LABEL: "1"},
                            // Not a child of whatever terminal launched us: it should outlive us
                            "no_parent": true,
                            "no_notify": true,
                            "cwd": home,
                            "size": {"cols": cols, "rows": rows},
                            "display": display,
                            "env": {"COLORTERM": "truecolor", "TERM_PROGRAM": "nerdr"},
                        }),
                    )?;
                    let a = run["address"].as_str().context("terminal.run: no address")?;
                    eprintln!("nerdr: started {a}");
                    a.to_string()
                }
            }
        }
    };
    let res = conn.call(
        "terminal.attach",
        json!({
            "terminal": address,
            "cols": cols,
            "rows": rows,
            "display": display,
            // Scrollback replayed ahead of the screen; it scrolls into alacritty's history
            "history": 5000,
        }),
    )?;
    let channel = res["channel"].as_u64().context("attach: no channel")? as u32;
    eprintln!(
        "nerdr: attached {address} on channel {channel} (resize_notifications={}, automatic_input={})",
        res["resize_notifications"], res["automatic_input"]
    );
    Ok(Attached {
        conn,
        channel,
        address,
    })
}
