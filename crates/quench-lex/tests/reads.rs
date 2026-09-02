//! What the lexer makes of Quench, and what it says when it cannot.

use quench_diag::SourceFile;
use quench_lex::{lex, Kind};

fn kinds(source: &str) -> Vec<Kind> {
    lex(source).tokens.iter().map(|t| t.kind).collect()
}

fn text(source: &str, n: usize) -> String {
    let t = lex(source).tokens[n];
    source[t.span.start..t.span.end].to_string()
}

#[test]
fn a_declaration_comes_apart_the_way_the_chain_reads() {
    use Kind::*;
    assert_eq!(
        kinds("var.mut.b16 ['x'] = [|1000|];"),
        [
            Word, Dot, Word, Dot, Word, // var . mut . b16
            OpenList, Name, CloseList,  // [ 'x' ]
            Equals,
            OpenList, Literal, CloseList, // [ |1000| ]
            Semicolon,
            End,
        ]
    );
}

#[test]
fn quench_reserves_no_words() {
    // Every one of these is a Word and nothing more. The parser decides from position.
    for word in ["var", "mut", "file", "program", "export", "b16", "START", "banana"] {
        assert_eq!(kinds(word), [Kind::Word, Kind::End], "{word}");
    }
}

#[test]
fn a_name_may_contain_anything_you_can_type() {
    let source = "var.str ['a friendly greeting'] = [|hello|];";
    assert_eq!(text(source, 4), "'a friendly greeting'");

    let source = "var.b16 ['🧑‍🧑‍🧒‍🧒'] = [|1|];";
    assert_eq!(text(source, 4), "'🧑‍🧑‍🧒‍🧒'");
}

#[test]
fn a_value_wears_bars_or_backticks_and_the_lexer_does_not_read_it() {
    // `1000` is a number under b16 and text under str. That is the type's decision, not
    // this one, so both arrive as the same kind of token.
    assert_eq!(kinds("[|1000|]"), [Kind::OpenList, Kind::Literal, Kind::CloseList, Kind::End]);
    assert_eq!(kinds("[`1000`]"), [Kind::OpenList, Kind::Literal, Kind::CloseList, Kind::End]);
    assert_eq!(text("[|1000|]", 1), "|1000|");
}

#[test]
fn a_hyphenated_word_is_one_word() {
    assert_eq!(kinds("no-visibility-stated"), [Kind::Word, Kind::End]);
    assert_eq!(text("no-visibility-stated", 0), "no-visibility-stated");
}

#[test]
fn a_comment_reaches_the_end_of_its_line_and_no_further() {
    assert_eq!(
        kinds("# this is ignored;\nvar.str ['a'] = [|b|];"),
        kinds("var.str ['a'] = [|b|];")
    );
}

#[test]
fn the_end_is_always_the_last_token() {
    for source in ["", "   ", "# only a comment", "var", "'unclosed"] {
        assert_eq!(*kinds(source).last().unwrap(), Kind::End, "{source:?}");
    }
}

#[test]
fn an_unclosed_name_says_so_where_it_opened() {
    let source = "var.str ['name = [|x|];\n";
    let out = lex(source);
    assert_eq!(out.errors.len(), 1, "{:?}", out.errors);

    let file = SourceFile::new("src/main.qnl", source);
    let rendered = quench_diag::report(&file, &out.errors);
    let expected = "\
Hello, I think there may be thing(s) wrong with your code. I'm sorry, if I'm wrong.

file: src/main.qnl, line: 1, column: 10 (src/main.qnl:1:10)

a name was opened here and never closed.

  1 | var.str ['name = [|x|];
    |          ^^^^^^^^^^^^^^ this `'` has no partner

Error code: E0002
Rule(s) broken: a name begins and ends with `'`, on one line
Tip(s): a line ending closes nothing — it is the mark that does.
Suggested fix(s): add a closing `'` before the end of the line

1 error.
";
    assert_eq!(rendered, expected, "\n--- got ---\n{rendered}");
}

#[test]
fn a_double_quoted_thing_is_one_mistake_not_two() {
    // Reported once, as a whole, with both replacements offered -- rather than once per
    // quote character with half the story each time.
    let out = lex("var.str ['a'] = [\"hello\"];");
    assert_eq!(out.errors.len(), 1, "{:#?}", out.errors);
    assert_eq!(out.errors[0].code, "E0003");
    let fix = out.errors[0].fixes.join(" ");
    assert!(fix.contains("`'hello'`"), "{fix}");
    assert!(fix.contains("`|hello|`"), "{fix}");
}

#[test]
fn a_value_written_without_bars_is_told_where_bars_go() {
    let out = lex("var.b16 ['x'] = [1000];");
    assert!(!out.ok());
    assert_eq!(out.errors[0].code, "E0001");
    assert!(out.errors[0].tips.join(" ").contains("between bars"), "{:?}", out.errors[0]);
}

#[test]
fn one_bad_line_does_not_hide_the_next() {
    // Three separate mistakes. A lexer that stopped at the first would report a third of
    // what is wrong with this file.
    let out = lex("var.str ['a = [|x|];\nvar.b16 ['y'] = [\"z\"];\nvar.b16 ['w'] = [9];\n");
    assert_eq!(out.errors.len(), 3, "{:#?}", out.errors);
    assert_eq!(out.errors.iter().map(|e| e.code.as_str()).collect::<Vec<_>>(), ["E0002", "E0003", "E0001"]);
}

#[test]
fn recovery_keeps_what_followed_the_bad_mark() {
    // The opening quote was the mistake; `name` after it is almost certainly real source,
    // so it is lexed rather than swallowed into a very long name.
    let out = lex("['name];\n");
    assert_eq!(out.errors.len(), 1, "{:#?}", out.errors);
    assert!(out.tokens.iter().any(|t| t.kind == Kind::Word), "{:?}", out.tokens);
}

/// The line the author wrote to see whether this would survive it.
const THE_HARD_ONE: &str =
    r"print[*Hello, World! 🤣 \* 234567uythgf{}9!@#$%^&* 'x' *🧑‍🧑‍🧒‍🧒🥹✌️* 'y' \n];";

#[test]
fn the_hard_one_comes_apart_exactly_as_written() {
    use Kind::*;
    let out = lex(THE_HARD_ONE);
    assert!(out.ok(), "{:#?}", out.errors);
    assert_eq!(
        out.tokens.iter().map(|t| t.kind).collect::<Vec<_>>(),
        [Word, OpenList, Text, Name, Text, Name, Escape, CloseList, Semicolon, End]
    );

    // The pieces, checked one at a time, because the interesting claim is where each
    // one *ends*.
    let piece = |n: usize| {
        let t = out.tokens[n];
        &THE_HARD_ONE[t.span.start..t.span.end]
    };
    assert_eq!(piece(0), "print");
    // The `&*` at the end is an ampersand and then the closing mark, not `&` and a star.
    assert_eq!(piece(2), r"*Hello, World! 🤣 \* 234567uythgf{}9!@#$%^&*");
    assert_eq!(piece(3), "'x'");
    assert_eq!(piece(4), "*🧑‍🧑‍🧒‍🧒🥹✌️*");
    assert_eq!(piece(5), "'y'");
    assert_eq!(piece(6), r"\n");
}

#[test]
fn text_holds_whatever_was_put_in_it() {
    // Braces, punctuation, digits and a semicolon are all just characters in here. If any
    // of them were tokens the line below would come apart into a dozen pieces.
    let source = r"print[*{};[]|'#$%^&= 12345*];";
    let out = lex(source);
    assert!(out.ok(), "{:#?}", out.errors);
    assert_eq!(out.tokens[2].kind, Kind::Text);
    assert_eq!(&source[out.tokens[2].span.start..out.tokens[2].span.end], r"*{};[]|'#$%^&= 12345*");
}

#[test]
fn a_star_inside_text_is_written_with_a_backslash() {
    let out = lex(r"print[*two \* three*];");
    assert!(out.ok(), "{:#?}", out.errors);
    assert_eq!(out.tokens[2].kind, Kind::Text);
}

#[test]
fn an_escape_between_the_marks_is_not_an_escape() {
    // The whole point of putting escapes outside: what is between the marks is literal,
    // so this is one piece of text containing a backslash and an `n`, not a newline.
    let out = lex(r"print[*a\nb*];");
    assert!(out.ok(), "{:#?}", out.errors);
    assert_eq!(out.tokens.iter().filter(|t| t.kind == Kind::Escape).count(), 0);
}

#[test]
fn the_escapes_stand_on_their_own() {
    for e in [r"\n", r"\t", r"\r", r"\\"] {
        let out = lex(&format!("print[{e}];"));
        assert!(out.ok(), "{e}: {:#?}", out.errors);
        assert_eq!(out.tokens[2].kind, Kind::Escape, "{e}");
    }
}

#[test]
fn an_escape_nobody_has_heard_of_lists_the_ones_that_exist() {
    let out = lex(r"print[\q];");
    assert_eq!(out.errors.len(), 1, "{:#?}", out.errors);
    assert_eq!(out.errors[0].code, "E0004");
    assert!(out.errors[0].rules.join(" ").contains(r"`\n`, `\t`, `\r` and `\\`"), "{:?}", out.errors[0]);
}

#[test]
fn unclosed_text_says_so_and_offers_the_backslash() {
    let out = lex("print[*Hello\n];");
    assert_eq!(out.errors.len(), 1, "{:#?}", out.errors);
    assert_eq!(out.errors[0].code, "E0002");
    assert!(out.errors[0].tips.join(" ").contains(r"put a `\` in front"), "{:?}", out.errors[0]);
}

#[test]
fn a_double_quote_now_offers_all_three_marks() {
    let out = lex("print[\"Hello\"];");
    assert_eq!(out.errors.len(), 1, "{:#?}", out.errors);
    let fix = out.errors[0].fixes.join(" ");
    assert!(fix.contains("`'Hello'`"), "{fix}");
    assert!(fix.contains("`|Hello|`"), "{fix}");
    assert!(fix.contains("`*Hello*`"), "{fix}");
}
