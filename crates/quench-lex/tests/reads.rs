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
        kinds("var.mut.b16 ['x'] = [*1000*];"),
        [
            Word, Dot, Word, Dot, Word, // var . mut . b16
            OpenList, Name, CloseList,  // [ 'x' ]
            Equals,
            OpenList, Written, CloseList, // [ |1000| ]
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
    let source = "var.str ['a friendly greeting'] = [*hello*];";
    assert_eq!(text(source, 4), "'a friendly greeting'");

    let source = "var.b16 ['🧑‍🧑‍🧒‍🧒'] = [*1*];";
    assert_eq!(text(source, 4), "'🧑‍🧑‍🧒‍🧒'");
}

#[test]
fn one_mark_for_a_written_value_and_the_lexer_does_not_read_it() {
    // `1000` is a number under b16 and text under str. That is the type's decision, not
    // this one, so there is one kind of token here and no reading of what is inside.
    assert_eq!(kinds("[*1000*]"), [Kind::OpenList, Kind::Written, Kind::CloseList, Kind::End]);
    assert_eq!(text("[*1000*]", 1), "*1000*");
    assert_eq!(kinds("[*hello*]"), kinds("[*1000*]"));
}

#[test]
fn a_hyphenated_word_is_one_word() {
    assert_eq!(kinds("no-visibility-stated"), [Kind::Word, Kind::End]);
    assert_eq!(text("no-visibility-stated", 0), "no-visibility-stated");
}

#[test]
fn a_comment_reaches_the_end_of_its_line_and_no_further() {
    assert_eq!(
        kinds("# this is ignored;\nvar.str ['a'] = [*b*];"),
        kinds("var.str ['a'] = [*b*];")
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
    let source = "var.str ['name = [*x*];\n";
    let out = lex(source);
    assert_eq!(out.errors.len(), 1, "{:?}", out.errors);

    let file = SourceFile::new("src/main.qnl", source);
    let rendered = quench_diag::report(&file, &out.errors);
    let expected = "\
Hello, I think there may be thing(s) wrong with your code. I'm sorry, if I'm wrong.

file: src/main.qnl, line: 1, column: 10 (src/main.qnl:1:10)

a name was opened here and never closed.

  1 | var.str ['name = [*x*];
    |          ^^^^^^^^^^^^^^ this `'` has no partner

Error code: E0002
Rule(s) broken: a name begins and ends with `'`, on one line
Tip(s):
  - a line ending closes nothing — it is the mark that does.
  - to write a `'` inside one, put a `\\` in front of it.
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
    assert!(fix.contains("`*hello*`"), "{fix}");
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
    let out = lex("var.str ['a = [*x*];\nvar.b16 ['y'] = [\"z\"];\nvar.b16 ['w'] = [9];\n");
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

/// The line the author wrote to see whether this would survive it, with the types the
/// written values now carry.
const THE_HARD_ONE: &str =
    r"print[str:*Hello, World! 🤣 \* 234567uythgf{}9!@#$%^&* 'x' str:*🧑‍🧑‍🧒‍🧒🥹✌️* 'y' \n];";

#[test]
fn the_hard_one_comes_apart_exactly_as_written() {
    use Kind::*;
    let out = lex(THE_HARD_ONE);
    assert!(out.ok(), "{:#?}", out.errors);
    assert_eq!(
        out.tokens.iter().map(|t| t.kind).collect::<Vec<_>>(),
        [
            Word, OpenList,
            Word, Colon, Written,   // str:*…*
            Name,                   // 'x' — bare, its declaration gave it a type
            Word, Colon, Written,
            Name,
            Escape, CloseList, Semicolon, End,
        ]
    );

    // The pieces, checked one at a time, because the interesting claim is where each
    // one *ends*.
    let piece = |n: usize| {
        let t = out.tokens[n];
        &THE_HARD_ONE[t.span.start..t.span.end]
    };
    assert_eq!(piece(0), "print");
    // The `&*` at the end is an ampersand and then the closing mark, not `&` and a star.
    assert_eq!(piece(4), r"*Hello, World! 🤣 \* 234567uythgf{}9!@#$%^&*");
    assert_eq!(piece(5), "'x'");
    assert_eq!(piece(8), "*🧑‍🧑‍🧒‍🧒🥹✌️*");
    assert_eq!(piece(9), "'y'");
    assert_eq!(piece(10), r"\n");
}

#[test]
fn text_holds_whatever_was_put_in_it() {
    // Braces, punctuation, digits and a semicolon are all just characters in here. If any
    // of them were tokens the line below would come apart into a dozen pieces.
    let source = r"print[str:*{};[]|'#$%^&= 12345*];";
    let out = lex(source);
    assert!(out.ok(), "{:#?}", out.errors);
    assert_eq!(out.tokens[4].kind, Kind::Written);
    assert_eq!(&source[out.tokens[4].span.start..out.tokens[4].span.end], r"*{};[]|'#$%^&= 12345*");
}

#[test]
fn a_star_inside_text_is_written_with_a_backslash() {
    let out = lex(r"print[str:*two \* three*];");
    assert!(out.ok(), "{:#?}", out.errors);
    assert_eq!(out.tokens[4].kind, Kind::Written);
}

#[test]
fn an_escape_between_the_marks_is_not_an_escape() {
    // The whole point of putting escapes outside: what is between the marks is literal,
    // so this is one piece of text containing a backslash and an `n`, not a newline.
    let out = lex(r"print[str:*a\nb*];");
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
    let out = lex("print[str:*Hello\n];");
    assert_eq!(out.errors.len(), 1, "{:#?}", out.errors);
    assert_eq!(out.errors[0].code, "E0002");
    assert!(out.errors[0].tips.join(" ").contains(r"put a `\` in front"), "{:?}", out.errors[0]);
}

#[test]
fn a_double_quote_offers_both_marks() {
    let out = lex("print[\"Hello\"];");
    assert_eq!(out.errors.len(), 1, "{:#?}", out.errors);
    let fix = out.errors[0].fixes.join(" ");
    assert!(fix.contains("`'Hello'`"), "{fix}");
    assert!(fix.contains("`*Hello*`"), "{fix}");
}

#[test]
fn a_value_is_a_list_of_items_juxtaposed() {
    use Kind::*;
    // Nothing joins them. There is no `+` because nothing is being built -- the items
    // sit next to each other and are used in order.
    let out = lex(r"var.str ['s'] = [*line one* \n *line two*];");
    assert!(out.ok(), "{:#?}", out.errors);
    assert_eq!(
        out.tokens.iter().map(|t| t.kind).collect::<Vec<_>>(),
        [
            Word, Dot, Word,                       // var . str
            OpenList, Name, CloseList,             // [ 's' ]
            Equals,
            OpenList, Written, Escape, Written, CloseList, // [ *line one* \n *line two* ]
            Semicolon,
            End,
        ]
    );
}

#[test]
fn commas_separate_the_values_and_juxtaposition_builds_each_one() {
    use Kind::*;
    // Two names, two values. The comma is the only thing that says where one value stops,
    // which is what lets a value be as many items long as it likes.
    let out = lex(r"var.str ['s', 'ss'] = [*line one* \n *line two*, *idk* \n *Claude*];");
    assert!(out.ok(), "{:#?}", out.errors);
    assert_eq!(
        out.tokens.iter().map(|t| t.kind).collect::<Vec<_>>(),
        [
            Word, Dot, Word,
            OpenList, Name, Comma, Name, CloseList,
            Equals,
            OpenList,
            Written, Escape, Written,   // 's'
            Comma,
            Written, Escape, Written,   // 'ss'
            CloseList,
            Semicolon,
            End,
        ]
    );
}

#[test]
fn a_printed_value_carries_the_type_that_reads_it() {
    use Kind::*;
    // `*1000*` is a number under `b16` and four characters under `str`. A declaration
    // says which in its chain; a print list has no chain, so the value says it itself.
    let out = lex(r"print[str:** \n];");
    assert!(out.ok(), "{:#?}", out.errors);
    assert_eq!(
        out.tokens.iter().map(|t| t.kind).collect::<Vec<_>>(),
        [Word, OpenList, Word, Colon, Written, Escape, CloseList, Semicolon, End]
    );
}

#[test]
fn an_empty_written_value_is_a_pair_of_marks_with_nothing_in_them() {
    let out = lex("print[str:**];");
    assert!(out.ok(), "{:#?}", out.errors);
    let t = out.tokens[4];
    assert_eq!(t.kind, Kind::Written);
    assert_eq!(&"print[str:**];"[t.span.start..t.span.end], "**");
}

#[test]
fn the_type_is_a_word_like_any_other() {
    use Kind::*;
    for ty in ["str", "b16", "i64", "bool"] {
        let source = format!("print[{ty}:*1000* \\n];");
        let out = lex(&source);
        assert!(out.ok(), "{ty}: {:#?}", out.errors);
        assert_eq!(
            out.tokens.iter().map(|t| t.kind).collect::<Vec<_>>(),
            [Word, OpenList, Word, Colon, Written, Escape, CloseList, Semicolon, End],
            "{ty}"
        );
    }
}

#[test]
fn grouping_has_its_own_marks() {
    use Kind::*;
    assert_eq!(
        kinds("[(*1* + *2*)]"),
        [OpenList, OpenGroup, Written, Plus, Written, CloseGroup, CloseList, End]
    );
}

#[test]
fn multiplication_can_be_written_either_way() {
    use Kind::*;
    // `*` is the mark a written value wears, so it cannot also be multiplication. The
    // sign gets a token; the word `x` is a word, and the parser knows what it means
    // where it stands -- which costs nothing, because Quench reserves no words.
    assert_eq!(kinds("[*2* \u{d7} *3*]"), [OpenList, Written, Times, Written, CloseList, End]);
    assert_eq!(kinds("[*2* x *3*]"), [OpenList, Written, Word, Written, CloseList, End]);
}

#[test]
fn a_hyphen_is_still_part_of_a_word_when_it_is_inside_one() {
    use Kind::*;
    assert_eq!(kinds("no-visibility-stated"), [Word, End]);
    assert_eq!(kinds("[*5* - *3*]"), [OpenList, Written, Minus, Written, CloseList, End]);
}
