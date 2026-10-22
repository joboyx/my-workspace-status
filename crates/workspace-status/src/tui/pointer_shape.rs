//! OSC 22 mouse pointer shape over the resize edges.
//!
//! While the pointer rests on a resize edge, or drags one, the terminal
//! pointer shows a resize arrow: `ew-resize` on the pane divider and the
//! side-by-side diff rule, `ns-resize` on the commit-message footer rule.
//! Terminals that implement OSC 22 with CSS pointer names (for example
//! kitty, foot, and Ghostty) show it; others ignore the sequence. Ratatui
//! cannot emit it, so `draw_frame` writes it after each frame, and only
//! when the shape changes. A launch that never hovers an edge writes no
//! OSC 22 at all. `restore_terminal` and the panic hook write
//! [`POINTER_RESET`] only while a resize shape shows ([`POINTER_SET`]).

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use super::split::ResizeEdge;

/// OSC 22 sequence that puts the terminal's default pointer back.
pub const POINTER_RESET: &str = "\x1b]22;default\x1b\\";

/// True while the terminal shows a resize pointer: the last shape a
/// [`TermPointer::default`] writer wrote was not [`PointerShape::Default`].
/// The exit restore and the panic hook write [`POINTER_RESET`] only then.
pub static POINTER_SET: AtomicBool = AtomicBool::new(false);

/// One mouse pointer shape the TUI asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerShape {
    /// The terminal's own pointer (CSS name `default`).
    Default,
    /// Left / right resize arrow (CSS name `ew-resize`).
    EwResize,
    /// Up / down resize arrow (CSS name `ns-resize`).
    NsResize,
}

impl PointerShape {
    /// The CSS pointer name that OSC 22 carries.
    pub fn css_name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::EwResize => "ew-resize",
            Self::NsResize => "ns-resize",
        }
    }
}

/// Pointer shape for the hovered or dragged resize edge
/// (`AppState::hovered_resize_edge`): the pane divider and the diff rule
/// move left / right, the commit-message rule moves up / down, and no edge
/// is the default pointer.
pub fn shape_for(edge: Option<ResizeEdge>) -> PointerShape {
    match edge {
        Some(ResizeEdge::Pane | ResizeEdge::DiffSplit) => PointerShape::EwResize,
        Some(ResizeEdge::CommitMsg) => PointerShape::NsResize,
        None => PointerShape::Default,
    }
}

/// The OSC 22 bytes for `shape`: `ESC ] 22 ; <name> ESC \`.
pub fn osc_22(shape: PointerShape) -> String {
    format!("\x1b]22;{}\x1b\\", shape.css_name())
}

/// Writes the pointer shape to the terminal when it changes.
#[derive(Debug)]
pub struct TermPointer {
    /// The shape the terminal shows now.
    last: PointerShape,
    /// Set while `last` is not the default; [`POINTER_SET`] outside tests.
    set: &'static AtomicBool,
}

impl Default for TermPointer {
    /// A writer that starts from a terminal with its default pointer and
    /// keeps [`POINTER_SET`] up to date.
    fn default() -> Self {
        Self::with_flag(&POINTER_SET)
    }
}

impl TermPointer {
    /// [`Self::default`] that keeps `set` instead of [`POINTER_SET`], so a
    /// test does not share the process-wide flag.
    pub(super) fn with_flag(set: &'static AtomicBool) -> Self {
        Self {
            last: PointerShape::Default,
            set,
        }
    }

    /// Write `next` to `out` and flush when it differs from the last shape
    /// written.
    pub fn write_if_changed<W: Write>(
        &mut self,
        out: &mut W,
        next: PointerShape,
    ) -> io::Result<()> {
        if next == self.last {
            return Ok(());
        }
        out.write_all(osc_22(next).as_bytes())?;
        out.flush()?;
        self.last = next;
        self.set
            .store(next != PointerShape::Default, Ordering::SeqCst);
        Ok(())
    }

    /// Put the default pointer back before another program takes the
    /// terminal (`$EDITOR`, an external diff tool). Writes nothing when the
    /// default already shows. The next frame after the resume writes the
    /// resize shape again if the pointer still rests on an edge.
    pub fn reset<W: Write>(&mut self, out: &mut W) -> io::Result<()> {
        self.write_if_changed(out, PointerShape::Default)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_22_bytes_for_each_shape() {
        assert_eq!(osc_22(PointerShape::EwResize), "\x1b]22;ew-resize\x1b\\");
        assert_eq!(osc_22(PointerShape::NsResize), "\x1b]22;ns-resize\x1b\\");
        assert_eq!(osc_22(PointerShape::Default), "\x1b]22;default\x1b\\");
        assert_eq!(osc_22(PointerShape::Default), POINTER_RESET);
    }

    #[test]
    fn shape_for_maps_each_edge() {
        assert_eq!(shape_for(None), PointerShape::Default);
        assert_eq!(shape_for(Some(ResizeEdge::Pane)), PointerShape::EwResize);
        assert_eq!(
            shape_for(Some(ResizeEdge::DiffSplit)),
            PointerShape::EwResize
        );
        assert_eq!(
            shape_for(Some(ResizeEdge::CommitMsg)),
            PointerShape::NsResize
        );
    }

    #[test]
    fn writer_emits_only_on_change() {
        static WRITER_SET: AtomicBool = AtomicBool::new(false);
        let mut pointer = TermPointer::with_flag(&WRITER_SET);
        let mut out = Vec::new();
        for (shape, set) in [
            (PointerShape::EwResize, true),
            (PointerShape::EwResize, true),
            (PointerShape::NsResize, true),
            (PointerShape::NsResize, true),
            (PointerShape::Default, false),
            (PointerShape::Default, false),
        ] {
            pointer.write_if_changed(&mut out, shape).unwrap();
            assert_eq!(
                WRITER_SET.load(Ordering::SeqCst),
                set,
                "set flag after {shape:?}"
            );
        }
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b]22;ew-resize\x1b\\\x1b]22;ns-resize\x1b\\\x1b]22;default\x1b\\"
        );
    }

    #[test]
    fn no_hover_launch_writes_nothing() {
        static IDLE_SET: AtomicBool = AtomicBool::new(false);
        let mut pointer = TermPointer::with_flag(&IDLE_SET);
        let mut out = Vec::new();
        for _ in 0..3 {
            pointer.write_if_changed(&mut out, shape_for(None)).unwrap();
        }
        pointer.reset(&mut out).unwrap();
        assert!(out.is_empty(), "the terminal starts on the default pointer");
        assert!(!IDLE_SET.load(Ordering::SeqCst));
    }

    #[test]
    fn reset_before_suspend_lets_the_next_frame_send_the_shape_again() {
        static SUSPEND_SET: AtomicBool = AtomicBool::new(false);
        let mut pointer = TermPointer::with_flag(&SUSPEND_SET);
        let mut out = Vec::new();
        pointer
            .write_if_changed(&mut out, PointerShape::EwResize)
            .unwrap();
        out.clear();

        pointer.reset(&mut out).unwrap();
        assert_eq!(String::from_utf8(out.clone()).unwrap(), POINTER_RESET);
        assert!(
            !SUSPEND_SET.load(Ordering::SeqCst),
            "a suspended TUI leaves no shape for the exit restore"
        );

        out.clear();
        pointer.reset(&mut out).unwrap();
        assert!(out.is_empty(), "a second reset writes nothing");

        // Resumed with the pointer still on the edge: the first frame
        // sends the shape again.
        pointer
            .write_if_changed(&mut out, PointerShape::EwResize)
            .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            osc_22(PointerShape::EwResize)
        );
        assert!(SUSPEND_SET.load(Ordering::SeqCst));
    }
}
