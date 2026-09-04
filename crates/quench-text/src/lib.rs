//! What a character is, and how wide it draws.
//!
//! Its own crate because two very different things need the same answer. The diagnostic
//! renderer counts characters to put a caret under the right column, and the *engines*
//! count them because `count['s']` is a question a program can ask — and an engine has
//! no business linking a renderer to find out.
//!
//! See [`grapheme`] for what a character is taken to be, and for the Unicode version
//! that answer is pinned to.

pub mod grapheme;
