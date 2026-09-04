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
        // A word that is not a type. There is no longer a second answer here -- every
        // type Quench meant to have is built, so "not built yet" has nothing to say.
        ("START { print.stdout[text:*1*]; }", "`text` is not a type"),
        // A number is not text, and nothing converts on its own.
        ("START { var.immut.i64 ['n'] = [*1*]; var.immut.str ['b'] = [*x* 'n']; }", "text is made of text"),
    ];
    for (source, expected) in cases {
        let rendered = report(source);
        assert!(rendered.contains(expected), "{source}\n{rendered}");
    }

    // And joining, which this test used to assert was not built, now is. So is `d32`,
    // which it named as the type that was not.
    assert!(lower("START { var.immut.str ['a'] = [*x*]; var.immut.str ['b'] = ['a' *y*]; }").ok());
    assert!(lower("START { print.stdout[d32:*1*]; }").ok());
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

#[test]
fn a_counting_loop_includes_both_ends() {
    assert_eq!(
        said("START {\n    loop.temp.range.i64 ['i'] = [*1*, *5*] {\n        print.stdout['i'];\n    }\n}\n"),
        "12345",
    );
}

#[test]
fn a_loop_carries_what_it_changed_out_of_itself() {
    assert_eq!(
        said("\
START {
    var.mut.i64 ['total'] = [*0*];
    loop.temp.range.i64 ['i'] = [*1*, *10*] {
        set ['total'] = ['total' + 'i'];
    }
    print.stdout['total'];
}
"),
        "55",
    );
}

#[test]
fn a_range_whose_start_is_past_its_end_runs_no_passes() {
    assert_eq!(
        said("\
START {
    loop.perm.range.i64 ['i'] = [*1*, *0*] {
        print.stdout[str:*never*];
    }
    print.stdout['i'];
}
"),
        // The counter never took a value, so `perm` holds where it would have started.
        "1",
    );
}

#[test]
fn a_perm_counter_holds_the_last_value_it_took() {
    // Five, not six. The counter is one past the end by the time the loop stops, and
    // nobody who wrote five means six -- which is the whole reason `perm` costs a
    // parameter that `temp` does not.
    assert_eq!(
        said("START {\n    loop.perm.range.i64 ['i'] = [*1*, *5*] { }\n    print.stdout['i'];\n}\n"),
        "5",
    );
    assert_eq!(
        said("\
START {
    loop.perm.range.i64 ['i'] = [*1*, *100*] {
        if 'i' == *3* { break; }
    }
    print.stdout['i'];
}
"),
        "3",
        "which is what `perm` is for: after an early `break`, where it stopped",
    );
}

#[test]
fn break_leaves_the_nearest_loop_and_no_further() {
    assert_eq!(
        said("\
START {
    loop.temp.range.i64 ['r'] = [*1*, *3*] {
        loop.temp.range.i64 ['c'] = [*1*, *4*] {
            if 'c' > *2* { break; }
            print.stdout['r' 'c'];
        }
    }
}
"),
        "111221223132",
    );
}

#[test]
fn an_if_whose_every_arm_leaves_still_lets_the_loop_end() {
    // Nothing comes out of the far side of that `if`, so the block it would have joined
    // into is unreachable -- and an unreachable block still has to be well formed.
    assert_eq!(
        said("\
START {
    var.mut.i64 ['seen'] = [*0*];
    loop.temp.range.i64 ['i'] = [*1*, *9*] {
        set ['seen'] = ['seen' + *1*];
        if 'i' == *4* { break; } else { print.stdout['i']; }
    }
    print.stdout[str:*/* 'seen'];
}
"),
        "123/4",
    );
}

#[test]
fn a_while_loop_asks_again_before_every_pass() {
    assert_eq!(
        said("\
START {
    var.mut.i64 ['n'] = [*1*];
    var.mut.i64 ['steps'] = [*0*];
    loop.while 'n' < *100* {
        set ['n'] = ['n' x *3*];
        set ['steps'] = ['steps' + *1*];
    }
    print.stdout['n' str:* in * 'steps'];
}
"),
        "243 in 5",
    );
    assert_eq!(
        said("\
START {
    var.immut.bool ['never'] = [*false*];
    loop.while 'never' {
        print.stdout[str:*no*];
    }
    print.stdout[str:*done*];
}
"),
        "done",
        "the question comes before the first pass, not after it",
    );
}

#[test]
fn a_body_may_declare_things_of_its_own() {
    // Which are gone at the closing brace, and whose values must not be carried past it
    // -- a join handed something defined inside the loop would be reaching where it
    // cannot see.
    assert_eq!(
        said("\
START {
    var.mut.i64 ['last'] = [*0*];
    loop.temp.range.i64 ['j'] = [*1*, *3*] {
        var.immut.i64 ['ten'] = ['j' x *10*];
        set ['last'] = ['ten'];
        print.stdout['ten' str:* *];
    }
    if 'last' == *30* { print.stdout[str:*kept*]; }
}
"),
        "10 20 30 kept",
    );
}

#[test]
fn count_bounds_a_loop_over_an_array() {
    assert_eq!(
        said("\
START {
    var.immut.arr.i64 (4) ['xs'] = [[*7* *8* *9* *10*]];
    var.mut.i64 ['sum'] = [*0*];
    loop.temp.range.i64 ['n'] = [*1*, call count['xs']] {
        set ['sum'] = ['sum' + 'xs'['n']];
    }
    print.stdout['sum'];
}
"),
        "34",
    );
}

#[test]
fn a_function_gives_an_answer_back() {
    assert_eq!(
        said("\
fn.file.i64 ['add'] [immut.i64 'a', immut.i64 'b'] {
    give ['a' + 'b'];
}
START {
    print.stdout[call 'add'[*1*, *2*]];
}
"),
        "3",
    );
}

#[test]
fn a_function_may_call_itself() {
    assert_eq!(
        said("\
fn.file.i64 ['factorial'] [immut.i64 'n'] {
    if 'n' <== *1* { give [*1*]; } else { give ['n' x call 'factorial'['n' - *1*]]; }
}
START {
    print.stdout[call 'factorial'[*10*]];
}
"),
        "3628800",
    );
}

#[test]
fn two_functions_may_call_each_other() {
    // Which needs every signature read before any body is, and is why they are.
    assert_eq!(
        said("\
fn.file.bool ['even'] [immut.i64 'n'] {
    if 'n' == *0* { give [*true*]; } else { give [call 'odd'['n' - *1*]]; }
}
fn.file.bool ['odd'] [immut.i64 'n'] {
    if 'n' == *0* { give [*false*]; } else { give [call 'even'['n' - *1*]]; }
}
START {
    print.stdout[call 'even'[*10*] str:* * call 'odd'[*10*]];
}
"),
        "true false",
    );
}

#[test]
fn a_function_that_gives_nothing_is_called_on_its_own() {
    assert_eq!(
        said("\
fn.file.nothing ['greet'] [immut.str 'name'] {
    print.stdout[str:*Hello, * 'name' str:*!*];
}
START {
    call 'greet'[*Tankun*];
}
"),
        "Hello, Tankun!",
    );
}

#[test]
fn give_leaves_a_function_early() {
    assert_eq!(
        said("\
fn.file.nothing ['upto'] [immut.i64 'stop'] {
    loop.temp.range.i64 ['i'] = [*1*, *9*] {
        if 'i' > 'stop' { give; }
        print.stdout['i'];
    }
}
START {
    call 'upto'[*3*];
    give;
}
"),
        "123",
    );
}

#[test]
fn a_constant_is_written_in_where_it_is_named() {
    assert_eq!(
        said("\
const.export.i64 ['LIMIT'] = [*10*];
const.file.str ['NAME'] = [*Quench*];

fn.file.bool ['past'] [immut.i64 'n'] {
    give ['n' > 'LIMIT'];
}
START {
    print.stdout['NAME' str:* * 'LIMIT' str:* * call 'past'[*42*]];
}
"),
        "Quench 10 true",
    );
}

#[test]
fn a_parameter_may_be_changed_without_the_caller_seeing_it() {
    // Nothing here is a reference yet, so `mut` on a parameter changes this function's
    // copy and stops there.
    assert_eq!(
        said("\
fn.file.i64 ['doubled'] [mut.i64 'n'] {
    set ['n'] = ['n' x *2*];
    give ['n'];
}
START {
    var.immut.i64 ['mine'] = [*21*];
    print.stdout[call 'doubled'['mine'] str:* * 'mine'];
}
"),
        "42 21",
    );
}

#[test]
fn an_exact_number_never_rounds() {
    // A third times three is one. Which is the whole of what `e` is for.
    assert_eq!(
        said("\
START {
    var.immut.e ['third'] = [*1* / *3*];
    var.immut.e ['back'] = ['third' x *3*];
    print.stdout['third' str:* * 'back'];
}
"),
        "1/3 1",
    );
}

#[test]
fn the_one_every_language_is_famous_for() {
    assert_eq!(
        said("\
START {
    var.immut.e ['sum'] = [e:*0.1* + e:*0.2*];
    var.immut.bool ['right'] = [e:*0.1* + e:*0.2* == e:*0.3*];
    print.stdout['sum' str:* * 'right'];
}
"),
        "3/10 true",
        "a decimal point is exact here, which is the reason to write one",
    );
}

#[test]
fn an_exact_number_is_as_big_as_it_needs_to_be() {
    assert_eq!(
        said("\
START {
    var.immut.e ['huge'] = [*99999999999999999999999999999999*];
    var.immut.e ['squared'] = ['huge' x 'huge'];
    print.stdout['squared'];
}
"),
        "9999999999999999999999999999999800000000000000000000000000000001",
    );
}

#[test]
fn a_whole_exact_number_wears_no_denominator() {
    assert_eq!(
        said("\
START {
    var.immut.e ['a'] = [*6* / *3*];
    var.immut.e ['b'] = [*-3* / *4*];
    var.immut.e ['c'] = [*0.5* + *0.5*];
    print.stdout['a' str:* * 'b' str:* * 'c'];
}
"),
        "2 -3/4 1",
    );
}

#[test]
fn exact_numbers_compare_exactly() {
    assert_eq!(
        said("\
START {
    var.immut.e ['third'] = [*1* / *3*];
    var.immut.e ['half'] = [*1* / *2*];
    print.stdout['third' str:*<* 'half' str:*: *];
    var.immut.bool ['less'] = ['third' < 'half'];
    var.immut.bool ['same'] = ['third' == 'third'];
    print.stdout['less' str:* * 'same'];
}
"),
        "1/3<1/2: true true",
    );
}

#[test]
fn a_power_binds_tightest_and_answers_by_squaring() {
    // Which mathematics settled long before computers, so Quench does not choose it.
    assert_eq!(
        said("START {\n    var.immut.i64 ['n'] = [*2* + *3* ^ *2*];\n    print.stdout['n'];\n}\n"),
        "11",
        "2 + (3^2), not (2 + 3)^2",
    );
}

#[test]
fn an_exact_number_takes_a_negative_power_and_a_whole_one_does_not() {
    // Two to the minus one is a half, and a half is a number an `e` holds.
    assert_eq!(
        said("\
START {
    var.immut.e ['half'] = [*2* ^ *-1*];
    var.immut.e ['big'] = [*2* ^ *100*];
    var.immut.e ['cube'] = [e:*2/3* ^ *3*];
    print.stdout['half' str:* * 'big' str:* * 'cube'];
}
"),
        "1/2 1267650600228229401496703205376 8/27",
    );
    assert_eq!(
        ended("START {\n    var.immut.i64 ['n'] = [*2* ^ *-1*];\n    print.stdout['n'];\n}\n"),
        quench_qir::Outcome::Trapped(quench_qir::Trap::NegativePower),
        "and a whole number does not: the answer to that is a fraction",
    );
}

/// The same, under settings of the caller's choosing.
fn said_under(source: &str, settings: quench_conf::Settings) -> String {
    let out = quench_lower::lower_under(source, settings);
    assert!(out.ok(), "{}", report(source));
    let module = out.module.expect("a program");

    let (mut out_bytes, mut err_bytes) = (Vec::new(), Vec::new());
    quench_interp::run_writing(
        &module,
        &mut quench_interp::Writing { out: &mut out_bytes, err: &mut err_bytes },
    )
    .expect("it runs");
    let walked = String::from_utf8(out_bytes).expect("text");
    let _ = err_bytes;

    let (_, compiled) = quench_dev::compile(&module).expect("it compiles").run_capturing();
    assert_eq!(walked, compiled.out, "the engines printed different things");
    walked
}

#[test]
fn and_or_and_not_answer() {
    assert_eq!(
        said("\
START {
    var.immut.bool ['t'] = [*true*];
    var.immut.bool ['f'] = [*false*];
    var.immut.bool ['a'] = ['t' and 'f'];
    var.immut.bool ['o'] = ['t' or 'f'];
    var.immut.bool ['n'] = [not 't'];
    print.stdout['a' str:* * 'o' str:* * 'n'];
}
"),
        "false true false",
    );
}

#[test]
fn whether_the_right_side_is_asked_is_a_setting() {
    // Which only became a question a program could see once it could call a function.
    // Before that, nothing inside an expression could do anything.
    let source = "\
fn.file.bool ['shout'] [immut.bool 'answer'] {
    print.stdout[str:*(asked)*];
    give ['answer'];
}
START {
    var.immut.bool ['f'] = [*false*];
    var.immut.bool ['x'] = ['f' and call 'shout'[*true*]];
    print.stdout[str:*/* 'x'];
}
";
    let early = quench_conf::Settings {
        logic: quench_conf::Logic::StopsEarly,
        ..quench_conf::Settings::default()
    };
    let both = quench_conf::Settings {
        logic: quench_conf::Logic::AsksBoth,
        ..quench_conf::Settings::default()
    };
    assert_eq!(said_under(source, early), "/false");
    assert_eq!(said_under(source, both), "(asked)/false");
}

#[test]
fn stopping_early_is_what_makes_a_guard_a_guard() {
    // Quench stops rather than having undefined behaviour, so the difference between
    // the two settings here is not speed -- it is whether the program survives.
    let source = "\
START {
    var.immut.i64 ['zero'] = [*0*];
    var.immut.bool ['safe'] = [('zero' !== *0*) and ((*100* / 'zero') > *5*)];
    print.stdout['safe'];
}
";
    let early = quench_conf::Settings {
        logic: quench_conf::Logic::StopsEarly,
        ..quench_conf::Settings::default()
    };
    assert_eq!(said_under(source, early), "false");

    let both = quench_conf::Settings {
        logic: quench_conf::Logic::AsksBoth,
        ..quench_conf::Settings::default()
    };
    let out = quench_lower::lower_under(source, both);
    assert_eq!(
        quench_interp::run(&out.module.expect("a program")).expect("it runs"),
        quench_interp::Outcome::Trapped(quench_interp::Trap::DividedByZero),
        "the guard is not a guard when both sides are always asked",
    );
}

#[test]
fn an_array_prints_everything_it_holds() {
    // Flat, however many dimensions it has, because flat is how the elements are
    // written: `[*1* *2* *3* *4* *5* *6*]` is a `(2 3)`, and nesting the output would
    // show a shape the input deliberately does not.
    assert_eq!(
        said("\
START {
    var.immut.arr.i64 (3) ['xs'] = [[*10* *20* *30*]];
    var.immut.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]];
    print.stdout['xs' str:* * 'm'];
}
"),
        "[10 20 30] [1 2 3 4 5 6]",
    );
}

#[test]
fn everything_that_can_be_the_same_can_be_compared() {
    // `==` used to work on `i64` and `e` and reach the IR verifier on everything else.
    assert_eq!(
        said("\
START {
    var.immut.str ['a'] = [*hello*];
    var.immut.str ['b'] = [*hello*];
    var.immut.str ['c'] = [*world*];
    var.immut.bool ['t'] = [*true*];
    var.immut.e ['half'] = [*1/2*];
    var.immut.e ['point'] = [*0.5*];

    var.immut.bool ['text'] = ['a' == 'b'];
    var.immut.bool ['other'] = ['a' !== 'c'];
    var.immut.bool ['flags'] = ['t' == 'text'];
    var.immut.bool ['exact'] = ['half' == 'point'];
    print.stdout['text' str:* * 'other' str:* * 'flags' str:* * 'exact'];
}
"),
        "true true true true",
        "and a half written two ways is one number",
    );
}

#[test]
fn share_makes_a_second_name_and_copy_makes_a_second_array() {
    assert_eq!(
        said("\
START {
    var.mut.arr.i64 (2) ['a'] = [[*1* *2*]];
    var.mut.arr.i64 (2) ['shared'] = [share 'a'];
    var.mut.arr.i64 (2) ['mine'] = [copy 'a'];
    set ['a'[*1*]] = [*99*];
    print.stdout['a' str:* * 'shared' str:* * 'mine'];
}
"),
        "[99 2] [99 2] [1 2]",
    );
}

#[test]
fn two_arrays_are_equal_when_they_hold_the_same_things() {
    // Not when they are the same array. `share` is what makes two names for one, and
    // this is the other question.
    assert_eq!(
        said("\
START {
    var.immut.arr.i64 (2) ['a'] = [[*1* *2*]];
    var.immut.arr.i64 (2) ['twin'] = [[*1* *2*]];
    var.immut.arr.i64 (2) ['other'] = [[*1* *3*]];
    var.immut.bool ['same'] = ['a' == 'twin'];
    var.immut.bool ['not'] = ['a' !== 'other'];
    print.stdout['same' str:* * 'not'];
}
"),
        "true true",
    );
}

#[test]
fn an_array_holds_any_type_that_is_built() {
    // Text wears its marks inside an array, because `[hello there world]` cannot be
    // read and `[*hello there* *world*]` can.
    assert_eq!(
        said("\
START {
    var.immut.arr.bool (3) ['flags'] = [[*true* *false* *true*]];
    var.immut.arr.str (2) ['words'] = [[*hello there* *world*]];
    var.immut.arr.e (3) ['exact'] = [[*1/3* *0.5* *7*]];
    print.stdout['flags' str:* * 'words' str:* * 'exact'];
}
"),
        "[true false true] [*hello there* *world*] [1/3 1/2 7]",
    );
}

#[test]
fn elements_come_back_out_as_what_they_are() {
    assert_eq!(
        said("\
START {
    var.immut.arr.bool (2) ['flags'] = [[*true* *false*]];
    var.immut.arr.str (2) ['words'] = [[*one* *two*]];
    var.immut.arr.e (2) ['exact'] = [[*1/3* *2*]];
    print.stdout['flags'[*1*] str:* * 'words'[*2*] str:* * 'exact'[*1*]];
}
"),
        "true two 1/3",
    );
}

#[test]
fn arrays_of_exact_numbers_compare_by_what_they_hold() {
    // A half written two ways is one number, inside an array as well as outside one.
    assert_eq!(
        said("\
START {
    var.immut.arr.e (2) ['a'] = [[*1/2* *2*]];
    var.immut.arr.e (2) ['b'] = [[*0.5* *2*]];
    var.immut.bool ['same'] = ['a' == 'b'];
    print.stdout['same'];
}
"),
        "true",
    );
}

#[test]
fn an_array_crosses_into_a_function_and_the_call_says_how() {
    assert_eq!(
        said("\
fn.file.i64 ['total'] [immut.arr.i64 (4) 'xs'] {
    var.mut.i64 ['sum'] = [*0*];
    loop.temp.range.i64 ['i'] = [*1*, call count['xs']] {
        set ['sum'] = ['sum' + 'xs'['i']];
    }
    give ['sum'];
}
fn.file.nothing ['zero_it'] [mut.arr.i64 (4) 'xs'] {
    loop.temp.range.i64 ['i'] = [*1*, call count['xs']] {
        set ['xs'['i']] = [*0*];
    }
}
START {
    var.mut.arr.i64 (4) ['xs'] = [[*1* *2* *3* *4*]];
    print.stdout[call 'total'[share 'xs'] str:* *];
    call 'zero_it'[copy 'xs'];
    print.stdout['xs' str:* *];
    call 'zero_it'[share 'xs'];
    print.stdout['xs'];
}
"),
        "10 [1 2 3 4] [0 0 0 0]",
        "which is the whole point of `share` and `copy`: the call site says",
    );
}

#[test]
fn an_array_comes_back_out_of_a_function() {
    assert_eq!(
        said("\
fn.file.arr.i64 (3) ['triple'] [immut.i64 'n'] {
    var.mut.arr.i64 (3) ['out'] = [[*0* *0* *0*]];
    loop.temp.range.i64 ['i'] = [*1*, *3*] {
        set ['out'['i']] = ['n' x 'i'];
    }
    give [share 'out'];
}
START {
    print.stdout[call 'triple'[*5*]];
}
"),
        "[5 10 15]",
    );
}

#[test]
fn an_array_of_arrays_is_two_allocations_and_shows_it() {
    // `arr.i64 (2 3)` and `arr.arr.i64 (2 3)` hold the same six numbers in a different
    // number of places, and only one of them can be taken apart. The printing says so.
    assert_eq!(
        said("\
START {
    var.immut.arr.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]];
    var.immut.arr.i64 (2 3) ['flat'] = [[*1* *2* *3* *4* *5* *6*]];
    print.stdout['m' str:* * 'flat'];
}
"),
        "[[1 2 3] [4 5 6]] [1 2 3 4 5 6]",
    );
}

#[test]
fn an_index_may_stop_where_an_allocation_ends() {
    // Which is the whole reason to write two `arr` links: the inner array is a thing,
    // and stopping there hands it to you.
    assert_eq!(
        said("\
START {
    var.mut.arr.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]];
    var.mut.arr.i64 (3) ['row'] = [share 'm'[*2*]];
    set ['row'[*1*]] = [*99*];
    print.stdout['m'[*1* *2*] str:* * 'row' str:* * 'm'];
}
"),
        "2 [99 5 6] [[1 2 3] [99 5 6]]",
        "and it is the array that lives there, not a copy of it",
    );
}

#[test]
fn nesting_goes_as_deep_as_it_is_written() {
    assert_eq!(
        said("\
START {
    var.mut.arr.arr.arr.i64 (2 2 2) ['deep'] = [[*1* *2* *3* *4* *5* *6* *7* *8*]];
    set ['deep'[*1* *1* *1*]] = [*99*];
    print.stdout['deep' str:* * 'deep'[*2* *1* *2*]];
}
"),
        "[[[99 2] [3 4]] [[5 6] [7 8]]] 6",
    );
}

#[test]
fn nested_arrays_compare_all_the_way_down() {
    assert_eq!(
        said("\
START {
    var.immut.arr.arr.i64 (2 2) ['a'] = [[*1* *2* *3* *4*]];
    var.immut.arr.arr.i64 (2 2) ['twin'] = [[*1* *2* *3* *4*]];
    var.immut.arr.arr.i64 (2 2) ['other'] = [[*1* *2* *3* *9*]];
    var.immut.bool ['same'] = ['a' == 'twin'];
    var.immut.bool ['not'] = ['a' == 'other'];
    print.stdout['same' str:* * 'not'];
}
"),
        "true false",
    );
}

#[test]
fn an_array_that_says_grow_can_be_made_longer() {
    assert_eq!(
        said("\
START {
    var.mut.arr.i64 (grow) ['xs'] = [[*1* *2* *3*]];
    add ['xs'] = [*4*];
    add ['xs'] = [*5*];
    print.stdout['xs' str:* * call count['xs']];
}
"),
        "[1 2 3 4 5] 5",
    );
}

#[test]
fn count_is_asked_when_the_shape_did_not_say() {
    // It folds to a constant on a fixed array and costs one call on a growing one,
    // which is the whole of what `grow` costs a reader.
    assert_eq!(
        said("\
START {
    var.mut.arr.i64 (grow) ['xs'] = [[]];
    loop.temp.range.i64 ['i'] = [*1*, *4*] {
        add ['xs'] = ['i' x 'i'];
    }
    var.mut.i64 ['sum'] = [*0*];
    loop.temp.range.i64 ['i'] = [*1*, call count['xs']] {
        set ['sum'] = ['sum' + 'xs'['i']];
    }
    print.stdout['xs' str:* * 'sum'];
}
"),
        "[1 4 9 16] 30",
    );
}

#[test]
fn rows_of_different_lengths() {
    // The thing fixed shapes cannot say at all.
    assert_eq!(
        said("\
START {
    var.mut.arr.arr.i64 (grow grow) ['jagged'] = [[]];
    var.mut.arr.i64 (grow) ['a'] = [[*1* *2*]];
    var.mut.arr.i64 (grow) ['b'] = [[*7* *8* *9* *10*]];
    add ['jagged'] = [share 'a'];
    add ['jagged'] = [share 'b'];
    add ['jagged'[*1*]] = [*3*];
    print.stdout['jagged' str:* * call count['jagged'] str:* * call count['jagged'[*2*]]];
}
"),
        "[[1 2 3] [7 8 9 10]] 2 4",
    );
}

#[test]
fn a_fixed_number_of_growing_rows_starts_as_that_many_empty_ones() {
    assert_eq!(
        said("\
START {
    var.mut.arr.arr.i64 (2 grow) ['two'] = [[]];
    add ['two'[*1*]] = [*7*];
    add ['two'[*2*]] = [*8*];
    add ['two'[*2*]] = [*9*];
    print.stdout['two'];
}
"),
        "[[7] [8 9]]",
    );
}

#[test]
fn a_constant_array_lives_in_the_module() {
    assert_eq!(
        said("\
const.export.arr.i64 (3) ['PRIMES'] = [[*2* *3* *5*]];
const.file.arr.arr.i64 (2 2) ['GRID'] = [[*1* *2* *3* *4*]];
START {
    print.stdout['PRIMES' str:* * 'GRID' str:* * 'GRID'[*2* *1*]];
}
"),
        "[2 3 5] [[1 2] [3 4]] 3",
    );
}

#[test]
fn there_is_one_of_a_constant_array() {
    // Which is what makes `share` mean something on one: it is written into the module
    // once, so every name for it is a name for the same thing.
    assert_eq!(
        said("\
const.file.arr.i64 (3) ['PRIMES'] = [[*2* *3* *5*]];
START {
    var.immut.arr.i64 (3) ['a'] = [share 'PRIMES'];
    var.mut.arr.i64 (3) ['mine'] = [copy 'PRIMES'];
    set ['mine'[*1*]] = [*99*];
    var.immut.bool ['same'] = ['a' == 'PRIMES'];
    print.stdout['same' str:* * 'mine' str:* * 'PRIMES'];
}
"),
        "true [99 3 5] [2 3 5]",
    );
}

#[test]
fn text_is_joined_by_writing_pieces_side_by_side() {
    // The same thing juxtaposition has always meant. What is new is that a piece may
    // not be known until the program runs.
    assert_eq!(
        said("\
fn.file.str ['greet'] [immut.str 'name'] {
    give [*Hello, * 'name' *!*];
}
START {
    var.immut.str ['name'] = [*Tankun*];
    var.immut.str ['hello'] = [*Hello, * 'name'];
    print.stdout['hello' str:*/* call 'greet'[*Claude*]];
}
"),
        "Hello, Tankun/Hello, Claude!",
    );
}

#[test]
fn built_text_is_the_same_as_written_text() {
    // Which is what makes comparing text a comparison of what it holds rather than of
    // which piece it is -- a built one is not in the module at all.
    assert_eq!(
        said("\
START {
    var.immut.str ['name'] = [*Tankun*];
    var.immut.str ['built'] = [*Hello, * 'name'];
    var.immut.bool ['same'] = ['built' == str:*Hello, Tankun*];
    var.immut.arr.str (2) ['words'] = [[*a* *b*]];
    var.immut.str ['joined'] = ['words'[*1*] 'words'[*2*] *!*];
    print.stdout['same' str:* * 'joined'];
}
"),
        "true ab!",
    );
}

#[test]
fn a_float_is_ieee_and_nothing_else() {
    // The famous one. `e` gets it right by never rounding; `b64` gets it wrong in the
    // way every conforming machine gets it wrong, which is the point of a standard.
    assert_eq!(
        said("\
START {
    var.immut.b64 ['a'] = [*1.5*];
    var.immut.b64 ['b'] = [*0.25*];
    var.immut.b64 ['sum'] = ['a' + 'b'];
    var.immut.b64 ['whole'] = [*3*];
    var.immut.b64 ['tenth'] = [*0.1*];
    var.immut.b64 ['fifth'] = [*0.2*];
    var.immut.b64 ['near'] = ['tenth' + 'fifth'];
    var.immut.bool ['ok'] = ['tenth' + 'fifth' == b64:*0.3*];
    print.stdout['sum' str:* * 'whole' str:* * 'near' str:* * 'ok'];
}
"),
        "1.75 3.0 0.30000000000000004 false",
    );
}

#[test]
fn infinity_and_not_a_number_are_answers_a_float_can_reach() {
    assert_eq!(
        said("\
START {
    var.immut.b64 ['one'] = [*1*];
    var.immut.b64 ['zero'] = [*0*];
    var.immut.b64 ['big'] = ['one' / 'zero'];
    var.immut.b64 ['none'] = ['zero' / 'zero'];
    var.immut.bool ['itself'] = ['none' == 'none'];
    print.stdout['big' str:* * 'none' str:* * 'itself'];
}
"),
        "infinity not-a-number false",
        "and a not-a-number is not even equal to itself, which is IEEE's rule",
    );
}

#[test]
fn no_number_says_whether_a_float_may_reach_one() {
    let source = "\
START {
    var.immut.b64 ['one'] = [*1*];
    var.immut.b64 ['zero'] = [*0*];
    var.immut.b64 ['big'] = ['one' / 'zero'];
    print.stdout['big'];
}
";
    let carries = quench_conf::Settings::default();
    assert_eq!(said_under(source, carries), "infinity");

    let stops = quench_conf::Settings {
        no_number: quench_conf::NoNumber::Stops,
        ..quench_conf::Settings::default()
    };
    let out = quench_lower::lower_under(source, stops);
    assert_eq!(
        quench_interp::run(&out.module.expect("a program")).expect("it runs"),
        quench_interp::Outcome::Trapped(quench_interp::Trap::NoNumber),
    );
}

#[test]
fn an_array_of_floats_shows_what_it_holds() {
    assert_eq!(
        said("START {\n    var.immut.arr.b64 (3) ['xs'] = [[*1.5* *2* *-0.25*]];\n    print.stdout['xs'];\n}\n"),
        "[1.5 2.0 -0.25]",
        "always with a point, so what is shown says which type it came from",
    );
}

/// What the heap kept, which is the one thing the oracle cannot see.
fn kept(source: &str) -> quench_interp::Kept {
    let out = lower(source);
    assert!(out.ok(), "{}", report(source));
    let (_, kept) = quench_interp::run_kept(&out.module.expect("a program")).expect("it runs");
    kept
}

#[test]
fn what_nothing_can_reach_goes_away() {
    // Twenty thousand arrays made and one held at a time. A heap that grew with the
    // loop would end with twenty thousand in it.
    let kept = kept("\
START {
    var.mut.i64 ['total'] = [*0*];
    loop.temp.range.i64 ['i'] = [*1*, *20000*] {
        var.immut.arr.i64 (3) ['scratch'] = [[*1* *2* *3*]];
        set ['total'] = ['total' + 'scratch'[*2*]];
    }
}
");
    assert!(kept.collections > 10, "it collected: {kept:?}");
    assert!(kept.live.0 < 1000, "and kept almost nothing: {kept:?}");
}

#[test]
fn text_and_exact_numbers_are_collected_too() {
    let kept = kept("\
START {
    loop.temp.range.i64 ['i'] = [*1*, *20000*] {
        var.immut.str ['junk'] = [*x* *y*];
        var.immut.e ['also'] = [e:*1/3* + e:*1/6*];
    }
}
");
    assert!(kept.live.1 < 1000, "text went away: {kept:?}");
    assert!(kept.live.2 < 1000, "and so did exact numbers: {kept:?}");
}

#[test]
fn what_something_can_reach_stays() {
    // An array of arrays is the one thing in Quench with edges to follow, and this is
    // the program that fails if tracing does not follow them: the rows are reachable
    // only through the outer array, and nothing else names them by the end.
    assert_eq!(
        said("\
START {
    var.mut.arr.arr.i64 (grow grow) ['kept'] = [[]];
    loop.temp.range.i64 ['r'] = [*1*, *3*] {
        var.mut.arr.i64 (grow) ['row'] = [[]];
        loop.temp.range.i64 ['c'] = [*1*, *3*] {
            add ['row'] = ['r' x 'c'];
        }
        add ['kept'] = [share 'row'];
    }
    loop.temp.range.i64 ['i'] = [*1*, *20000*] {
        var.immut.arr.i64 (2) ['junk'] = [[*1* *2*]];
    }
    print.stdout['kept'];
}
"),
        "[[1 2 3] [2 4 6] [3 6 9]]",
    );
}

#[test]
fn an_array_written_empty_still_says_what_it_will_hold() {
    // Its header cannot come from its elements, because it has none -- and an array
    // written empty is exactly what a growing one starts as. This program freed its
    // rows underneath itself until the header came from the type instead.
    assert_eq!(
        said("\
START {
    var.mut.arr.arr.i64 (grow grow) ['rows'] = [[]];
    var.mut.arr.i64 (grow) ['one'] = [[*7* *8*]];
    add ['rows'] = [share 'one'];
    loop.temp.range.i64 ['i'] = [*1*, *20000*] {
        var.immut.str ['junk'] = [*a* *b*];
    }
    print.stdout['rows'];
}
"),
        "[[7 8]]",
    );
}

#[test]
fn what_a_program_was_written_with_outlives_every_collection() {
    // A constant array and a written piece of text are in the artefact rather than on
    // the heap, so nothing can ever be the last to let go of one.
    assert_eq!(
        said("\
const.file.arr.i64 (3) ['PRIMES'] = [[*2* *3* *5*]];
START {
    loop.temp.range.i64 ['i'] = [*1*, *20000*] {
        var.immut.arr.i64 (2) ['junk'] = [[*1* *2*]];
    }
    print.stdout['PRIMES' str:* * str:*written*];
}
"),
        "[2 3 5] written",
    );
}

#[test]
fn the_dev_jit_collects_too() {
    // The engine that has no list of its own to walk. Every reference-typed value in a
    // function gets a slot in a frame the runtime owns, written where the value is
    // made -- so wherever compiled code is when a collection happens, it has already
    // said what it is holding.
    let source = "\
START {
    var.mut.i64 ['total'] = [*0*];
    loop.temp.range.i64 ['i'] = [*1*, *20000*] {
        var.immut.arr.i64 (3) ['scratch'] = [[*1* *2* *3*]];
        var.immut.str ['junk'] = [*x* *y*];
        set ['total'] = ['total' + 'scratch'[*2*]];
    }
    print.stdout['total'];
}
";
    let out = lower(source);
    assert!(out.ok(), "{}", report(source));
    let module = out.module.expect("a program");

    let compiled = quench_dev::compile(&module).expect("it compiles");
    let (_, printed) = compiled.run_capturing();
    assert_eq!(printed.out, "40000", "and it still answers");

    let (arrays, texts, _, collections) = compiled.kept();
    assert!(collections > 10, "it collected: {collections}");
    assert!(arrays < 1000, "and kept almost nothing: {arrays} arrays");
    assert!(texts < 1000, "text included: {texts} texts");
}

#[test]
fn both_engines_keep_what_the_other_keeps() {
    // Not something a program can see, and that is the point -- but the two arriving at
    // nearly the same heap is what says the roots are the same roots. The Dev JIT holds
    // a little more, because a slot it wrote is not cleared when the value dies: it
    // keeps a thing alive slightly longer, which is the direction to be wrong in.
    let source = "\
START {
    var.mut.arr.arr.i64 (grow grow) ['kept'] = [[]];
    var.mut.arr.i64 (grow) ['row'] = [[*1* *2*]];
    add ['kept'] = [share 'row'];
    loop.temp.range.i64 ['i'] = [*1*, *20000*] {
        var.immut.arr.i64 (2) ['junk'] = [[*1* *2*]];
    }
    print.stdout['kept'];
}
";
    let out = lower(source);
    assert!(out.ok(), "{}", report(source));
    let module = out.module.expect("a program");

    let (_, kept) = quench_interp::run_kept(&module).expect("it runs");
    let compiled = quench_dev::compile(&module).expect("it compiles");
    let (_, printed) = compiled.run_capturing();
    let (arrays, _, _, _) = compiled.kept();

    assert_eq!(printed.out, "[[1 2]]", "the rows survived in the compiled engine too");
    let difference = arrays.abs_diff(kept.live.0);
    assert!(difference < 50, "interpreter {} against dev jit {arrays}", kept.live.0);
}

#[test]
fn all_three_binary_widths_round_the_way_they_should() {
    // The same source under three types, and the differences are the formats' own.
    assert_eq!(
        said("\
START {
    var.immut.b64 ['d'] = [*0.1*];
    var.immut.b32 ['s'] = [*0.1*];
    var.immut.b16 ['h'] = [*0.1*];
    print.stdout['d' str:* * 's' str:* * 'h'];
}
"),
        "0.1 0.1 0.099975586",
        "binary16 cannot hold a tenth and says which one it holds instead",
    );
    assert_eq!(
        said("\
START {
    var.immut.b64 ['d'] = [*0.1* + *0.2*];
    var.immut.b32 ['s'] = [*0.1* + *0.2*];
    var.immut.b16 ['h'] = [*0.1* + *0.2*];
    print.stdout['d' str:* * 's' str:* * 'h'];
}
"),
        "0.30000000000000004 0.3 0.2998047",
    );
}

#[test]
fn a_b16_reaches_its_own_edges() {
    assert_eq!(
        said("\
START {
    var.immut.b16 ['big'] = [*65504*];
    var.immut.b16 ['over'] = [*65504* + *65504*];
    var.immut.b16 ['tiny'] = [*0.00000006*];
    print.stdout['big' str:* * 'over' str:* * 'tiny'];
}
"),
        "65504.0 infinity 0.000000059604645",
        "the largest binary16, what is past it, and the smallest subnormal",
    );
}

#[test]
fn every_whole_number_type_wraps_at_its_own_edge() {
    // All of them ride in an `i64`, so what makes a `u8` a `u8` is being put back
    // inside it after every operation.
    assert_eq!(
        said("\
START {
    var.immut.u8 ['a'] = [*200*];
    var.immut.u8 ['b'] = [*100*];
    var.immut.i8 ['c'] = [*120*];
    var.immut.i16 ['low'] = [*-32768*];
    var.immut.u8 ['over'] = ['a' + 'b'];
    var.immut.i8 ['twice'] = ['c' + 'c'];
    var.immut.i16 ['under'] = ['low' - *1*];
    print.stdout['over' str:* * 'twice' str:* * 'under'];
}
"),
        "44 -16 32767",
    );
}

#[test]
fn a_u64_is_read_as_unsigned_wherever_that_shows() {
    // Past `i64::MAX` a `u64` is a negative number in a slot and is not a negative
    // number. Printing, comparing and dividing are the three places that notice.
    assert_eq!(
        said("\
START {
    var.immut.u64 ['big'] = [*18446744073709551615*];
    var.immut.u64 ['one'] = [*1*];
    var.immut.bool ['gt'] = ['big' > 'one'];
    var.immut.u64 ['half'] = ['big' / *2*];
    print.stdout['big' str:* * 'gt' str:* * 'half'];
}
"),
        "18446744073709551615 true 9223372036854775807",
    );
}

#[test]
fn a_written_number_is_read_by_the_width_asking_for_it() {
    assert_eq!(
        said("START {\n    var.immut.u32 ['q'] = [*4000000000*];\n    var.immut.u32 ['h'] = ['q' / *2*];\n    print.stdout['h'];\n}\n"),
        "2000000000",
        "and an unsigned division has neither edge a signed one has",
    );
}

#[test]
fn a_decimal_rounds_in_the_base_it_was_written_in() {
    // The same sum a `b64` is famous for getting wrong, got right -- not by being more
    // accurate, but by rounding where a person writing `0.1` expects it to.
    assert_eq!(
        said("\
START {
    var.immut.d64 ['sum'] = [*0.1* + *0.2*];
    var.immut.bool ['right'] = [d64:*0.1* + d64:*0.2* == d64:*0.3*];
    var.immut.b64 ['binary'] = [*0.1* + *0.2*];
    print.stdout['sum' str:* * 'right' str:* * 'binary'];
}
"),
        "0.3 true 0.30000000000000004",
    );
}

#[test]
fn a_decimal_keeps_the_cohort_it_was_given() {
    // `2.50` and `2.5` are the same number and not the same *written* number: a
    // trailing zero in decimal says something about how far the precision goes, so
    // arithmetic carries it and a comparison ignores it.
    assert_eq!(
        said("\
START {
    var.immut.d64 ['sum'] = [*2.50* + *1.00*];
    var.immut.d64 ['product'] = [*1.005* x *100*];
    var.immut.bool ['equal'] = [d64:*2.50* == d64:*2.5*];
    print.stdout['sum' str:* * 'product' str:* * 'equal'];
}
"),
        "3.50 100.500 true",
    );
}

#[test]
fn a_d32_keeps_seven_digits_and_a_d64_sixteen() {
    // Which is the whole difference between the two, and the reason the digit count
    // rides along with every operation rather than living in the type.
    assert_eq!(
        said("\
START {
    var.immut.d32 ['narrow'] = [*1* / *3*];
    var.immut.d64 ['wide'] = [*1* / *3*];
    var.immut.d32 ['rounded'] = [*12345678*];
    print.stdout['narrow' str:* * 'wide' str:* * 'rounded'];
}
"),
        "0.3333333 0.3333333333333333 1.234568E+7",
    );
}

#[test]
fn an_exact_decimal_division_does_not_grow_a_tail() {
    // A division is worked out with a digit to spare and then walked back to the
    // exponent it would have had if nothing needed sparing. Without that, dividing by
    // one would lengthen a number every time.
    assert_eq!(
        said("\
START {
    var.immut.d32 ['same'] = [*2.5* / *1*];
    var.immut.d64 ['half'] = [*1* / *2*];
    print.stdout['same' str:* * 'half'];
}
"),
        "2.5 0.5",
    );
}

#[test]
fn a_decimal_answers_where_an_exact_number_stops() {
    // Dividing by nought is the difference between a float and a ratio: one has a value
    // to give back and the other has nothing to say.
    assert_eq!(
        said("\
START {
    var.immut.d64 ['zero'] = [*0*];
    var.immut.d64 ['big'] = [*1* / 'zero'];
    var.immut.d64 ['neither'] = ['zero' / 'zero'];
    print.stdout['big' str:* * 'neither'];
}
"),
        "infinity not-a-number",
    );
}

#[test]
fn a_not_a_number_is_none_of_less_equal_or_greater() {
    // Four answers, not three -- which is why `<==` and `>==` are not one comparison
    // against one number the way `<` and `==` are.
    assert_eq!(
        said("\
START {
    var.immut.d64 ['zero'] = [*0*];
    var.immut.d64 ['none'] = ['zero' / 'zero'];
    var.immut.d64 ['one'] = [*1*];
    var.immut.bool ['under'] = ['none' <== 'one'];
    var.immut.bool ['over'] = ['none' >== 'one'];
    var.immut.bool ['same'] = ['none' == 'none'];
    var.immut.bool ['differs'] = ['none' !== 'none'];
    print.stdout['under' str:* * 'over' str:* * 'same' str:* * 'differs'];
}
"),
        "false false false true",
    );
}

#[test]
fn decimals_are_collected_too() {
    // Every answer is a fresh handle, so a loop that works out two hundred of them and
    // keeps one has to be able to free the rest without freeing that one.
    assert_eq!(
        said("\
START {
    var.mut.arr.d64 (grow) ['kept'] = [[]];
    loop.temp.range.i64 ['i'] = [*1*, *200*] {
        var.immut.d64 ['made'] = [d64:*1* / d64:*8*];
        add ['kept'] = ['made'];
    }
    var.immut.d64 ['last'] = ['kept'[*200*]];
    print.stdout['last'];
}
"),
        "0.125",
    );
}

#[test]
fn an_array_of_decimals_compares_by_value() {
    // Two names for one number are not the only way to hold the same one, so these
    // compare by what they are -- and `0.10` is `0.1`, however it was written.
    assert_eq!(
        said("\
START {
    var.immut.arr.d64 (3) ['xs'] = [*0.1* *0.2* *0.3*];
    var.immut.arr.d64 (3) ['ys'] = [*0.10* *0.20* *0.30*];
    var.immut.bool ['same'] = ['xs' == 'ys'];
    print.stdout['xs' str:* * 'same'];
}
"),
        "[0.1 0.2 0.3] true",
    );
}

#[test]
fn a_decimal_refuses_what_a_binary_float_refuses() {
    // `^` and `mod` for the same reason: no standard says how a `pow` rounds, and a
    // remainder is a question for the types that do not round at all.
    let power = report("START { var.immut.d64 ['x'] = [*2* ^ *3*]; print.stdout['x']; }");
    assert!(power.contains("`^` on a `d64` is not built yet"), "{power}");
    // `mod` says the true reason rather than "not built": a float division answers with
    // the nearest float and leaves nothing behind to ask about.
    let left = report("START { var.immut.d64 ['x'] = [*7* mod *3*]; print.stdout['x']; }");
    assert!(left.contains("a `d64` division leaves nothing"), "{left}");

    // And a ratio, which is written the way an `e` is written and is not a decimal.
    let rendered = report("START { var.immut.d64 ['x'] = [*1/3*]; print.stdout['x']; }");
    assert!(rendered.contains("is not a `d64`"), "{rendered}");
}

#[test]
fn a_function_may_take_and_give_back_a_float() {
    // A parameter and a return type are the two places a type appears that a body never
    // reaches, and until this test nothing exercised a float in either. The Dev JIT
    // built the stand-in return of its stopping block with an integer constant, which
    // Cranelift's verifier refuses outright for a float -- so every function giving one
    // back crashed the compiler, and the oracle never wrote one to notice.
    assert_eq!(
        said("\
fn.file.b64 ['halved'] [immut.b64 'x'] { give ['x' x *0.5*]; }
fn.file.b32 ['narrow'] [immut.b32 'x'] { give ['x' + *1.0*]; }
fn.file.b16 ['half of'] [immut.b16 'x'] { give ['x' x *0.5*]; }
fn.file.d64 ['tenth of'] [immut.d64 'x'] { give ['x' / *10*]; }
START {
    var.immut.b64 ['a'] = [call 'halved'[*3.0*]];
    var.immut.b32 ['b'] = [call 'narrow'[*1.5*]];
    var.immut.b16 ['c'] = [call 'half of'[*5.0*]];
    var.immut.d64 ['d'] = [call 'tenth of'[*1*]];
    print.stdout['a' str:* * 'b' str:* * 'c' str:* * 'd'];
}
"),
        "1.5 2.5 2.5 0.1",
    );
}

#[test]
fn an_array_of_floats_crosses_into_a_function() {
    assert_eq!(
        said("\
fn.file.b64 ['sum of'] [immut.arr.b64 (2) 'xs'] {
    var.mut.b64 ['sum'] = [*0.0*];
    loop.temp.range.i64 ['i'] = [*1*, call count['xs']] {
        set ['sum'] = ['sum' + 'xs'['i']];
    }
    give ['sum'];
}
START {
    var.immut.arr.b64 (2) ['xs'] = [*1.5* *2.5*];
    print.stdout[call 'sum of'[share 'xs']];
}
"),
        "4.0",
    );
}

#[test]
fn stitch_is_how_a_number_becomes_text() {
    // The one conversion in the language. A program could always *show* a number and
    // could not hold the text of one, so no message, no log line, no file.
    assert_eq!(
        said("\
START {
    var.immut.i64 ['n'] = [*42*];
    var.immut.str ['line'] = [call stitch[*n is * 'n' *!*]];
    print.stdout['line'];
}
"),
        "n is 42!",
    );
}

#[test]
fn stitch_writes_every_type_the_way_print_does() {
    // Each `Say` is the same expression as the `Print` beside it, so the two cannot
    // drift: a `d64` keeps its cohort, an `e` wears a denominator, a `u64` reads as
    // unsigned, and an array wears its brackets.
    assert_eq!(
        said("\
START {
    var.immut.b64 ['x'] = [*1.5*];
    var.immut.e ['third'] = [*1* / *3*];
    var.immut.d64 ['due'] = [*7.00*];
    var.immut.bool ['yes'] = [*true*];
    var.immut.arr.i64 (3) ['xs'] = [*1* *2* *3*];
    var.immut.u64 ['big'] = [*18446744073709551615*];
    print.stdout[call stitch['x' str:* * 'third' str:* * 'due' str:* * 'yes' str:* * 'xs' str:* * 'big']];
}
"),
        "1.5 1/3 7.00 true [1 2 3] 18446744073709551615",
    );
}

#[test]
fn what_stitch_builds_is_what_print_would_have_written() {
    // The claim that makes one implementation of each rather than two: the text a
    // program keeps and the text it shows are the same characters.
    let both = said("\
START {
    var.immut.b64 ['x'] = [*0.1* + *0.2*];
    print.stdout['x' \\n];
    print.stdout[call stitch['x'] \\n];
}
");
    let lines: Vec<&str> = both.lines().collect();
    assert_eq!(lines.len(), 2, "{both}");
    assert_eq!(lines[0], lines[1], "printed and stitched must agree");
}

#[test]
fn a_stitched_string_is_a_string_like_any_other() {
    assert_eq!(
        said("\
fn.file.str ['line for'] [immut.i64 'n', immut.d64 'price'] {
    give [call stitch[*item * 'n' *: * 'price']];
}
START {
    var.immut.str ['a'] = [call 'line for'[*7*, *2.50*]];
    var.immut.str ['b'] = [call 'line for'[*7*, *2.50*]];
    var.immut.bool ['same'] = ['a' == 'b'];
    var.immut.str ['joined'] = ['a' *, again*];
    print.stdout['same' str:* * 'joined'];
}
"),
        "true item 7: 2.50, again",
    );
}

#[test]
fn stitch_takes_one_list_and_no_arithmetic() {
    for (source, expected) in [
        (
            "START { var.immut.str ['x'] = [call stitch[*a*, *b*]]; print.stdout['x']; }",
            "takes one list",
        ),
        (
            "START { var.immut.str ['x'] = [call stitch[*1* + *2*]]; print.stdout['x']; }",
            "joins its pieces rather than working them out",
        ),
    ] {
        let rendered = report(source);
        assert!(rendered.contains(expected), "{source}\n{rendered}");
        assert!(rendered.contains("Error code: E0493"), "{source}\n{rendered}");
    }

    // And juxtaposing text with a number is still refused without it, which is the
    // rule `stitch` is the exception to.
    let bare = report("START { var.immut.i64 ['n'] = [*1*]; var.immut.str ['x'] = [*n is * 'n']; }");
    assert!(bare.contains("text is made of text"), "{bare}");
}

#[test]
fn what_counts_as_one_character_is_a_setting() {
    // The only setting about text rather than numbers, and the one place the answer is
    // visible: `é` written as two scalars, and an emoji welded out of seven.
    let source = "\
START {
    var.immut.str ['plain'] = [*café*];
    var.immut.str ['acute'] = [*e\u{0301}*];
    var.immut.str ['family'] = [*\u{1F9D1}\u{200D}\u{1F9D1}\u{200D}\u{1F9D2}\u{200D}\u{1F9D2}*];
    print.stdout[call stitch[call count['plain'] str:* * call count['acute'] str:* * call count['family']]];
}
";
    let under = |characters| {
        said_under(source, quench_conf::Settings { characters, ..Default::default() })
    };
    assert_eq!(under(quench_conf::Characters::Clusters), "4 1 1");
    assert_eq!(under(quench_conf::Characters::Letters), "4 2 7");
}

#[test]
fn count_takes_an_array_or_a_piece_of_text_and_nothing_else() {
    let rendered = report("START { var.immut.i64 ['n'] = [*1*]; var.immut.i64 ['c'] = [call count['n']]; }");
    assert!(rendered.contains("`count` was given an `i64`"), "{rendered}");
    assert!(rendered.contains("an array and a piece of text"), "{rendered}");
}

#[test]
fn the_maths_ieee_requires_is_the_maths_there_is() {
    assert_eq!(
        said("\
START {
    var.immut.b64 ['two'] = [*2.0*];
    var.immut.b64 ['half'] = [*0.5*];
    var.immut.b64 ['neg'] = [*-2.5*];
    print.stdout[call stitch[
        call sqrt['two'] str:* *
        call abs['neg'] str:* *
        call floor['neg'] str:* *
        call ceil['neg'] str:* *
        call trunc['neg'] str:* *
        call min['two', 'half'] str:* *
        call max['two', 'half'] str:* *
        call copysign['two', 'neg'] str:* *
        call fma['two', 'half', 'half']
    ]];
}
"),
        "1.4142135623730951 2.5 -3.0 -2.0 -2.0 0.5 2.0 -2.0 1.5",
    );
}

#[test]
fn remainder_takes_the_nearest_quotient_and_mod_takes_the_near_one() {
    // The difference from `%`, and from Quench's `mod`: this one takes the quotient to
    // the *nearest* integer, so what is left is never more than half the divisor and can
    // come out with the opposite sign to what went in.
    assert_eq!(
        said("START {
    var.immut.b64 ['five'] = [*5.0*];
    var.immut.b64 ['seven'] = [*7.0*];
    var.immut.b64 ['two'] = [*2.0*];
    print.stdout[call stitch[
        call remainder['five', 'two'] str:* *
        call remainder['seven', 'two'] str:* *
        call remainder['five', 'two']
    ]];
}
"),
        "1.0 -1.0 1.0",
        "seven over two is four, not three, so one is owed rather than left",
    );
}

#[test]
fn round_breaks_a_tie_to_the_even_one() {
    // IEEE says `roundToIntegralTiesToEven`. Rust's own `f64::round` breaks ties away
    // from zero, so writing this in terms of it would have been quietly wrong in both
    // engines at once — which is the one kind of wrong the oracle cannot see.
    assert_eq!(
        said("\
START {
    var.immut.b64 ['a'] = [*2.5*];
    var.immut.b64 ['b'] = [*3.5*];
    var.immut.b64 ['c'] = [*-2.5*];
    print.stdout[call stitch[call round['a'] str:* * call round['b'] str:* * call round['c']]];
}
"),
        "2.0 4.0 -2.0",
        "two and four, not three and four",
    );
}

#[test]
fn the_maths_works_on_every_width_and_keeps_it() {
    assert_eq!(
        said("\
START {
    var.immut.b32 ['a'] = [*2.0*];
    var.immut.b16 ['b'] = [*2.0*];
    print.stdout[call stitch[call sqrt['a'] str:* * call sqrt['b']]];
}
"),
        "1.4142135 1.4140625",
        "a `b32`'s answer and a `b16`'s, each rounded to its own width",
    );
}

#[test]
fn the_maths_takes_floats_of_one_width_and_says_so() {
    for (source, expected) in [
        (
            "START { var.immut.i64 ['n'] = [*2*]; var.immut.b64 ['x'] = [call sqrt['n']]; print.stdout['x']; }",
            "works on binary floats",
        ),
        (
            "START { var.immut.b64 ['a'] = [*1.0*]; var.immut.b32 ['b'] = [*1.0*]; var.immut.b64 ['x'] = [call min['a', 'b']]; print.stdout['x']; }",
            "takes one width, and was given two",
        ),
        (
            "START { var.immut.b64 ['a'] = [*1.0*]; var.immut.b64 ['x'] = [call sqrt['a', 'a']]; print.stdout['x']; }",
            "`sqrt` takes one number",
        ),
    ] {
        let rendered = report(source);
        assert!(rendered.contains(expected), "{source}\n{rendered}");
        assert!(rendered.contains("Error code: E0494"), "{source}\n{rendered}");
    }

    // And a bare word that is none of them lists the ones that are. This has been
    // `sin` and then `sinh`, both of which were built out from under it within the
    // hour, so it is now something with no plans: the error function.
    let unknown = report("START { var.immut.b64 ['x'] = [call erf[*1.0*]]; print.stdout['x']; }");
    assert!(unknown.contains("there is nothing called `erf`"), "{unknown}");
    assert!(unknown.contains("`sqrt`"), "{unknown}");
}

#[test]
fn the_maths_ieee_only_recommends_is_worked_out_rather_than_asked_for() {
    // Every library gets these a little bit wrong in its own way, so three engines
    // calling three libraries would be three answers. Quench works them out instead, at
    // whatever width the particular answer needs, and rounds once.
    assert_eq!(
        said("\
START {
    var.immut.b64 ['one'] = [*1.0*];
    var.immut.b64 ['ten'] = [*10.0*];
    var.immut.b64 ['two'] = [*2.0*];
    print.stdout[call stitch[
        call exp['one'] str:* *
        call ln['ten'] str:* *
        ('two' ^ 'ten') str:* *
        ('two' ^ b64:*0.5*)
    ]];
}
"),
        "2.718281828459045 2.302585092994046 1024.0 1.4142135623730951",
    );
}

#[test]
fn the_recommended_maths_is_a_b64_and_says_why() {
    let rendered = report("START { var.immut.b32 ['x'] = [*2.0*]; var.immut.b32 ['y'] = [call exp['x']]; print.stdout['y']; }");
    assert!(rendered.contains("`exp` works on a `b64`"), "{rendered}");
    assert!(rendered.contains("would round twice"), "{rendered}");
    assert!(rendered.contains("Error code: E0495"), "{rendered}");

    // And on something that is not a float at all, the reason is the other one.
    let whole = report("START { var.immut.i64 ['n'] = [*2*]; var.immut.b64 ['y'] = [call ln['n']]; print.stdout['y']; }");
    assert!(whole.contains("`ln` works on a `b64`"), "{whole}");
    assert!(whole.contains("because the standard settles those"), "{whole}");
}

#[test]
fn the_trig_reduces_an_argument_no_library_would_dare_to() {
    // Working out which quarter turn `1e300` lands in needs π to a thousand bits, which
    // is why a C library gets vague out here and this one does not: π is a `Big` and it
    // is asked for as many bits as the argument has exponent.
    assert_eq!(
        said("\
START {
    var.immut.b64 ['one'] = [*1.0*];
    var.immut.b64 ['huge'] = [*1e300*];
    print.stdout[call stitch[
        call sin['one'] str:* *
        call cos['one'] str:* *
        call tan['one'] str:* *
        call atan['one'] str:* *
        call atan2['one', 'one'] str:* *
        call sin['huge']
    ]];
}
"),
        "0.8414709848078965 0.5403023058681398 1.5574077246549023 \
0.7853981633974483 0.7853981633974483 -0.8178819121159085",
    );
}

#[test]
fn the_inverse_and_hyperbolic_functions_are_here_too() {
    assert_eq!(
        said("\
START {
    var.immut.b64 ['half'] = [*0.5*];
    var.immut.b64 ['one'] = [*1.0*];
    var.immut.b64 ['two'] = [*2.0*];
    print.stdout[call stitch[
        call asin['half'] str:* * call acos['half'] str:* *
        call sinh['one'] str:* * call tanh['one'] str:* *
        call acosh['two'] str:* * call atanh['half'] str:* *
        call cbrt['two'] str:* * call hypot['one', 'two']
    ]];
}
"),
        "0.5235987755982989 1.0471975511965979 1.1752011936438014 0.7615941559557649 \
1.3169578969248168 0.5493061443340549 1.2599210498948732 2.23606797749979",
    );
}

#[test]
fn what_min_and_max_do_with_a_not_a_number_is_a_setting() {
    // Both are somebody's idea of right — C's `fmin` skips, Java's `Math.min` spreads —
    // so it is a key rather than a decision.
    let source = "\
START {
    var.immut.b64 ['zero'] = [*0.0*];
    var.immut.b64 ['nan'] = ['zero' / 'zero'];
    var.immut.b64 ['five'] = [*5.0*];
    print.stdout[call stitch[call min['nan', 'five'] str:* * call max['nan', 'five']]];
}
";
    let under = |min_max| {
        said_under(source, quench_conf::Settings { min_max, ..Default::default() })
    };
    assert_eq!(under(quench_conf::MinMax::Skips), "5.0 5.0");
    assert_eq!(under(quench_conf::MinMax::Spreads), "not-a-number not-a-number");

    // And with no not-a-number in sight the two agree, which is most programs.
    let ordinary = "\
START {
    var.immut.b64 ['a'] = [*2.0*];
    var.immut.b64 ['b'] = [*5.0*];
    print.stdout[call stitch[call min['a', 'b'] str:* * call max['a', 'b']]];
}
";
    for setting in [quench_conf::MinMax::Skips, quench_conf::MinMax::Spreads] {
        assert_eq!(
            said_under(ordinary, quench_conf::Settings { min_max: setting, ..Default::default() }),
            "2.0 5.0"
        );
    }
}

#[test]
fn a_power_is_the_operator_and_there_is_no_second_name_for_it() {
    // `^` had been refused on a float with "not built yet" while the same answer was
    // reachable as `call pow[…]`, which is two spellings of one operator — the thing
    // one-spelling-per-operator exists to stop. The operator is the spelling.
    assert_eq!(
        said("\
START {
    var.immut.b64 ['two'] = [*2.0*];
    var.immut.b64 ['ten'] = [*10.0*];
    var.immut.i64 ['n'] = [*2* ^ *10*];
    var.immut.e ['third'] = [*2* ^ *3*];
    print.stdout[call stitch[
        ('two' ^ 'ten') str:* * ('two' ^ b64:*0.5*) str:* * 'n' str:* * 'third'
    ]];
}
"),
        "1024.0 1.4142135623730951 1024 8",
        "one operator, four types, and the same meaning in each",
    );

    // And the second name is gone.
    let gone = report("START { var.immut.b64 ['a'] = [*2.0*]; var.immut.b64 ['x'] = [call pow['a', 'a']]; print.stdout['x']; }");
    assert!(gone.contains("there is nothing called `pow`"), "{gone}");

    // The narrow floats say why, and it is the double rounding rather than "not built".
    let narrow = report("START { var.immut.b32 ['a'] = [*2.0*]; var.immut.b32 ['x'] = ['a' ^ 'a']; print.stdout['x']; }");
    assert!(narrow.contains("`^` on a `b32` is not built yet"), "{narrow}");
    assert!(narrow.contains("would round twice"), "{narrow}");

    // And `mod` on a float now says the true reason rather than "no standard settles it".
    let left = report("START { var.immut.b64 ['a'] = [*2.0*]; var.immut.b64 ['x'] = ['a' mod 'a']; print.stdout['x']; }");
    assert!(left.contains("a `b64` division leaves nothing"), "{left}");
    assert!(left.contains("call remainder"), "{left}");
}
