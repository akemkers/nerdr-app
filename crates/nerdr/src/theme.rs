//! Colors: a fixed dark palette (Tokyo Night) and alacritty's color model mapped onto it.

use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui::Hsla;

pub const FG: Rgb = Rgb { r: 0xc0, g: 0xca, b: 0xf5 };
pub const BG: Rgb = Rgb { r: 0x1a, g: 0x1b, b: 0x26 };
pub const CURSOR: Rgb = Rgb { r: 0xc0, g: 0xca, b: 0xf5 };
pub const SELECTION: Rgb = Rgb { r: 0x33, g: 0x46, b: 0x7c };

const ANSI: [u32; 16] = [
    0x15161e, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xa9b1d6, //
    0x414868, 0xf7768e, 0x9ece6a, 0xe0af68, 0x7aa2f7, 0xbb9af7, 0x7dcfff, 0xc0caf5,
];

fn hex(v: u32) -> Rgb {
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
        0..16 => hex(ANSI[i]),
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
            dim(hex(ANSI[base]), BG)
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
