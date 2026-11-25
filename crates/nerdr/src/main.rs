//! nerdr: a GPUI window of hc terminals, in tabs grouped into tasks.
//!
//! `nerdr [TERMINAL]` opens the saved layout, plus that terminal (an address or a name) as
//! a tab. With nothing saved and no terminal given, it starts a shell.

mod hc;
mod ime;
mod keys;
mod panes;
mod prefix;
mod repos;
mod seen;
mod tasks;
mod theme;
mod view;
mod workspace;
mod worktree;

use std::borrow::Cow;

use anyhow::Result;
use gpui::{
    App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size,
};
use futures::StreamExt;

use crate::view::Metrics;
use crate::workspace::Workspace;

const FONTS: [&[u8]; 4] = [
    include_bytes!("../fonts/JetBrainsMono-Regular.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Italic.ttf"),
    include_bytes!("../fonts/JetBrainsMono-Bold.ttf"),
    include_bytes!("../fonts/JetBrainsMono-BoldItalic.ttf"),
];

pub const TITLE: &str = concat!("nerdr-app v", env!("CARGO_PKG_VERSION"));

fn main() {
    let address = std::env::args().nth(1);
    let font_size: f32 = std::env::var("NERDR_FONT_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(16.);

    Application::new().run(move |cx: &mut App| {
        cx.text_system()
            .add_fonts(FONTS.map(Cow::Borrowed).to_vec())
            .expect("the built-in font should load");
        let bounds = Bounds::centered(None, size(px(1000.), px(680.)), cx);
        let opened = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Maximized(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(TITLE.into()),
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
                let display = workspace::display(&metrics, window);
                let started = connect().and_then(|client| {
                    let (layout, version, info) =
                        workspace::load(&client, address.as_deref(), cols, rows, &display)?;
                    Ok((client, layout, version, info))
                });
                let (client, layout, version, info) = started.unwrap_or_else(|e| {
                    eprintln!("nerdr: {e:#}");
                    std::process::exit(1);
                });
                let ws = cx.new(|cx| {
                    Workspace::new(client, layout, version, info, metrics, display, window, cx)
                });
                ws.update(cx, |ws, cx| ws.focus_active(window, cx));
                ws
            },
        );
        opened.expect("could not open window");
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}

fn connect() -> Result<hc::Client> {
    let client = hc::Client::connect()?;
    if std::env::var_os("NERDR_DEBUG").is_some() {
        let mut notices = client.notices();
        std::thread::spawn(move || {
            while let Some(n) = futures::executor::block_on(notices.next()) {
                match n {
                    hc::Notice::Snapshot { terminals, .. } => {
                        eprintln!("nerdr: snapshot, {} terminals", terminals.len())
                    }
                    n => eprintln!("nerdr: {n:?}"),
                }
            }
        });
    }
    Ok(client)
}
