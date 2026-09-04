//! Where a thing is in a source file, and how to say so.
//!
//! Derived from `luarust-diag` in [Luarust], the author's earlier language, now
//! abandoned. MIT, copyright 2026 Tankun Sriket — the same copyright holder, which is
//! why it can be relicensed under Quench's `MIT OR Apache-2.0`. The layout, the three
//! measurements below, and the idea that an error should name the *rule* it broke are
//! all Luarust's; Quench changed the names and the worked example and left the rest
//! alone, because it was already right.
//!
//! [Luarust]: https://github.com/Artificial-IntelligenceAI/Luarust
//!
//! Quench reports a position three different ways at once, because a position is three
//! different numbers and only one of them is the one a person means:
//!
//! - the **column a reader is given** counts graphemes, so `🧑‍🧑‍🧒‍🧒` is one character
//!   exactly as `c` is;
//! - the **column in `file:line:column`** counts bytes, because that form exists to be
//!   pasted into an editor or a `grep`, and bytes are what those understand;
//! - and the **caret** is placed by terminal cells, because that emoji draws two cells
//!   wide where a letter draws one.
//!
//! Get any of the three wrong and the error still looks plausible, which is why they are
//! separated here rather than left for whoever writes the renderer to conflate.

pub mod diag;
pub use quench_text::grapheme;
pub mod render;
pub mod source;

pub use diag::{Diagnostic, Label, LabelStyle};
pub use render::{report, GREETING};
pub use source::{Position, SourceFile, Span};
