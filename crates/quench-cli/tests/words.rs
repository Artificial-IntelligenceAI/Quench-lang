//! Whether `quench words` says everything the language actually knows.
//!
//! Six separate hand-written lists of these have been found wrong in a day: the
//! statement keywords in a diagnostic, the provided functions twice within an hour, the
//! settings keys, the bar syntax in three comments, and a website's copy of the lot. In
//! every case the list was checked in the easy direction — is everything I know still
//! there — which catches a word being removed and can never catch one being added.
//!
//! So this runs the other way. It reads every word the parser and the checker actually
//! compare a token against, out of their source, and demands `quench words` knows each
//! one. `else` and `else-if` were both found by exactly this, having been in the
//! language since `if` was and on no list anywhere.

use std::path::Path;

/// Every word the source dispatches on: `"word" =>`, `"a" | "b"`, or `== "word"`.
///
/// Reading source with a regular expression is a blunt instrument and it is meant to be.
/// The list has to be held against the code, and the code is the only thing that cannot
/// be wrong about what the code does.
fn compared_against() -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for file in ["quench-parse/src/lib.rs", "quench-check/src/lib.rs"] {
        let path = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../")).join(file);
        let text = std::fs::read_to_string(&path).expect("a crate beside this one");
        for line in text.lines() {
            let trimmed = line.trim();
            // Prose in a diagnostic is not a word the language knows.
            if trimmed.starts_with("//") {
                continue;
            }
            let mut rest = line;
            while let Some(open) = rest.find('"') {
                let after = &rest[open + 1..];
                let Some(close) = after.find('"') else { break };
                let word = &after[..close];
                let tail = after[close + 1..].trim_start();
                let dispatched = tail.starts_with("=>") || tail.starts_with("| \"");
                let compared = rest[..open].trim_end().ends_with("==");
                if (dispatched || compared)
                    && !word.is_empty()
                    && word.len() < 16
                    && word.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                {
                    found.push(word.to_string());
                }
                rest = &after[close + 1..];
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

fn listed() -> Vec<String> {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_quench"))
        .arg("words")
        .output()
        .expect("`quench words` runs");
    String::from_utf8(out.stdout)
        .expect("text")
        .lines()
        .filter_map(|line| line.split_once('\t').map(|(_, word)| word.to_string()))
        .collect()
}

#[test]
fn every_word_the_compiler_dispatches_on_is_a_word_it_lists() {
    let listed = listed();
    let missing: Vec<String> = compared_against()
        .into_iter()
        .filter(|word| !listed.contains(word))
        .collect();
    assert!(
        missing.is_empty(),
        "the parser or checker dispatches on {missing:?}, and `quench words` has never heard of them"
    );
}

#[test]
fn the_list_is_grouped_and_says_where_each_word_stands() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_quench"))
        .arg("words")
        .output()
        .expect("`quench words` runs");
    let text = String::from_utf8(out.stdout).expect("text");
    assert!(text.lines().all(|line| line.contains('\t')), "every line is a group and a word");
    for expected in ["statement\tgive", "after a block\telse-if", "provided\tsqrt", "type\td64"] {
        assert!(text.contains(expected), "`{expected}` missing:\n{text}");
    }
    // None of them is reserved, which is the thing the number on its own would imply.
    assert!(quench_lower::lower("START { var.immut.i64 ['loop'] = [*1*]; print.stdout['loop']; }").ok());
}
