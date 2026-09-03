//! Syntax written in a comment is syntax the language has.
//!
//! The README is a test, so prose in `.md` cannot drift. Comments in `.rs` are prose
//! too, and nothing was watching them: the marks around a written value changed from
//! `|1000|` to `*1000*` long ago and three comments kept the old form, including the
//! lexer's own module doc — which renders on `cargo doc` and explained the lexer's
//! design using syntax the lexer refuses outright.
//!
//! This catches that one shape. It is not a parser for comments and does not want to
//! be; what it is, is the cheapest thing that would have caught the drift that happened.

use std::path::{Path, PathBuf};

fn crates() -> PathBuf {
    let at = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/.."));
    at.canonicalize().unwrap_or_else(|_| at.to_path_buf())
}

/// This file, which is the one place the old form has to be written down in order to
/// be forbidden.
fn itself() -> PathBuf {
    let at = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/comments.rs"));
    at.canonicalize().unwrap_or_else(|_| at.to_path_buf())
}

/// Every `.rs` file under `crates/`, whatever it is nested in.
fn sources(at: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(at) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

/// Whether this line writes a value between bars.
///
/// Two shapes, because the three that drifted used both. A run of digits between bars
/// is never valid Rust -- a closure's parameter cannot start with a digit -- so it can
/// be flagged wherever it appears. A bracket opening onto a bar is the other, and it is
/// how `[ |1000| ]` is written when somebody has put spaces in.
fn writes_between_bars(line: &str, bar: char) -> bool {
    let letters: Vec<char> = line.chars().collect();
    for (i, c) in letters.iter().enumerate() {
        if *c != bar {
            continue;
        }
        // `|1000|`, digits all the way to the closing bar.
        let digits = letters[i + 1..].iter().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && letters.get(i + 1 + digits) == Some(&bar) {
            return true;
        }
        // `[ |` and `| ]`, whatever sits between the two.
        let before = letters[..i].iter().rev().find(|c| !c.is_whitespace());
        let after = letters[i + 1..].iter().find(|c| !c.is_whitespace());
        if before == Some(&'[') || after == Some(&']') {
            return true;
        }
    }
    false
}

#[test]
fn no_source_file_writes_a_value_between_bars() {
    // Built rather than written, so that this file does not find itself.
    let bar = char::from(b'|');

    let mut files = Vec::new();
    sources(&crates(), &mut files);
    assert!(files.len() > 20, "found {} files, which is too few to be all of them", files.len());

    let mut wrong = Vec::new();
    let this = itself();
    for path in files {
        if path == this {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        for (n, line) in text.lines().enumerate() {
            if writes_between_bars(line, bar) {
                wrong.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "a written value wears `*marks*`, and the lexer refuses `{bar}` outright:\n{}",
        wrong.join("\n")
    );
}
