//! Whole programs, from what somebody typed to what the machine said.

use quench_diag::SourceFile;
use quench_lower::lower;

/// What the interpreter prints, and what the Dev JIT prints. They must match.
fn both(source: &str) -> (String, String) {
    let out = lower(source);
    assert!(out.ok(), "{}", report(source));
    let module = out.module.expect("a program");

    let mut walked = Vec::new();
    quench_interp::run_writing(&module, &mut walked).expect("it runs");
    let (_, compiled) = quench_dev::compile(&module).expect("it compiles").run_capturing();
    (String::from_utf8(walked).expect("text"), compiled)
}

fn said(source: &str) -> String {
    let (walked, compiled) = both(source);
    assert_eq!(walked, compiled, "the engines printed different things");
    walked
}

fn report(source: &str) -> String {
    let out = lower(source);
    quench_diag::report(&SourceFile::new("src/main.qnl", source), &out.errors)
}

#[test]
fn hello_world() {
    assert_eq!(said("START {\n    print[str:*Hello, World!* \\n];\n}\n"), "Hello, World!\n");
}

#[test]
fn the_pieces_are_printed_in_the_order_they_were_written() {
    // Nothing is joined first. The list is the list, and it goes out in order.
    assert_eq!(
        said("START {\n print[str:*one* \\n str:*two* \\n];\n}\n"),
        "one\ntwo\n"
    );
}

#[test]
fn every_escape_means_what_it_says() {
    assert_eq!(said("START { print[\\n]; }"), "\n");
    assert_eq!(said("START { print[\\t]; }"), "\t");
    assert_eq!(said("START { print[\\r]; }"), "\r");
    assert_eq!(said("START { print[\\\\]; }"), "\\");
}

#[test]
fn a_written_value_is_literal_and_only_its_mark_escapes() {
    // Between the marks everything is the character it looks like. `\n` in there is a
    // backslash and an `n`, which is the whole reason escapes stand outside.
    assert_eq!(said(r"START { print[str:*a\nb*]; }"), r"a\nb");
    assert_eq!(said(r"START { print[str:*two \* three*]; }"), "two * three");
    assert_eq!(said(r"START { print[str:*a \\ b*]; }"), r"a \ b");
}

#[test]
fn anything_you_can_type_survives_the_whole_way() {
    let source = "START { print[str:*🧑\u{200d}🧑\u{200d}🧒\u{200d}🧒 hi! {};|'#$%^& 12345*]; }";
    assert_eq!(said(source), "🧑\u{200d}🧑\u{200d}🧒\u{200d}🧒 hi! {};|'#$%^& 12345");
}

#[test]
fn the_same_text_twice_is_stored_once() {
    let out = lower("START { print[str:*hi* str:*hi* str:*ho*]; }");
    let module = out.module.expect("a program");
    assert_eq!(module.text.len(), 2, "{:?}", module.text);
    assert_eq!(said("START { print[str:*hi* str:*hi* str:*ho*]; }"), "hihiho");
}

#[test]
fn a_file_with_no_start_is_not_a_program() {
    // The parser is happy with this -- a file of declarations is a fine thing to parse.
    // Only something trying to *run* it knows that was not what was wanted.
    let out = quench_parse::parse("");
    assert!(out.ok(), "the parser has no complaint");

    let rendered = report("");
    assert!(rendered.contains("no `START`, so there is nothing to run"), "{rendered}");
    assert!(rendered.contains("It just cannot be run"), "{rendered}");
}

#[test]
fn the_parts_that_are_not_built_say_so_rather_than_failing_oddly() {
    let cases = [
        ("START { var.str ['a'] = [*x*]; }", "declaring things is not built yet"),
        ("START { print['a']; }", "printing a name is not built yet"),
        ("START { print[b16:*1*]; }", "`b16` is not built yet"),
    ];
    for (source, expected) in cases {
        let rendered = report(source);
        assert!(rendered.contains(expected), "{source}\n{rendered}");
    }
}

#[test]
fn a_program_that_prints_nothing_is_still_a_program() {
    assert_eq!(said("START { }"), "");
}
