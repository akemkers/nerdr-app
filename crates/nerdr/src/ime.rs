//! Text from the platform's input method: IME composition and dead keys. Plain text keys
//! come here instead of `on_key_down`, the rest of the keyboard doesn't.

use std::ops::Range;

use gpui::{App, Bounds, Entity, InputHandler, Pixels, Point, UTF16Selection, Window};

use crate::view::TerminalView;

pub struct TermInput(pub Entity<TerminalView>);

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

impl InputHandler for TermInput {
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<UTF16Selection> {
        let end = self.0.read(cx).marked().map_or(0, utf16_len);
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(&mut self, _: &mut Window, cx: &mut App) -> Option<Range<usize>> {
        self.0.read(cx).marked().map(|m| 0..utf16_len(m))
    }

    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted: &mut Option<Range<usize>>,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<String> {
        let units: Vec<u16> = self.0.read(cx).marked()?.encode_utf16().collect();
        let range = range.start.min(units.len())..range.end.min(units.len());
        *adjusted = Some(range.clone());
        String::from_utf16(&units[range]).ok()
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut App,
    ) {
        self.0.update(cx, |view, cx| view.commit_text(text, cx));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut App,
    ) {
        self.0.update(cx, |view, cx| view.set_marked(text, cx));
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut App) {
        self.0.update(cx, |view, cx| view.set_marked("", cx));
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Window,
        cx: &mut App,
    ) -> Option<Bounds<Pixels>> {
        Some(self.0.read(cx).cursor_bounds())
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut App,
    ) -> Option<usize> {
        None
    }

    // Holding a key repeats it, as in other terminals, instead of opening the accent menu
    fn apple_press_and_hold_enabled(&mut self) -> bool {
        false
    }
}
