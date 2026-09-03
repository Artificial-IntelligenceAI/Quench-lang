//! Whole programs, from what somebody typed to what the machine said.

use quench_diag::SourceFile;
use quench_lower::lower;

/// What both engines printed, on both streams, insisting they agree.
fn printed(source: &str) -> quench_dev::Printed {
    let out = lower(source);
    assert!(out.ok(), "{}", report(source));
    let module = out.module.expect("a program");

    let (mut out_bytes, mut err_bytes) = (Vec::new(), Vec::new());
    quench_interp::run_writing(
        &module,
        &mut quench_interp::Writing { out: &mut out_bytes, err: &mut err_bytes },
    )
    .expect("it runs");
    let walked = quench_dev::Printed {
        out: String::from_utf8(out_bytes).expect("text"),
        err: String::from_utf8(err_bytes).expect("text"),
    };

    let (_, compiled) = quench_dev::compile(&module).expect("it compiles").run_capturing();
    assert_eq!(walked, compiled, "the engines printed different things");
    walked
}

/// What a program wrote to standard output.
fn said(source: &str) -> String {
    printed(source).out
}

fn report(source: &str) -> String {
    let out = lower(source);
    quench_diag::report(&SourceFile::new("src/main.qnl", source), &out.errors)
}

#[test]
fn hello_world() {
    assert_eq!(said("START {\n    print.stdout[str:*Hello, World!* \\n];\n}\n"), "Hello, World!\n");
}

#[test]
fn the_pieces_are_printed_in_the_order_they_were_written() {
    // Nothing is joined first. The list is the list, and it goes out in order.
    assert_eq!(
        said("START {\n print.stdout[str:*one* \\n str:*two* \\n];\n}\n"),
        "one\ntwo\n"
    );
}

#[test]
fn every_escape_means_what_it_says() {
    assert_eq!(said("START { print.stdout[\\n]; }"), "\n");
    assert_eq!(said("START { print.stdout[\\t]; }"), "\t");
    assert_eq!(said("START { print.stdout[\\r]; }"), "\r");
    assert_eq!(said("START { print.stdout[\\\\]; }"), "\\");
}

#[test]
fn a_written_value_is_literal_and_only_its_mark_escapes() {
    // Between the marks everything is the character it looks like. `\n` in there is a
    // backslash and an `n`, which is the whole reason escapes stand outside.
    assert_eq!(said(r"START { print.stdout[str:*a\nb*]; }"), r"a\nb");
    assert_eq!(said(r"START { print.stdout[str:*two \* three*]; }"), "two * three");
    assert_eq!(said(r"START { print.stdout[str:*a \\ b*]; }"), r"a \ b");
}

#[test]
fn anything_you_can_type_survives_the_whole_way() {
    let source = "START { print.stdout[str:*🧑\u{200d}🧑\u{200d}🧒\u{200d}🧒 hi! {};|'#$%^& 12345*]; }";
    assert_eq!(said(source), "🧑\u{200d}🧑\u{200d}🧒\u{200d}🧒 hi! {};|'#$%^& 12345");
}

#[test]
fn the_same_text_twice_is_stored_once() {
    let out = lower("START { print.stdout[str:*hi* str:*hi* str:*ho*]; }");
    let module = out.module.expect("a program");
    assert_eq!(module.text.len(), 2, "{:?}", module.text);
    assert_eq!(said("START { print.stdout[str:*hi* str:*hi* str:*ho*]; }"), "hihiho");
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
        ("START { print.stdout[b16:*1*]; }", "`b16` is not built yet"),
        // Joining a name to something else builds a new value, which needs the collector.
        ("START { var.immut.str ['a'] = [*x*]; var.immut.str ['b'] = ['a' *y*]; }", "needs the collector"),
    ];
    for (source, expected) in cases {
        let rendered = report(source);
        assert!(rendered.contains(expected), "{source}\n{rendered}");
    }

    // And declaring things, which this test used to assert was not built, now is.
    assert!(lower("START { var.immut.str ['a'] = [*x*]; print.stdout['a']; }").ok());
}

#[test]
fn a_program_that_prints_nothing_is_still_a_program() {
    assert_eq!(said("START { }"), "");
}

#[test]
fn a_declaration_and_the_name_that_uses_it() {
    assert_eq!(
        said("START {\n var.immut.str ['greeting'] = [*Hello*];\n print.stdout['greeting' str:*, World!* \\n];\n}\n"),
        "Hello, World!\n"
    );
}

#[test]
fn a_number_prints_as_a_number() {
    assert_eq!(said("START { var.immut.i64 ['n'] = [*42*]; print.stdout['n']; }"), "42");
    assert_eq!(said("START { var.immut.i64 ['n'] = [*-7*]; print.stdout['n']; }"), "-7");
    assert_eq!(
        said("START { var.immut.i64 ['n'] = [*9223372036854775807*]; print.stdout['n']; }"),
        "9223372036854775807",
        "the whole range of an i64 survives the trip"
    );
}

#[test]
fn the_same_characters_are_a_number_or_text_depending_on_the_type() {
    // Which is the rule the marks exist for, running for the first time.
    assert_eq!(said("START { var.immut.i64 ['a'] = [*1000*]; print.stdout['a']; }"), "1000");
    assert_eq!(said("START { var.immut.str ['a'] = [*1000*]; print.stdout['a']; }"), "1000");
    // The same output, arrived at two different ways -- one printed a number, the other
    // printed four characters.
}

#[test]
fn copying_a_value_names_it_again_rather_than_building_anything() {
    assert_eq!(
        said("START { var.immut.str ['a'] = [*x*]; var.immut.str ['b'] = ['a']; print.stdout['a' 'b']; }"),
        "xx"
    );
    // And the module holds that text once, because copying did not make a second one.
    let out = lower("START { var.immut.str ['a'] = [*x*]; var.immut.str ['b'] = ['a']; print.stdout['a' 'b']; }");
    assert_eq!(out.module.expect("a program").text.len(), 1);
}

#[test]
fn a_declaration_that_is_never_used_still_has_to_make_sense() {
    assert!(!lower("START { var.immut.i64 ['n'] = [*hello*]; }").ok());
}

#[test]
fn a_program_that_does_not_check_out_is_not_lowered() {
    // Building QIR from a program that failed checking would make nonsense out of it,
    // and the nonsense would be reported by an engine rather than by the compiler.
    let out = lower("START { print.stdout['nope']; }");
    assert!(out.module.is_none());
    assert!(!out.errors.is_empty());
}

#[test]
fn arithmetic_comes_out_the_same_in_both_engines() {
    assert_eq!(said("START { var.immut.i64 ['n'] = [*7* + *3*]; print.stdout['n']; }"), "10");
    assert_eq!(said("START { var.immut.i64 ['n'] = [*7* - *3*]; print.stdout['n']; }"), "4");
    assert_eq!(said("START { var.immut.i64 ['n'] = [*7* x *3*]; print.stdout['n']; }"), "21");
    assert_eq!(said("START { var.immut.i64 ['n'] = [*7* / *3*]; print.stdout['n']; }"), "2");
    assert_eq!(said("START { var.immut.i64 ['n'] = [*7* mod *3*]; print.stdout['n']; }"), "1");
}

#[test]
fn precedence_shows_up_in_the_answer() {
    assert_eq!(said("START { var.immut.i64 ['n'] = [*1* + *2* x *3*]; print.stdout['n']; }"), "7");
    assert_eq!(said("START { var.immut.i64 ['n'] = [(*1* + *2*) x *3*]; print.stdout['n']; }"), "9");
    // Equal tiers go left to right, which is what everybody expects of subtraction.
    assert_eq!(said("START { var.immut.i64 ['n'] = [*10* - *3* - *2*]; print.stdout['n']; }"), "5");
}

#[test]
fn a_comparison_prints_as_a_word() {
    assert_eq!(said("START { var.immut.bool ['b'] = [*7* > *3*]; print.stdout['b']; }"), "true");
    assert_eq!(said("START { var.immut.bool ['b'] = [*7* < *3*]; print.stdout['b']; }"), "false");
    assert_eq!(said("START { var.immut.bool ['b'] = [*7* == *7*]; print.stdout['b']; }"), "true");
}

#[test]
fn the_division_setting_reaches_the_answer() {
    use quench_conf::{Division, Settings};
    let source = "START { var.immut.i64 ['n'] = [*0* - *7*]; var.immut.i64 ['q'] = ['n' / *2*]; print.stdout['q']; }";

    let truncated = quench_lower::lower_under(source, Settings::default());
    let floored = quench_lower::lower_under(
        source,
        Settings { division: Division::Floored, ..Settings::default() },
    );

    let ran = |lowered: quench_lower::Lowered| {
        let module = lowered.module.expect("a program");
        let (_, wrote) = quench_dev::compile(&module).expect("it compiles").run_capturing();
        wrote.out
    };

    // The same source, two different programs. Which is the whole reason a semantic
    // setting multiplies what the oracle has to prove.
    assert_eq!(ran(truncated), "-3", "toward zero");
    assert_eq!(ran(floored), "-4", "toward negative infinity");
}

#[test]
fn an_array_holds_what_it_was_given() {
    assert_eq!(
        said("START { var.immut.arr.i64 (3) ['xs'] = [[*10* *20* *30*]]; print.stdout['xs'[*2*]]; }"),
        "20"
    );
}

#[test]
fn a_shaped_array_is_written_flat_and_indexed_by_dimension() {
    // (2 3) is two rows of three, laid out row by row in one allocation. Element (2, 3)
    // is the last one, and finding it is arithmetic rather than following a handle.
    let source = "START {
        var.immut.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]];
        print.stdout['m'[*1* *1*] str:*,* 'm'[*1* *3*] str:*,* 'm'[*2* *1*] str:*,* 'm'[*2* *3*]];
    }";
    assert_eq!(said(source), "1,3,4,6");
}

#[test]
fn arrays_are_counted_from_one() {
    // Which is not a preference: a counting loop is inclusive and its counter unsigned,
    // so `[1, count]` walks an array exactly while `[0, count - 1]` wraps on an empty one.
    assert_eq!(
        said("START { var.immut.arr.i64 (2) ['xs'] = [[*7* *8*]]; print.stdout['xs'[*1*]]; }"),
        "7",
        "the first element is 1"
    );
}

#[test]
fn an_index_outside_the_array_stops_the_interpreter() {
    // The Dev JIT aborts instead, having nowhere to put a failure and no way to unwind.
    // That asymmetry is why the generator writes nothing that can stop, and it is the
    // thing to fix before it can.
    let out = lower("START { var.immut.arr.i64 (2) ['xs'] = [[*1* *2*]]; print.stdout['xs'[*3*]]; }");
    let module = out.module.expect("a program");
    assert_eq!(
        quench_interp::run(&module).expect("it runs"),
        quench_interp::Outcome::Trapped(quench_interp::Trap::OutsideTheArray)
    );

    let out = lower("START { var.immut.arr.i64 (2) ['xs'] = [[*1* *2*]]; print.stdout['xs'[*0*]]; }");
    assert_eq!(
        quench_interp::run(&out.module.expect("a program")).expect("it runs"),
        quench_interp::Outcome::Trapped(quench_interp::Trap::OutsideTheArray),
        "and nought is no element at all"
    );
}

/// What both engines say a program did, insisting they agree.
fn ended(source: &str) -> quench_qir::Outcome {
    let module = lower(source).module.expect("a program");
    let walked = quench_interp::run(&module).expect("it runs");
    let (compiled, _) = quench_dev::compile(&module).expect("it compiles").run_capturing();
    assert_eq!(walked, compiled, "the engines ended differently");
    walked
}

#[test]
fn stopping_is_agreed_on_as_much_as_answering() {
    use quench_qir::{Outcome, Trap};

    // Not merely *that* it stopped -- which stop it was. An engine that said "something
    // went wrong" could not be compared with one that said what.
    assert_eq!(
        ended("START { var.immut.i64 ['z'] = [*0*]; var.immut.i64 ['q'] = [*1* / 'z']; print.stdout['q']; }"),
        Outcome::Trapped(Trap::DividedByZero)
    );
    assert_eq!(
        ended("START { var.immut.i64 ['z'] = [*0*]; var.immut.i64 ['q'] = [*1* mod 'z']; print.stdout['q']; }"),
        Outcome::Trapped(Trap::DividedByZero)
    );
    assert_eq!(
        ended("START { var.immut.arr.i64 (2) ['xs'] = [[*1* *2*]]; print.stdout['xs'[*9*]]; }"),
        Outcome::Trapped(Trap::OutsideTheArray)
    );
    assert_eq!(
        ended("START { var.immut.arr.i64 (2) ['xs'] = [[*1* *2*]]; print.stdout['xs'[*0*]]; }"),
        Outcome::Trapped(Trap::OutsideTheArray),
        "counted from one, so nought is no element"
    );
}

#[test]
fn the_one_division_that_does_not_fit() {
    use quench_qir::{Outcome, Trap};
    // i64::MIN / -1 is one larger than an i64 holds. It is a different stop from
    // dividing by zero, and both engines have to know which happened.
    let source = "START {
        var.immut.i64 ['least'] = [*-9223372036854775808*];
        var.immut.i64 ['minus'] = [*0* - *1*];
        var.immut.i64 ['q'] = ['least' / 'minus'];
        print.stdout['q'];
    }";
    assert_eq!(ended(source), Outcome::Trapped(Trap::DivisionOverflowed));
}

#[test]
fn a_program_that_stops_stops_where_it_stopped() {
    // What ran before the stop happened; what came after did not.
    let source = "START {
        print.stdout[str:*before* \\n];
        var.immut.i64 ['z'] = [*0*];
        var.immut.i64 ['q'] = [*1* / 'z'];
        print.stdout[str:*after* \\n];
    }";
    let module = lower(source).module.expect("a program");
    let (outcome, wrote) = quench_dev::compile(&module).expect("it compiles").run_capturing();
    assert_eq!(wrote.out, "before\n", "and nothing after it");
    assert!(matches!(outcome, quench_qir::Outcome::Trapped(_)));
}

#[test]
fn setting_a_variable_changes_what_it_holds() {
    assert_eq!(
        said("START { var.mut.i64 ['n'] = [*1*]; set ['n'] = ['n' + *41*]; print.stdout['n']; }"),
        "42"
    );
    assert_eq!(
        said("START { var.mut.str ['s'] = [*a*]; set ['s'] = [*b*]; print.stdout['s']; }"),
        "b"
    );
}

#[test]
fn setting_an_element_changes_only_that_one() {
    let source = "START {
        var.mut.arr.i64 (3) ['xs'] = [[*1* *2* *3*]];
        set ['xs'[*2*]] = [*99*];
        print.stdout['xs'[*1*] str:*,* 'xs'[*2*] str:*,* 'xs'[*3*]];
    }";
    assert_eq!(said(source), "1,99,3");
}

#[test]
fn setting_an_element_outside_the_array_stops_both_engines() {
    use quench_qir::{Outcome, Trap};
    assert_eq!(
        ended("START { var.mut.arr.i64 (2) ['xs'] = [[*1* *2*]]; set ['xs'[*7*]] = [*0*]; }"),
        Outcome::Trapped(Trap::OutsideTheArray)
    );
}

#[test]
fn the_overflow_setting_reaches_the_answer() {
    use quench_conf::{Overflow, Settings};
    use quench_qir::{Outcome, Trap};
    let source = "START {
        var.immut.i64 ['big'] = [*9223372036854775807*];
        var.immut.i64 ['n'] = ['big' + *1*];
        print.stdout['n'];
    }";

    let ran = |settings: Settings| {
        let module = quench_lower::lower_under(source, settings).module.expect("a program");
        let walked = quench_interp::run_named(&module, quench_qir::ENTRY).expect("it runs");
        let (compiled, _) = quench_dev::compile(&module).expect("it compiles").run_capturing();
        assert_eq!(walked, compiled, "the engines disagree");
        walked
    };

    // One program, two languages -- which is what a semantic setting is.
    assert!(matches!(
        ran(Settings { overflow: Overflow::Wrap, ..Settings::default() }),
        Outcome::Returned(_)
    ));
    assert_eq!(
        ran(Settings { overflow: Overflow::Trap, ..Settings::default() }),
        Outcome::Trapped(Trap::Overflowed)
    );
}

#[test]
fn exactly_one_arm_runs() {
    let source = "START {
        var.immut.i64 ['n'] = [*12*];
        if 'n' > *100* { print.stdout[str:*huge*]; }
        else-if 'n' > *10* { print.stdout[str:*big*]; }
        else-if 'n' > *1*  { print.stdout[str:*small*]; }
        else { print.stdout[str:*tiny*]; }
    }";
    assert_eq!(said(source), "big", "the first that held, and no other");
}

#[test]
fn an_if_with_no_else_may_do_nothing_at_all() {
    assert_eq!(said("START { if *1* > *2* { print.stdout[str:*no*]; } print.stdout[str:*after*]; }"), "after");
    assert_eq!(said("START { if *2* > *1* { print.stdout[str:*yes*]; } print.stdout[str:*after*]; }"), "yesafter");
}

#[test]
fn a_variable_changed_in_one_arm_is_changed_after_it() {
    // Which is the whole of the block-parameter work: `label` is a different value
    // depending on which arm ran, and afterwards it is whichever one that was.
    let source = "START {
        var.immut.i64 ['n'] = [*12*];
        var.mut.i64 ['label'] = [*0*];
        if 'n' > *100* { set ['label'] = [*3*]; }
        else-if 'n' > *10* { set ['label'] = [*2*]; }
        else { set ['label'] = [*1*]; }
        print.stdout['label'];
    }";
    assert_eq!(said(source), "2");
}

#[test]
fn a_variable_left_alone_in_an_arm_keeps_what_it_had() {
    let source = "START {
        var.mut.i64 ['n'] = [*7*];
        if *1* > *2* { set ['n'] = [*99*]; }
        print.stdout['n'];
    }";
    assert_eq!(said(source), "7");
}

#[test]
fn conditionals_nest() {
    let source = "START {
        var.immut.i64 ['a'] = [*2*];
        var.mut.str ['out'] = [*none*];
        if 'a' > *1* {
            if 'a' > *5* { set ['out'] = [*both*]; }
            else { set ['out'] = [*outer only*]; }
        }
        print.stdout['out'];
    }";
    assert_eq!(said(source), "outer only");
}

#[test]
fn an_arm_can_change_an_array_element() {
    let source = "START {
        var.mut.arr.i64 (3) ['xs'] = [[*1* *2* *3*]];
        if *2* > *1* { set ['xs'[*2*]] = [*99*]; }
        print.stdout['xs'[*1*] str:*,* 'xs'[*2*] str:*,* 'xs'[*3*]];
    }";
    assert_eq!(said(source), "1,99,3");
}

#[test]
fn a_program_says_where_its_output_goes() {
    // Which is the whole reason the destination is written down. Go's built-in
    // `println` writes to standard error and nothing about writing it says so.
    let wrote = printed(
        "START { print.stdout[str:*answer*]; print.stderr[str:*complaint*]; }",
    );
    assert_eq!(wrote.out, "answer");
    assert_eq!(wrote.err, "complaint");
}

#[test]
fn the_two_streams_do_not_leak_into_each_other() {
    assert_eq!(printed("START { print.stderr[str:*only here*]; }").out, "");
    assert_eq!(printed("START { print.stdout[str:*only here*]; }").err, "");
}
