//! Colors: a fixed dark palette (Catppuccin Mocha, as in kitty) and alacritty's color model mapped onto it.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui::Hsla;

pub const FG: Rgb = rgb(0xcdd6f4);
pub const BG: Rgb = rgb(0x181825);
pub const CURSOR: Rgb = rgb(0xf5e0dc);
pub const SELECTION: Rgb = rgb(0xf5e0dc);
pub const SELECTION_FG: Rgb = rgb(0x1e1e2e);

pub const SIDEBAR_BG: Rgb = rgb(0x11111b);
pub const SIDEBAR_SELECTED: Rgb = rgb(0x313244);
pub const MUTED: Rgb = rgb(0x7f849c);
/// Lines between panes; the ones next to the active pane are YELLOW, as in the user's tmux.
pub const PANE_BORDER: Rgb = rgb(0x585b70);

pub const YELLOW: Rgb = rgb(0xf9e2af);
pub const RED: Rgb = rgb(0xf38ba8);
pub const GREEN: Rgb = rgb(0xa6e3a1);

const ANSI: [u32; 16] = [
    0x45475a, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xbac2de, //
    0x585b70, 0xf38ba8, 0xa6e3a1, 0xf9e2af, 0x89b4fa, 0xf5c2e7, 0x94e2d5, 0xa6adc8,
];

const fn rgb(v: u32) -> Rgb {
    Rgb {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    }
}

pub fn hex_str(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// The xterm 256-color palette with our first 16.
fn indexed_default(i: usize) -> Rgb {
    match i {
        0..16 => rgb(ANSI[i]),
        16..232 => {
            let i = i - 16;
            let step = |v: usize| if v == 0 { 0 } else { (v * 40 + 55) as u8 };
            Rgb {
                r: step(i / 36),
                g: step((i / 6) % 6),
                b: step(i % 6),
            }
        }
        _ => {
            let v = (8 + (i - 232) * 10) as u8;
            Rgb { r: v, g: v, b: v }
        }
    }
}

fn named_default(n: NamedColor) -> Rgb {
    use NamedColor::*;
    match n {
        Foreground | BrightForeground => FG,
        Background => BG,
        Cursor => CURSOR,
        DimForeground => dim(FG, BG),
        DimBlack | DimRed | DimGreen | DimYellow | DimBlue | DimMagenta | DimCyan | DimWhite => {
            let base = n as usize - DimBlack as usize;
            dim(rgb(ANSI[base]), BG)
        }
        other => indexed_default(other as usize),
    }
}

/// `Colors` holds what the program set with OSC 4/10/11/12; unset entries use the theme.
pub fn resolve(c: Color, colors: &Colors) -> Rgb {
    match c {
        Color::Spec(rgb) => rgb,
        Color::Indexed(i) => colors[i as usize].unwrap_or_else(|| indexed_default(i as usize)),
        Color::Named(n) => colors[n].unwrap_or_else(|| named_default(n)),
    }
}

pub fn dim(c: Rgb, bg: Rgb) -> Rgb {
    let mix = |a: u8, b: u8| ((a as f32) * 0.66 + (b as f32) * 0.34) as u8;
    Rgb {
        r: mix(c.r, bg.r),
        g: mix(c.g, bg.g),
        b: mix(c.b, bg.b),
    }
}

pub fn hsla(c: Rgb) -> Hsla {
    gpui::rgb(((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32).into()
}
