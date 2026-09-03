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
    // Nothing before this point complains -- a file of declarations is a fine thing to
    // parse and a fine thing to check. Only something trying to *run* it knows that was
    // not what was wanted.
    assert!(quench_check::check("").ok(), "checking has no complaint either");

    let rendered = report("");
    assert!(rendered.contains("no `START`, so there is nothing to run"), "{rendered}");
    assert!(rendered.contains("It just cannot be run"), "{rendered}");
}

#[test]
fn the_parts_that_are_not_built_say_so_rather_than_failing_oddly() {
    let cases = [
        // A type Quench means to have and does not have yet.
        ("START { print[b16:*1*]; }", "`b16` is not built yet"),
        // Joining a name to something else builds a new value, which needs the collector.
        ("START { var.str ['a'] = [*x*]; var.str ['b'] = ['a' *y*]; }", "needs the collector"),
    ];
    for (source, expected) in cases {
        let rendered = report(source);
        assert!(rendered.contains(expected), "{source}\n{rendered}");
    }

    // And declaring things, which this test used to assert was not built, now is.
    assert!(lower("START { var.str ['a'] = [*x*]; print['a']; }").ok());
}

#[test]
fn a_program_that_prints_nothing_is_still_a_program() {
    assert_eq!(said("START { }"), "");
}

#[test]
fn a_declaration_and_the_name_that_uses_it() {
    assert_eq!(
        said("START {\n var.str ['greeting'] = [*Hello*];\n print['greeting' str:*, World!* \\n];\n}\n"),
        "Hello, World!\n"
    );
}

#[test]
fn a_number_prints_as_a_number() {
    assert_eq!(said("START { var.i64 ['n'] = [*42*]; print['n']; }"), "42");
    assert_eq!(said("START { var.i64 ['n'] = [*-7*]; print['n']; }"), "-7");
    assert_eq!(
        said("START { var.i64 ['n'] = [*9223372036854775807*]; print['n']; }"),
        "9223372036854775807",
        "the whole range of an i64 survives the trip"
    );
}

#[test]
fn the_same_characters_are_a_number_or_text_depending_on_the_type() {
    // Which is the rule the marks exist for, running for the first time.
    assert_eq!(said("START { var.i64 ['a'] = [*1000*]; print['a']; }"), "1000");
    assert_eq!(said("START { var.str ['a'] = [*1000*]; print['a']; }"), "1000");
    // The same output, arrived at two different ways -- one printed a number, the other
    // printed four characters.
}

#[test]
fn copying_a_value_names_it_again_rather_than_building_anything() {
    assert_eq!(
        said("START { var.str ['a'] = [*x*]; var.str ['b'] = ['a']; print['a' 'b']; }"),
        "xx"
    );
    // And the module holds that text once, because copying did not make a second one.
    let out = lower("START { var.str ['a'] = [*x*]; var.str ['b'] = ['a']; print['a' 'b']; }");
    assert_eq!(out.module.expect("a program").text.len(), 1);
}

#[test]
fn a_declaration_that_is_never_used_still_has_to_make_sense() {
    assert!(!lower("START { var.i64 ['n'] = [*hello*]; }").ok());
}

#[test]
fn a_program_that_does_not_check_out_is_not_lowered() {
    // Building QIR from a program that failed checking would make nonsense out of it,
    // and the nonsense would be reported by an engine rather than by the compiler.
    let out = lower("START { print['nope']; }");
    assert!(out.module.is_none());
    assert!(!out.errors.is_empty());
}

#[test]
fn arithmetic_comes_out_the_same_in_both_engines() {
    assert_eq!(said("START { var.i64 ['n'] = [*7* + *3*]; print['n']; }"), "10");
    assert_eq!(said("START { var.i64 ['n'] = [*7* - *3*]; print['n']; }"), "4");
    assert_eq!(said("START { var.i64 ['n'] = [*7* x *3*]; print['n']; }"), "21");
    assert_eq!(said("START { var.i64 ['n'] = [*7* / *3*]; print['n']; }"), "2");
    assert_eq!(said("START { var.i64 ['n'] = [*7* mod *3*]; print['n']; }"), "1");
}

#[test]
fn precedence_shows_up_in_the_answer() {
    assert_eq!(said("START { var.i64 ['n'] = [*1* + *2* x *3*]; print['n']; }"), "7");
    assert_eq!(said("START { var.i64 ['n'] = [(*1* + *2*) x *3*]; print['n']; }"), "9");
    // Equal tiers go left to right, which is what everybody expects of subtraction.
    assert_eq!(said("START { var.i64 ['n'] = [*10* - *3* - *2*]; print['n']; }"), "5");
}

#[test]
fn a_comparison_prints_as_a_word() {
    assert_eq!(said("START { var.bool ['b'] = [*7* > *3*]; print['b']; }"), "true");
    assert_eq!(said("START { var.bool ['b'] = [*7* < *3*]; print['b']; }"), "false");
    assert_eq!(said("START { var.bool ['b'] = [*7* == *7*]; print['b']; }"), "true");
}

#[test]
fn the_division_setting_reaches_the_answer() {
    use quench_conf::{Division, Settings};
    let source = "START { var.i64 ['n'] = [*0* - *7*]; var.i64 ['q'] = ['n' / *2*]; print['q']; }";

    let truncated = quench_lower::lower_under(source, Settings::default());
    let floored = quench_lower::lower_under(
        source,
        Settings { division: Division::Floored, ..Settings::default() },
    );

    let ran = |lowered: quench_lower::Lowered| {
        let module = lowered.module.expect("a program");
        let (_, said) = quench_dev::compile(&module).expect("it compiles").run_capturing();
        let mut walked = Vec::new();
        quench_interp::run_writing(&module, &mut walked).expect("it runs");
        assert_eq!(said, String::from_utf8(walked).expect("text"), "the engines disagree");
        said
    };

    // The same source, two different programs. Which is the whole reason a semantic
    // setting multiplies what the oracle has to prove.
    assert_eq!(ran(truncated), "-3", "toward zero");
    assert_eq!(ran(floored), "-4", "toward negative infinity");
}

#[test]
fn an_array_holds_what_it_was_given() {
    assert_eq!(
        said("START { var.arr.i64 (3) ['xs'] = [[*10* *20* *30*]]; print['xs'[*2*]]; }"),
        "20"
    );
}

#[test]
fn a_shaped_array_is_written_flat_and_indexed_by_dimension() {
    // (2 3) is two rows of three, laid out row by row in one allocation. Element (2, 3)
    // is the last one, and finding it is arithmetic rather than following a handle.
    let source = "START {
        var.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]];
        print['m'[*1* *1*] str:*,* 'm'[*1* *3*] str:*,* 'm'[*2* *1*] str:*,* 'm'[*2* *3*]];
    }";
    assert_eq!(said(source), "1,3,4,6");
}

#[test]
fn arrays_are_counted_from_one() {
    // Which is not a preference: a counting loop is inclusive and its counter unsigned,
    // so `[1, count]` walks an array exactly while `[0, count - 1]` wraps on an empty one.
    assert_eq!(
        said("START { var.arr.i64 (2) ['xs'] = [[*7* *8*]]; print['xs'[*1*]]; }"),
        "7",
        "the first element is 1"
    );
}

#[test]
fn an_index_outside_the_array_stops_the_interpreter() {
    // The Dev JIT aborts instead, having nowhere to put a failure and no way to unwind.
    // That asymmetry is why the generator writes nothing that can stop, and it is the
    // thing to fix before it can.
    let out = lower("START { var.arr.i64 (2) ['xs'] = [[*1* *2*]]; print['xs'[*3*]]; }");
    let module = out.module.expect("a program");
    assert_eq!(
        quench_interp::run(&module).expect("it runs"),
        quench_interp::Outcome::Trapped(quench_interp::Trap::OutsideTheArray)
    );

    let out = lower("START { var.arr.i64 (2) ['xs'] = [[*1* *2*]]; print['xs'[*0*]]; }");
    assert_eq!(
        quench_interp::run(&out.module.expect("a program")).expect("it runs"),
        quench_interp::Outcome::Trapped(quench_interp::Trap::OutsideTheArray),
        "and nought is no element at all"
    );
}
