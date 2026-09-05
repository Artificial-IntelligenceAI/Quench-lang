//! Taking text apart: a piece of it, where something is in it, and what is left when
//! the space around it goes.
//!
//! Every one of these is written once, here, and called by every engine — the same
//! reason [`crate::grapheme`] is. Two implementations of "where does `sub` start" would
//! be two answers eventually, and an answer that depends on which engine ran it is the
//! thing the whole project is built to refuse.
//!
//! **Which of these depend on `[defaults] characters`, and which do not.** Slicing and
//! finding hand back or take a *position*, and what a position counts is exactly what
//! that setting decides — so each has two forms. Having, splitting and trimming do not:
//! a substring is found by its bytes whatever a character is taken to be, and the space
//! around a piece of text is space in either reading.

use crate::grapheme;

/// Where each character begins, and where the text ends.
///
/// One walk, so a slice costs one pass rather than one per end. The last entry is the
/// length of the whole text, which is what makes `from` of `count + 1` an empty slice
/// rather than a mistake.
fn starts(text: &str, clusters: bool) -> Vec<usize> {
    let mut at = Vec::new();
    if clusters {
        let mut offset = 0;
        for cluster in grapheme::graphemes(text) {
            at.push(offset);
            offset += cluster.len();
        }
    } else {
        at.extend(text.char_indices().map(|(offset, _)| offset));
    }
    at.push(text.len());
    at
}

/// How many characters, in whichever reading.
pub fn count(text: &str, clusters: bool) -> usize {
    if clusters { grapheme::count(text) } else { text.chars().count() }
}

/// The characters from `from` to `to`, both counted from one and both included.
///
/// `None` when a position is outside the text, which is the same refusal an index off
/// the end of an array gets. A `to` *before* a `from` is not that: it is empty text, the
/// way `loop.temp.range.i64 ['i'] = [*5*, *1*]` runs no times rather than complaining.
pub fn slice(text: &str, from: i64, to: i64, clusters: bool) -> Option<String> {
    if from < 1 {
        return None;
    }
    if to < from {
        return Some(String::new());
    }
    let at = starts(text, clusters);
    let held = at.len() - 1;
    let (from, to) = (from as usize, to as usize);
    if to > held {
        return None;
    }
    Some(text[at[from - 1]..at[to]].to_string())
}

/// Whether `sub` is anywhere in `text`.
///
/// By bytes, and deliberately: UTF-8 is self-synchronising, so a byte match is always a
/// character match and can never begin in the middle of one. Nothing here depends on
/// what a character is taken to be.
pub fn has(text: &str, sub: &str) -> bool {
    text.contains(sub)
}

/// Where `sub` begins in `text`, counted from one, in characters.
///
/// `None` when it is not there at all, which is what `has` is for asking first.
pub fn find(text: &str, sub: &str, clusters: bool) -> Option<i64> {
    let byte = text.find(sub)?;
    // Which character that byte begins is the question the setting answers, and the two
    // readings differ on exactly the text people test with.
    let at = starts(text, clusters);
    let which = at.iter().position(|start| *start == byte)?;
    Some(which as i64 + 1)
}

/// `text` cut at every `sep`, in order.
///
/// A separator that is not there gives one piece, which is the whole text — the answer
/// that makes splitting and then joining give back what went in. An empty separator has
/// no answer of that kind and is refused where this is called.
pub fn split(text: &str, sep: &str) -> Vec<String> {
    text.split(sep).map(str::to_string).collect()
}

/// `text` with the space at each end taken off.
pub fn trim(text: &str) -> String {
    text.trim_matches(is_space).to_string()
}

/// Whether a character is space, by Unicode's `White_Space` property.
///
/// Written down here rather than taken from the standard library for the reason the
/// grapheme tables are: what a Quench program answers must not depend on which compiler
/// built it. An artefact travels, and a `.qnlo` whose `trim` behaved one way here and
/// another way there would be the one place in the language where that is true.
///
/// `tests/spaces.rs` holds it against `char::is_whitespace`, so it is ours *and*
/// checked, rather than ours and trusted.
pub fn is_space(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{0085}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

/// Bytes as text, or `None` when they are not text and `[defaults] bad-bytes` says stop.
///
/// Lives here rather than in either engine so that both call the *same* rule: where a
/// replacement character lands is not something the interpreter and the Dev JIT may
/// each decide, and a second copy is exactly the kind of thing the oracle finds.
pub fn text_of(bytes: &[u8], stops: bool) -> Option<String> {
    match core::str::from_utf8(bytes) {
        Ok(said) => Some(said.to_owned()),
        Err(_) if stops => None,
        Err(_) => Some(String::from_utf8_lossy(bytes).into_owned()),
    }
}
