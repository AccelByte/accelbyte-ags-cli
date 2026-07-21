//! A ratatui [`Backend`] wrapper that strips foreground/background colour from
//! every drawn cell when `--no-color` / `NO_COLOR` is in effect.
//!
//! ratatui renders the `Style`/`Color` values it is given regardless of any
//! colour preference, so the inline and fullscreen TUIs would otherwise ignore
//! `--no-color`. Wrapping the real backend lets us honour it in one place
//! instead of threading the flag through every styled span. Text and non-colour
//! modifiers (bold, italic, underline) are preserved — only colour is reset.

use std::io;

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::Color;

/// Wraps any [`Backend`], resetting cell colours on draw when `strip` is true.
pub struct NoColorBackend<B> {
    inner: B,
    strip: bool,
}

impl<B> NoColorBackend<B> {
    /// Wrap `inner`. When `strip` is true, each drawn cell's fg/bg are reset to
    /// [`Color::Reset`] (the symbol and modifiers such as bold are unchanged).
    pub fn new(inner: B, strip: bool) -> Self {
        Self { inner, strip }
    }
}

impl<B: Backend> Backend for NoColorBackend<B> {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        if !self.strip {
            return self.inner.draw(content);
        }
        let cells: Vec<(u16, u16, Cell)> = content
            .map(|(x, y, cell)| (x, y, strip_cell_colour(cell)))
            .collect();
        self.inner.draw(cells.iter().map(|(x, y, c)| (*x, *y, c)))
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }

    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Clone `cell` with its fg/bg reset to [`Color::Reset`]; symbol and modifiers
/// are preserved.
fn strip_cell_colour(cell: &Cell) -> Cell {
    let mut cell = cell.clone();
    cell.set_fg(Color::Reset);
    cell.set_bg(Color::Reset);
    cell
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;

    #[test]
    fn test_strip_cell_colour_resets_fg_bg_keeps_symbol_and_modifier() {
        let mut cell = Cell::default();
        cell.set_symbol("x");
        cell.set_fg(Color::Cyan);
        cell.set_bg(Color::Red);
        cell.modifier = Modifier::BOLD;
        let stripped = strip_cell_colour(&cell);
        assert_eq!(stripped.symbol(), "x", "symbol preserved");
        assert_eq!(stripped.fg, Color::Reset, "fg reset");
        assert_eq!(stripped.bg, Color::Reset, "bg reset");
        assert!(stripped.modifier.contains(Modifier::BOLD), "modifier kept");
    }

    #[test]
    fn test_no_strip_when_disabled_is_identity_on_draw() {
        // With strip=false the wrapper draws the original styled cells unchanged
        // into the inner TestBackend.
        use ratatui::backend::TestBackend;
        let mut backend = NoColorBackend::new(TestBackend::new(1, 1), false);
        let mut cell = Cell::default();
        cell.set_symbol("y");
        cell.set_fg(Color::Cyan);
        Backend::draw(&mut backend, std::iter::once((0u16, 0u16, &cell))).unwrap();
        assert_eq!(backend.inner.buffer()[(0, 0)].fg, Color::Cyan);
    }

    #[test]
    fn test_strip_when_enabled_resets_drawn_cell() {
        use ratatui::backend::TestBackend;
        let mut backend = NoColorBackend::new(TestBackend::new(1, 1), true);
        let mut cell = Cell::default();
        cell.set_symbol("y");
        cell.set_fg(Color::Cyan);
        cell.set_bg(Color::Red);
        Backend::draw(&mut backend, std::iter::once((0u16, 0u16, &cell))).unwrap();
        let drawn = &backend.inner.buffer()[(0, 0)];
        assert_eq!(drawn.symbol(), "y");
        assert_eq!(drawn.fg, Color::Reset);
        assert_eq!(drawn.bg, Color::Reset);
    }
}
