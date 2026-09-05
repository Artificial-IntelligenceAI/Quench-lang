//! Whether every kind of token is one the lexer actually makes.
//!
//! `Kind::Times` was declared, given a spelling in `describe` — `` `x` `` — and matched
//! by the parser, and the lexer never produced one. It was two designs old: `x` is a
//! *word* now, and the character `×` is deliberately not read at all, which
//! `notes/precedence-stops-where-maths-stopped.md` says outright. Three mentions in the
//! whole workspace, none of them a construction, and nothing noticed for as long as it
//! existed.
//!
//! The check that would have caught it runs the direction lists usually do not: rather
//! than asking whether every kind the lexer makes is known, it asks whether every kind
//! that is *known* is ever made. Same shape as `Trap::ALL`, one layer down — and the
//! website session found this one by reading a page, which is the fifth time a person
//! has caught what no comparison could.

use quench_lex::Kind;

use std::path::Path;

/// Quench holding every kind of token there is, including the ones the examples have no
/// reason to use.
const CORNERS: &str = "\
import [maths];
const.file.i64 ['A'] = [*1*];
fn.file.bool ['odd'] [immut.i64 'n'] {
    give [('n' ^ *2*) !== *0* and ('n' <== *9*) or ('n' >== *0*)];
}
START {
    var.mut.arr.i64 (2 3) ['grid'] = [[*1* *2* *3* *4* *5* *6*]];
    var.immut.bool ['same'] = ['grid'[*1* *1*] == *1*];
    print.stdout[str:*a* \\n 'same' call maths.sqrt[*4.0*]];
}
";

fn made_by(source: &str) -> Vec<Kind> {
    quench_lex::lex(source).tokens.iter().map(|token| token.kind).collect()
}

#[test]
fn every_kind_of_token_is_one_the_lexer_makes() {
    let mut seen: Vec<Kind> = made_by(CORNERS);

    // The examples too, so the corpus is not only a snippet written to satisfy this.
    let examples = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("examples");
    let mut read = 0;
    let mut walk = vec![examples];
    while let Some(at) = walk.pop() {
        for entry in std::fs::read_dir(&at).expect("the examples are beside the crates") {
            let path = entry.expect("a directory entry").path();
            if path.is_dir() {
                walk.push(path);
            } else if path.extension().is_some_and(|end| end == "qnl") {
                seen.extend(made_by(&std::fs::read_to_string(&path).expect("readable")));
                read += 1;
            }
        }
    }
    assert!(read > 5, "only {read} examples were read");

    let never: Vec<Kind> = Kind::ALL.iter().copied().filter(|kind| !seen.contains(kind)).collect();
    assert!(
        never.is_empty(),
        "nothing the lexer reads produces {never:?} — either it is dead like `Times` was, \
         or the corpus above is missing the Quench that would make one"
    );
}

#[test]
fn the_list_of_kinds_is_the_enum() {
    // `listed` is an exhaustive match, so a kind added to the enum and not to `ALL`
    // stops the build rather than going unchecked. This is the other half: nothing is
    // in the list twice, and everything in it answers to itself.
    for kind in Kind::ALL {
        assert!(kind.listed(), "{kind:?} is in `ALL` and does not answer to it");
    }
    for (n, kind) in Kind::ALL.iter().enumerate() {
        assert!(
            !Kind::ALL[..n].contains(kind),
            "{kind:?} is listed twice"
        );
    }
}
