//! What a program means, and what is said when it does not mean anything.

use quench_check::check;
use quench_diag::SourceFile;

fn errors(source: &str) -> String {
    let out = check(source);
    quench_diag::report(&SourceFile::new("src/main.qnl", source), &out.errors)
}

fn codes(source: &str) -> Vec<String> {
    check(source).errors.iter().map(|e| e.code.clone()).collect()
}

#[test]
fn a_declaration_is_understood() {
    let out = check("START { var.mut.i64 ['count'] = [*7*]; }");
    assert!(out.ok(), "{}", errors("START { var.mut.i64 ['count'] = [*7*]; }"));
    assert_eq!(out.locals().len(), 1);
    assert_eq!(out.locals()[0].name, "count");
    assert_eq!(out.locals()[0].ty, quench_check::Ty::I64);
    assert!(out.locals()[0].mutable);
}

#[test]
fn declaring_a_name_twice_is_the_error_this_project_was_pitched_on() {
    let source = "\
START {
    var.immut.str ['name'] = [*Tankun*];
    var.immut.i64 ['name'] = [*1000*];
}
";
    let rendered = errors(source);
    assert!(rendered.contains("`'name'` is declared twice."), "{rendered}");
    assert!(rendered.contains("declared here first, as `str`"), "{rendered}");
    assert!(rendered.contains("and declared again here, as `i64`"), "{rendered}");
    assert!(rendered.contains("Error code: E0201"), "{rendered}");
    // The name is what collided, so the carets are under the name and not the chain.
    assert!(rendered.contains("~~~~~~ declared here first"), "{rendered}");
}

#[test]
fn a_type_that_is_not_built_does_not_hide_a_name_declared_twice() {
    // Two separate mistakes on one line, and both get said. Reporting only the type
    // would leave the reader to discover the collision on their own after fixing it.
    let source = "START { var.immut.str ['x'] = [*a*]; var.immut.b16 ['x'] = [*1*]; }";
    let found = codes(source);
    assert!(found.contains(&"E0405".to_string()), "{}", errors(source));
    assert!(found.contains(&"E0201".to_string()), "{}", errors(source));
}

#[test]
fn errors_come_out_in_the_order_they_appear_in_the_file() {
    // Otherwise a reader jumps around the file to follow their own mistakes.
    let source = "\
START {
    var.immut.b17 ['a'] = [*1*];
    print.stdout['nope'];
    var.immut.i64 ['b'] = [*hello*];
}
";
    let out = check(source);
    let places: Vec<usize> =
        out.errors.iter().filter_map(|e| e.primary_label()).map(|l| l.span.start).collect();
    let mut sorted = places.clone();
    sorted.sort_unstable();
    assert_eq!(places, sorted, "{}", errors(source));
}

#[test]
fn a_name_that_is_nearly_right_gets_told_which_one() {
    let source = "START { var.immut.str ['greeting'] = [*Hello*]; print.stdout['greetng' \\n]; }";
    let rendered = errors(source);
    assert!(rendered.contains("`'greetng'` is not declared."), "{rendered}");
    assert!(rendered.contains("did you mean `'greeting'`?"), "{rendered}");
}

#[test]
fn a_name_that_is_nothing_like_anything_is_not_guessed_at() {
    // A suggestion that is not the answer costs the reader a second look, so nothing is
    // offered unless it is within one edit.
    let source = "START { var.immut.str ['greeting'] = [*Hello*]; print.stdout['wobble' \\n]; }";
    let rendered = errors(source);
    assert!(rendered.contains("is not declared"), "{rendered}");
    assert!(!rendered.contains("did you mean"), "{rendered}");
    assert!(rendered.contains("declare it above, with `var`"), "{rendered}");
}

#[test]
fn a_type_that_is_meant_to_exist_and_a_type_that_is_not_are_different_errors() {
    // `b16` is a type Quench means to have. `b17` is a typo. A reader deserves to know
    // which of those happened.
    let not_built = errors("START { var.immut.b16 ['x'] = [*1*]; }");
    assert!(not_built.contains("`b16` is not built yet"), "{not_built}");
    assert!(not_built.contains("Error code: E0405"), "{not_built}");

    let nonsense = errors("START { var.immut.b17 ['x'] = [*1*]; }");
    assert!(nonsense.contains("`b17` is not a type"), "{nonsense}");
    assert!(nonsense.contains("Error code: E0402"), "{nonsense}");
}

#[test]
fn nothing_converts_on_its_own() {
    let source = "START { var.immut.i64 ['n'] = [*1*]; var.immut.str ['s'] = ['n']; }";
    let rendered = errors(source);
    assert!(rendered.contains("this is an `i64`, and it is being given to a `str`"), "{rendered}");
    assert!(rendered.contains("two types meet only where something says they should"), "{rendered}");
}

#[test]
fn a_written_value_is_read_by_the_type_it_is_given_to() {
    let rendered = errors("START { var.immut.i64 ['n'] = [*hello*]; }");
    assert!(rendered.contains("`hello` is not a whole number"), "{rendered}");
    assert!(rendered.contains("-9223372036854775808"), "{rendered}");

    // And the same characters are perfectly good text.
    assert!(check("START { var.immut.str ['s'] = [*hello*]; }").ok());
    // While these are a fine number and fine text both.
    assert!(check("START { var.immut.i64 ['n'] = [*1000*]; }").ok());
    assert!(check("START { var.immut.str ['s'] = [*1000*]; }").ok());
}

#[test]
fn juxtaposition_builds_text_and_says_so_when_it_cannot() {
    let rendered = errors("START { var.immut.i64 ['n'] = [*1* \\n *2*]; }");
    assert!(rendered.contains("a number is one written value, not several"), "{rendered}");
    assert!(rendered.contains("`str` is the type where a value is a list of pieces"), "{rendered}");
}

#[test]
fn mut_goes_before_the_type() {
    let rendered = errors("START { var.immut.i64.mut ['n'] = [*1*]; }");
    assert!(rendered.contains("`mut` comes before the type"), "{rendered}");
    assert!(rendered.contains("`var.mut.<type>`"), "{rendered}");
}

#[test]
fn a_declaration_always_says_what_it_declares() {
    let rendered = errors("START { var.immut ['n'] = [*1*]; }");
    assert!(rendered.contains("does not say what it is declaring"), "{rendered}");
    assert!(rendered.contains("a written value means nothing without one"), "{rendered}");
}

#[test]
fn a_declaration_always_says_whether_it_can_change() {
    // The same rule as visibility, applied where it had not been: silence is not an
    // answer, it is the absence of one.
    assert!(check("START { var.immut.i64 ['n'] = [*1*]; }").ok());
    assert!(check("START { var.mut.i64 ['n'] = [*1*]; }").ok());

    let rendered = errors("START { var.i64 ['n'] = [*1*]; }");
    assert!(rendered.contains("does not say whether it can change"), "{rendered}");
    assert!(rendered.contains("silence is not one of them"), "{rendered}");
    assert!(rendered.contains("`var.immut.<type>` if it never changes"), "{rendered}");

    let twice = errors("START { var.mut.immut.i64 ['n'] = [*1*]; }");
    assert!(twice.contains("says twice whether it can change"), "{twice}");
}

#[test]
fn a_name_inside_a_longer_value_needs_something_that_is_not_built() {
    // Joining a name to text builds a *new* value, which needs the collector. Copying a
    // whole one does not, and works.
    let rendered = errors("START { var.immut.str ['a'] = [*x*]; var.immut.str ['b'] = ['a' *y*]; }");
    assert!(rendered.contains("cannot be one piece of a longer value yet"), "{rendered}");
    assert!(rendered.contains("needs the collector"), "{rendered}");

    assert!(check("START { var.immut.str ['a'] = [*x*]; var.immut.str ['b'] = ['a']; }").ok());
}

#[test]
fn everything_wrong_is_reported_and_not_the_first_thing() {
    let source = "\
START {
    var.immut.b17 ['a'] = [*1*];
    var.immut.i64 ['b'] = [*hello*];
    print.stdout['nope'];
}
";
    assert_eq!(codes(source), ["E0402", "E0407", "E0413"], "{}", errors(source));
}

// --- arithmetic ---------------------------------------------------------------------

#[test]
fn the_precedence_mathematics_settled_is_kept() {
    // `x` before `+`, and comparison looser than both. Nobody has to be told these.
    for source in [
        "START { var.immut.i64 ['n'] = [*1* + *2* x *3*]; }",
        "START { var.immut.i64 ['n'] = [*1* x *2* + *3*]; }",
        "START { var.immut.bool ['b'] = [*1* + *2* < *4*]; }",
        "START { var.immut.i64 ['n'] = [*10* / *2* - *1*]; }",
    ] {
        assert!(check(source).ok(), "{source}\n{}", errors(source));
    }
}

#[test]
fn what_mathematics_left_open_is_refused_with_both_readings() {
    let source = "START { var.immut.i64 ['n'] = [*10* mod *3* + *1*]; }";
    let rendered = errors(source);
    assert!(rendered.contains("`mod` and `+` have no agreed order"), "{rendered}");
    assert!(rendered.contains("could be read two ways"), "{rendered}");
    assert!(rendered.contains("which of these first?"), "{rendered}");
    assert!(rendered.contains("this one"), "{rendered}");
    assert!(rendered.contains("or this one"), "{rendered}");
    assert!(rendered.contains("Error code: E0421"), "{rendered}");

    // And brackets settle it, both ways.
    assert!(check("START { var.immut.i64 ['n'] = [(*10* mod *3*) + *1*]; }").ok());
    assert!(check("START { var.immut.i64 ['n'] = [*10* mod (*3* + *1*)]; }").ok());
}

#[test]
fn one_unsettled_operator_on_its_own_is_fine() {
    // There is nothing to be ambiguous *with*. The rule is about two operators, not
    // about `mod` being suspicious.
    assert!(check("START { var.immut.i64 ['n'] = [*10* mod *3*]; }").ok());
}

#[test]
fn a_chain_of_comparisons_is_not_settled_either() {
    // Mathematics reads `a < b < c` as two comparisons joined; most languages read it
    // as one comparison against a boolean. Nobody agreed, so nobody guesses.
    let rendered = errors("START { var.immut.bool ['b'] = [*1* < *2* < *3*]; }");
    assert!(rendered.contains("have no agreed order"), "{rendered}");
}

#[test]
fn a_line_that_both_joins_and_adds_says_which_it_meant() {
    let rendered = errors("START { var.immut.i64 ['n'] = [*1* *2* + *3*]; }");
    assert!(rendered.contains("some of these are joined and some are added"), "{rendered}");
    assert!(rendered.contains("a value does one or the other"), "{rendered}");
}

#[test]
fn arithmetic_is_for_numbers() {
    let rendered = errors("START { var.immut.str ['s'] = [*a*]; var.immut.i64 ['n'] = ['s' + *1*]; }");
    assert!(rendered.contains("`+` works on numbers"), "{rendered}");
    assert!(rendered.contains("nothing converts on its own"), "{rendered}");
}

#[test]
fn a_comparison_is_a_bool_and_a_sum_is_not() {
    assert!(check("START { var.immut.bool ['b'] = [*1* < *2*]; }").ok());

    let rendered = errors("START { var.immut.i64 ['n'] = [*1* < *2*]; }");
    assert!(rendered.contains("works out to a `bool`"), "{rendered}");
    assert!(rendered.contains("given to an `i64`"), "{rendered}");
}

#[test]
fn a_power_is_built_now() {
    assert!(check("START { var.immut.i64 ['n'] = [*2* ^ *8*]; }").ok());
    assert!(check("START { var.immut.i64 ['n'] = [*2* xx *8*]; }").ok(), "the other spelling");
    assert!(check("START { var.immut.e ['n'] = [*2* ^ *-1*]; }").ok(), "an `e` takes a negative one");
}

#[test]
fn every_operator_the_language_has_is_built() {
    // This used to be the list of ones that were not. It is empty now.
    for source in [
        "START { var.immut.i64 ['n'] = [*7* + *1* - *2* \u{d7} *3* / *4*]; }",
        "START { var.immut.i64 ['n'] = [*7* mod *3*]; }",
        "START { var.immut.i64 ['n'] = [*2* ^ *8*]; }",
        "START { var.immut.bool ['b'] = [*true* and *false*]; }",
        "START { var.immut.bool ['b'] = [*true* or *false*]; }",
        "START { var.immut.bool ['b'] = [not *true*]; }",
        "START { var.immut.bool ['b'] = [*1* </= *2*]; }",
    ] {
        assert!(check(source).ok(), "{source}\n{}", errors(source));
    }
}

// --- arrays -------------------------------------------------------------------------

#[test]
fn an_array_says_its_size_in_its_type() {
    let out = check("START { var.immut.arr.i64 (5) ['xs'] = [[*1* *2* *3* *4* *5*]]; }");
    assert!(out.ok(), "{}", errors("START { var.immut.arr.i64 (5) ['xs'] = [[*1* *2* *3* *4* *5*]]; }"));
    assert_eq!(out.locals()[0].ty.name(), "arr.i64 (5)");
}

#[test]
fn the_shape_and_the_elements_have_to_agree() {
    let rendered = errors("START { var.immut.arr.i64 (5) ['xs'] = [[*1* *2*]]; }");
    assert!(rendered.contains("this holds 5 element(s), and 2 were written"), "{rendered}");
    assert!(rendered.contains("written flat, row by row"), "{rendered}");
    assert!(rendered.contains("declared (5)"), "{rendered}");
}

#[test]
fn a_shape_is_written_without_marks_because_it_is_part_of_a_type() {
    // `(5)`, not `(*5*)`. Marks tell a name from a written value, and a size is neither.
    assert!(check("START { var.immut.arr.i64 (5) ['xs'] = [[*1* *2* *3* *4* *5*]]; }").ok());

    let rendered = errors("START { var.immut.i64 ['n'] = [5]; }");
    assert!(rendered.contains("a bare number is a size, not a value"), "{rendered}");
    assert!(rendered.contains("`*5*`"), "{rendered}");
}

#[test]
fn an_array_has_to_say_how_big_it_is() {
    let rendered = errors("START { var.immut.arr.i64 ['xs'] = [[*1*]]; }");
    assert!(rendered.contains("does not say how big it is"), "{rendered}");
    assert!(rendered.contains("`grow` is a size too"), "{rendered}");
    assert!(rendered.contains("`arr.i64 (grow)`"), "{rendered}");
}

#[test]
fn only_an_array_has_a_shape() {
    let rendered = errors("START { var.immut.i64 (5) ['n'] = [*1*]; }");
    assert!(rendered.contains("only an array has a shape"), "{rendered}");
}

#[test]
fn an_index_may_stop_where_an_allocation_ends_and_nowhere_else() {
    // One `arr` is one allocation, so a `(2 3)` takes two indices and no fewer.
    let rendered = errors("START { var.immut.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]]; print.stdout['m'[*1*]]; }");
    assert!(rendered.contains("this takes 2 index(es), and 1 were given"), "{rendered}");
    assert!(rendered.contains("declared `arr.i64 (2 3)`"), "{rendered}");

    // Two `arr` links are two allocations, so stopping between them is a place to
    // stop -- and what it hands back is the array that lives there.
    let out = check("START { var.immut.arr.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]]; var.immut.arr.i64 (3) ['row'] = [share 'm'[*2*]]; }");
    assert!(out.ok(), "{}", errors("START { var.immut.arr.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]]; var.immut.arr.i64 (3) ['row'] = [share 'm'[*2*]]; }"));
}

#[test]
fn only_an_array_can_be_indexed() {
    let rendered = errors("START { var.immut.i64 ['n'] = [*1*]; print.stdout['n'[*1*]]; }");
    assert!(rendered.contains("`i64` is not an array"), "{rendered}");
}

#[test]
fn every_element_is_the_type_the_array_said() {
    let rendered = errors("START { var.immut.arr.i64 (2) ['xs'] = [[*1* *hello*]]; }");
    assert!(rendered.contains("is not a whole number"), "{rendered}");
}

#[test]
fn every_arr_link_is_one_allocation_and_says_how_big() {
    // One size per link, and the innermost takes what is left -- which is what keeps
    // `arr.i64 (2 3)` a rectangle while `arr.arr.i64 (2 3)` is two rows of three.
    let out = check("START { var.immut.arr.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]]; }");
    assert!(out.ok(), "{}", errors("START { var.immut.arr.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]]; }"));
    assert_eq!(out.locals()[0].ty.name(), "arr.arr.i64 (2 3)", "named the way it was written");

    let rendered = errors("START { var.immut.arr.arr.i64 (2) ['m'] = [[*1* *2*]]; }");
    assert!(rendered.contains("this says `arr` two times and gives one size."), "{rendered}");
    assert!(rendered.contains("every `arr` link is one allocation"), "{rendered}");
}

#[test]
fn an_array_holds_any_of_the_types_that_are_built() {
    // It used to hold `i64` and nothing else. A slot is the same width whatever is in
    // it, so what was missing was telling the runtime which -- not room for it.
    for source in [
        "START { var.immut.arr.i64 (2) ['xs'] = [[*1* *2*]]; }",
        "START { var.immut.arr.bool (2) ['xs'] = [[*true* *false*]]; }",
        "START { var.immut.arr.str (2) ['xs'] = [[*a* *b*]]; }",
        "START { var.immut.arr.e (2) ['xs'] = [[*1/3* *0.5*]]; }",
    ] {
        assert!(check(source).ok(), "{source}\n{}", errors(source));
    }

    // An element is read under the element type, the same way a declaration's value is.
    assert_eq!(codes("START { var.immut.arr.i64 (1) ['xs'] = [[*0.5*]]; }"), ["E0407"]);
    assert!(check("START { var.immut.arr.e (1) ['xs'] = [[*0.5*]]; }").ok());
}

#[test]
fn an_array_of_nothing_is_refused() {
    let rendered = errors("START { var.immut.arr.i64 (0) ['xs'] = [[]]; }");
    assert!(rendered.contains("an array of nothing holds nothing"), "{rendered}");
}

// --- changing things ----------------------------------------------------------------

#[test]
fn mut_finally_means_something() {
    // It has been a word in the chain that did nothing since the chain was designed.
    assert!(check("START { var.mut.i64 ['n'] = [*0*]; set ['n'] = [*5*]; }").ok());

    let rendered = errors("START { var.immut.i64 ['total'] = [*0*]; set ['total'] = [*55*]; }");
    assert!(
        rendered.contains("`'total'` cannot be changed, because its declaration never said it could."),
        "{rendered}"
    );
    assert!(rendered.contains("declared `immut` here"), "{rendered}");
    assert!(rendered.contains("changed here"), "{rendered}");
    assert!(rendered.contains("a variable changes only if its declaration says `mut`"), "{rendered}");
    // The fix is the line they wanted, not a description of it -- and `immut` is
    // replaced rather than `mut` inserted, or it would say both.
    let out = check("START { var.immut.i64 ['total'] = [*0*]; set ['total'] = [*55*]; }");
    let fixes = out.errors[0].fixes.join(" ");
    assert_eq!(fixes, "`var.mut.i64`", "the fix must not keep the word it replaces");
}

#[test]
fn changing_something_that_was_never_declared() {
    let rendered = errors("START { set ['nope'] = [*1*]; }");
    assert!(rendered.contains("`'nope'` is not declared"), "{rendered}");
}

#[test]
fn what_is_put_in_has_to_fit_what_is_there() {
    let rendered = errors("START { var.mut.i64 ['n'] = [*1*]; set ['n'] = [*hello*]; }");
    assert!(rendered.contains("is not a whole number"), "{rendered}");
}

#[test]
fn only_an_array_has_an_element_to_change() {
    let rendered = errors("START { var.mut.i64 ['n'] = [*1*]; set ['n'[*1*]] = [*2*]; }");
    assert!(rendered.contains("`i64` is not an array"), "{rendered}");
}

#[test]
fn set_gives_one_value_for_each_thing_it_changes() {
    let rendered = errors("START { var.mut.i64 ['a', 'b'] = [*1*, *2*]; set ['a', 'b'] = [*9*]; }");
    assert!(rendered.contains("two things changed, and one value given"), "{rendered}");
}

// --- deciding -----------------------------------------------------------------------

#[test]
fn a_condition_is_a_bool_and_nothing_is_truthy() {
    assert!(check("START { var.immut.i64 ['n'] = [*1*]; if 'n' > *0* { print.stdout[str:*yes*]; } }").ok());
    assert!(check("START { var.immut.bool ['f'] = [*true*]; if 'f' { print.stdout[str:*yes*]; } }").ok());

    let rendered = errors("START { var.immut.i64 ['n'] = [*1*]; if 'n' { print.stdout[str:*yes*]; } }");
    assert!(rendered.contains("`if` asks something true or false, and this is an `i64`"), "{rendered}");
    assert!(rendered.contains("nothing is truthy"), "{rendered}");
    assert!(rendered.contains("such as `> *0*`"), "{rendered}");
}

#[test]
fn an_arm_is_a_scope_of_its_own() {
    // An `if` introduces nothing, so what is declared inside one is gone at the brace.
    let rendered = errors(
        "START { if *true* == *true* { var.immut.i64 ['inside'] = [*1*]; } print.stdout['inside']; }",
    );
    assert!(rendered.contains("`'inside'` is not declared"), "{rendered}");

    // And the same name may be used again afterwards, since the first one is gone.
    assert!(check(
        "START { if *true* == *true* { var.immut.i64 ['x'] = [*1*]; } var.immut.i64 ['x'] = [*2*]; }"
    )
    .ok());
}

#[test]
fn a_wrong_condition_does_not_hide_what_is_inside_the_arm() {
    let out = check("START { var.immut.i64 ['n'] = [*1*]; if 'n' { print.stdout['nope']; } }");
    let codes: Vec<&str> = out.errors.iter().map(|e| e.code.as_str()).collect();
    assert!(codes.contains(&"E0440"), "{codes:?}");
    assert!(codes.contains(&"E0413"), "the undeclared name inside is reported too: {codes:?}");
}

#[test]
fn the_same_or_not_works_on_anything_but_larger_only_on_numbers() {
    // Two things are the same or they are not, whatever they are. Which of two is
    // *larger* only means something for numbers.
    assert!(check("START { var.immut.bool ['b'] = [*true* == *false*]; }").ok());
    assert!(check("START { var.immut.bool ['b'] = [*1* == *2*]; }").ok());

    let rendered = errors("START { var.immut.bool ['b'] = [*true* > *false*]; }");
    assert!(rendered.contains("`>` works on numbers"), "{rendered}");

    let mixed = errors("START { var.immut.str ['s'] = [*a*]; var.immut.bool ['b'] = ['s' == *1*]; }");
    assert!(mixed.contains("compares two of the same thing"), "{mixed}");
    assert!(mixed.contains("two types are never equal"), "{mixed}");
}

#[test]
fn types_get_the_right_article() {
    // A language selling its error messages cannot write "a i64" in one.
    let rendered = errors("START { var.immut.str ['s'] = [*a*]; var.immut.i64 ['n'] = ['s']; }");
    assert!(rendered.contains("this is a `str`"), "{rendered}");
    assert!(rendered.contains("given to an `i64`"), "{rendered}");
}

#[test]
fn a_print_says_where_it_goes() {
    assert!(check("START { print.stdout[str:*hi*]; }").ok());
    assert!(check("START { print.stderr[str:*hi*]; }").ok());

    let rendered = errors("START { print.somewhere[str:*hi*]; }");
    assert!(rendered.contains("`somewhere` is not somewhere to print"), "{rendered}");
    assert!(rendered.contains("a reader should not have to know a default"), "{rendered}");
    assert!(rendered.contains("there is `stdout` and `stderr`"), "{rendered}");
}

#[test]
fn a_counter_belongs_to_its_loop() {
    // Not the `immut` error, and deliberately so: the counter does change, every pass.
    // What is wrong is who changes it, and that is a different sentence.
    let source = "\
START {
    loop.temp.range.i64 ['i'] = [*1*, *5*] {
        set ['i'] = [*9*];
    }
}
";
    let rendered = errors(source);
    assert!(rendered.contains("`'i'` is a loop's counter, and the loop is what moves it."), "{rendered}");
    assert!(rendered.contains("Error code: E0454"), "{rendered}");
    assert!(rendered.contains("the loop counts this"), "{rendered}");
    assert!(rendered.contains("and this would move it too"), "{rendered}");
}

#[test]
fn a_counting_loop_says_how_long_its_counter_lives() {
    // The rule is flat: `range` always has a counter, `while` never does. So `temp` and
    // `perm` are required on one and refused on the other, and neither has a default.
    assert_eq!(codes("START { loop.range.i64 ['i'] = [*1*, *5*] { } }"), ["E0451"]);
    assert_eq!(codes("START { loop.temp.range ['i'] = [*1*, *5*] { } }"), ["E0452"]);
    assert_eq!(
        codes("START { var.mut.i64 ['d'] = [*1*]; loop.perm.while 'd' > *0* { } }"),
        ["E0449"]
    );
}

#[test]
fn break_looks_past_every_if_to_the_nearest_loop() {
    assert_eq!(codes("START { break; }"), ["E0446"]);
    assert_eq!(codes("START { if *1* == *1* { break; } }"), ["E0446"]);
    assert!(check("START { loop.temp.range.i64 ['i'] = [*1*, *5*] { if 'i' == *3* { break; } } }").ok());
}

#[test]
fn nothing_under_a_break_can_run() {
    let source = "\
START {
    loop.temp.range.i64 ['i'] = [*1*, *5*] {
        break;
        print.stdout[\\n];
    }
}
";
    let rendered = errors(source);
    assert!(rendered.contains("nothing here can run."), "{rendered}");
    assert!(rendered.contains("Error code: E0445"), "{rendered}");
    assert!(rendered.contains("the loop is left here"), "{rendered}");
}

#[test]
fn a_temp_counter_is_gone_afterwards_and_a_perm_one_is_not() {
    assert_eq!(
        codes("START { loop.temp.range.i64 ['i'] = [*1*, *5*] { } print.stdout['i' \\n]; }"),
        ["E0413"],
        "`temp` means what it says"
    );
    assert!(
        check("START { loop.perm.range.i64 ['i'] = [*1*, *5*] { } print.stdout['i' \\n]; }").ok(),
        "`perm` is the one thing in Quench that outlives its block"
    );
}

#[test]
fn count_is_answered_where_the_shape_was_written() {
    // A shape never changes, so this is a number long before anything runs -- which is
    // why a loop bounded by `count` costs nothing at all.
    let out = check("START { var.immut.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]]; var.immut.i64 ['n'] = [count['m']]; }");
    assert!(out.ok());
    let quench_check::Stmt::Declare { value, .. } = &out.body()[1] else { panic!() };
    assert_eq!(*value, quench_check::Value::Number(6), "every element, however many dimensions");

    assert_eq!(codes("START { var.immut.i64 ['n'] = [*1*]; var.immut.i64 ['c'] = [count['n']]; }"), ["E0457"]);
    assert_eq!(codes("START { var.immut.i64 ['c'] = [size['n']]; }"), ["E0455"]);
}

#[test]
fn a_function_says_what_it_gives_back_and_who_can_see_it() {
    assert_eq!(codes("fn.file ['a'] [] { give [*1*]; }\nSTART { }"), ["E0464"]);
    assert_eq!(codes("fn.i64 ['a'] [] { give [*1*]; }\nSTART { }"), ["E0459"]);
    assert_eq!(codes("const.i64 ['A'] = [*1*];\nSTART { }"), ["E0459"]);
    // `nothing` is a real link rather than an omission, so this is fine.
    assert!(check("fn.file.nothing ['a'] [] { print.stdout[\\n]; }\nSTART { }").ok());
}

#[test]
fn a_function_that_answers_answers_on_every_way_out() {
    let source = "\
fn.file.i64 ['bigger'] [immut.i64 'n'] {
    if 'n' > *0* {
        give ['n'];
    }
}
START { }
";
    let rendered = errors(source);
    assert!(rendered.contains("this function says it gives back an `i64`, and does not always."), "{rendered}");
    assert!(rendered.contains("Error code: E0466"), "{rendered}");

    // With an `else`, every way out ends in a `give`, and it checks out.
    assert!(check("\
fn.file.i64 ['bigger'] [immut.i64 'n'] {
    if 'n' > *0* { give ['n']; } else { give [*0*]; }
}
START { }
").ok());
}

#[test]
fn a_call_is_checked_against_what_was_declared() {
    let one = "fn.file.i64 ['twice'] [immut.i64 'n'] { give ['n' + 'n']; }\n";
    assert_eq!(codes(&format!("{one}START {{ var.immut.i64 ['x'] = [twice[*1*, *2*]]; }}")), ["E0470"]);
    assert_eq!(codes(&format!("{one}START {{ var.immut.i64 ['x'] = [twice[*a*]]; }}")), ["E0407"]);
    assert_eq!(codes(&format!("{one}START {{ var.immut.str ['x'] = [twice[*1*]]; }}")), ["E0406"]);
    assert!(check(&format!("{one}START {{ var.immut.i64 ['x'] = [twice[*21*]]; }}")).ok());
}

#[test]
fn a_function_written_underneath_can_still_be_called() {
    // Signatures are collected before any body is read, which is what lets two
    // functions call each other and one call itself.
    assert!(check("\
fn.file.bool ['even'] [immut.i64 'n'] {
    if 'n' == *0* { give [*true*]; } else { give [odd['n' - *1*]]; }
}
fn.file.bool ['odd'] [immut.i64 'n'] {
    if 'n' == *0* { give [*false*]; } else { give [even['n' - *1*]]; }
}
START { }
").ok());
}

#[test]
fn a_program_does_not_rewrite_what_it_was_written_with() {
    let source = "const.file.i64 ['A'] = [*1*];\nSTART { set ['A'] = [*2*]; }";
    let rendered = errors(source);
    assert!(rendered.contains("`'A'` is a constant."), "{rendered}");
    assert!(rendered.contains("and changed here"), "{rendered}");
    assert!(rendered.contains("Error code: E0472"), "{rendered}");

    assert_eq!(codes("const.file.mut.i64 ['A'] = [*1*];\nSTART { }"), ["E0473"]);

    // Named as a value, it is that value -- and it needs no storage to be one.
    assert!(check("const.file.i64 ['A'] = [*1*];\nSTART { print.stdout['A' \\n]; }").ok());
}

#[test]
fn a_constant_array_lives_in_the_module() {
    // Beside the text, which every engine lays out before anything runs -- so its
    // handle is known here and naming it costs nothing at all.
    for source in [
        "const.file.arr.i64 (2) ['A'] = [[*1* *2*]];\nSTART { }",
        "const.file.arr.str (2) ['A'] = [[*a* *b*]];\nSTART { }",
        "const.file.arr.arr.i64 (2 2) ['A'] = [[*1* *2* *3* *4*]];\nSTART { }",
    ] {
        assert!(check(source).ok(), "{source}\n{}", errors(source));
    }

    // It has somewhere it lives, so it is indexed like any other array.
    assert!(check("const.file.arr.i64 (2) ['A'] = [[*1* *2*]];\nSTART { print.stdout['A'[*1*] \\n]; }").ok());
    // What nothing can do is change it.
    assert_eq!(
        codes("const.file.arr.i64 (2) ['A'] = [[*1* *2*]];\nSTART { set ['A'[*1*]] = [*9*]; }"),
        ["E0472"]
    );

    // What is written down is however many were written, so it cannot grow.
    assert_eq!(codes("const.file.arr.i64 (grow) ['A'] = [[*1*]];\nSTART { }"), ["E0460"]);
    // And an `e` slot holds a handle the runtime makes, not a number the module carries.
    assert_eq!(codes("const.file.arr.e (2) ['A'] = [[*1/2* *3*]];\nSTART { }"), ["E0485"]);
}

#[test]
fn a_parameter_is_a_variable_and_says_so() {
    assert_eq!(codes("fn.file.i64 ['a'] [i64 'n'] { give ['n']; }\nSTART { }"), ["E0465"]);
    assert!(check("fn.file.i64 ['a'] [immut.i64 'n'] { give ['n']; }\nSTART { }").ok());
}

#[test]
fn start_has_nobody_to_answer() {
    let rendered = errors("START { give [*1*]; }");
    assert!(rendered.contains("`START` has nobody to give an answer to."), "{rendered}");
    assert!(rendered.contains("`START` is where the program begins"), "{rendered}");
    // But leaving early is a thing you do on purpose, and works.
    assert!(check("START { give; }").ok());
}

#[test]
fn an_exact_number_is_a_type_now() {
    let out = check("START { var.immut.e ['a'] = [*-3/4*]; }");
    assert!(out.ok(), "{}", errors("START { var.immut.e ['a'] = [*-3/4*]; }"));
    assert_eq!(out.locals()[0].ty, quench_check::Ty::Exact);
    assert_eq!(out.locals()[0].ty.name(), "e");

    // Whole, ratio and decimal, and the decimal is exact.
    assert!(check("START { var.immut.e ['a'] = [*12*]; }").ok());
    assert!(check("START { var.immut.e ['a'] = [*0.1*]; }").ok());
    assert_eq!(codes("START { var.immut.e ['a'] = [*hello*]; }"), ["E0474"]);
}

#[test]
fn an_i64_and_an_e_do_not_mix() {
    // They are both numbers and they are not the same number, and nothing in Quench
    // converts on its own.
    let source = "START {
    var.immut.e ['a'] = [*1*];
    var.immut.i64 ['b'] = [*2*];
    var.immut.e ['c'] = ['a' + 'b'];
}";
    let rendered = errors(source);
    assert!(rendered.contains("an `e` and an `i64`"), "{rendered}");
    assert!(rendered.contains("Error code: E0420"), "{rendered}");
}

#[test]
fn an_exact_division_leaves_no_remainder_to_ask_about() {
    let rendered = errors("START { var.immut.e ['a'] = [*7*]; var.immut.e ['b'] = ['a' mod *2*]; }");
    assert!(rendered.contains("`mod` asks what a division left over, and an `e` division leaves nothing."), "{rendered}");
    assert!(rendered.contains("Error code: E0476"), "{rendered}");
}

#[test]
fn a_chain_says_what_its_numbers_are() {
    // `*0.1*` is one tenth under an `e` chain and a mistake under an `i64` one -- the
    // same rule that makes `*1000*` a number under `i64` and four characters under `str`.
    assert!(check("START { var.immut.e ['a'] = [*0.1* + *0.2*]; }").ok());
    assert_eq!(codes("START { var.immut.i64 ['a'] = [*0.1* + *0.2*]; }"), ["E0407"]);

    // Where the chain cannot say -- a comparison under a `bool` -- the value says it.
    assert!(check("START { var.immut.bool ['a'] = [e:*0.1* == e:*0.3*]; }").ok());
    // And where the chain did say, saying it again is still refused.
    assert_eq!(codes("START { var.immut.str ['a'] = [str:*twice*]; }"), ["E0107"]);
}

#[test]
fn and_or_and_not_are_built_and_are_for_bool() {
    assert!(check("START { var.immut.bool ['a'] = [*true*]; var.immut.bool ['b'] = ['a' and 'a']; }").ok());
    assert!(check("START { var.immut.bool ['a'] = [*true*]; var.immut.bool ['b'] = [not 'a']; }").ok());

    // Nothing is truthy, here as everywhere else.
    let rendered = errors("START { var.immut.i64 ['n'] = [*1*]; var.immut.bool ['b'] = ['n' and 'n']; }");
    assert!(rendered.contains("`and` joins two things that are true or false."), "{rendered}");
    assert!(rendered.contains("Error code: E0422"), "{rendered}");

    let rendered = errors("START { var.immut.i64 ['n'] = [*1*]; var.immut.bool ['b'] = [not 'n']; }");
    assert!(rendered.contains("`not` turns a `bool` round, and this is an `i64`."), "{rendered}");
    assert!(rendered.contains("Error code: E0418"), "{rendered}");
}

#[test]
fn the_logical_operators_have_no_agreed_order() {
    // C put `&` too loose and Python put it too tight, and both produced famous traps.
    // The lesson is not that C chose wrong but that there was nothing to choose.
    let source = "START {
    var.immut.bool ['a'] = [*true*];
    var.immut.bool ['b'] = ['a' and 'a' or 'a'];
}";
    let rendered = errors(source);
    assert!(rendered.contains("`and` and `or` have no agreed order"), "{rendered}");
    assert!(rendered.contains("Error code: E0421"), "{rendered}");

    // And against a comparison, for the same reason.
    assert_eq!(
        codes("START { var.immut.i64 ['n'] = [*1*]; var.immut.bool ['b'] = ['n' > *0* and 'n' < *9*]; }"),
        ["E0421"]
    );
    // With brackets, it is one reading and it checks out.
    assert!(check("START { var.immut.i64 ['n'] = [*1*]; var.immut.bool ['b'] = [('n' > *0*) and ('n' < *9*)]; }").ok());
}

#[test]
fn comparing_two_arrays_asks_about_their_contents() {
    // Not whether they are the same array. `share` is what makes two names for one, and
    // this is the other question -- which is why both had to be sayable before this was.
    assert!(check("START {
    var.immut.arr.i64 (2) ['a'] = [[*1* *2*]];
    var.immut.arr.i64 (2) ['b'] = [[*1* *2*]];
    var.immut.bool ['s'] = ['a' == 'b'];
}").ok());
}

#[test]
fn binding_an_array_says_share_or_copy() {
    let source = "START {
    var.mut.arr.i64 (2) ['a'] = [[*1* *2*]];
    var.mut.arr.i64 (2) ['b'] = ['a'];
}";
    let rendered = errors(source);
    assert!(rendered.contains("this does not say whether it shares `\'a\'` or copies it."), "{rendered}");
    assert!(rendered.contains("Error code: E0478"), "{rendered}");

    for said in ["share", "copy"] {
        let source = format!("START {{
    var.mut.arr.i64 (2) ['a'] = [[*1* *2*]];
    var.mut.arr.i64 (2) ['b'] = [{said} 'a'];
}}");
        assert!(check(&source).ok(), "{}", errors(&source));
    }

    // Everything else is a value, so naming it again is naming the value and there is
    // nothing to share.
    assert_eq!(
        codes("START { var.immut.i64 ['n'] = [*1*]; var.immut.i64 ['m'] = [share 'n']; }"),
        ["E0479"]
    );

    // Printing an array is not binding it, and indexing one is not either.
    assert!(check("START { var.immut.arr.i64 (2) ['a'] = [[*1* *2*]]; print.stdout['a' \n]; }").ok());
    assert!(check("START { var.immut.arr.i64 (2) ['a'] = [[*1* *2*]]; var.immut.i64 ['n'] = ['a'[*1*]]; }").ok());
}

#[test]
fn text_and_flags_can_be_compared_and_not_ordered() {
    assert!(check("START { var.immut.str ['a'] = [*x*]; var.immut.bool ['s'] = ['a' == 'a']; }").ok());
    assert!(check("START { var.immut.bool ['a'] = [*true*]; var.immut.bool ['s'] = ['a' != 'a']; }").ok());
    // Which of two is *larger* still only means something for numbers.
    assert_eq!(
        codes("START { var.immut.str ['a'] = [*x*]; var.immut.bool ['s'] = ['a' < 'a']; }"),
        ["E0420"]
    );
}

#[test]
fn an_array_can_be_printed() {
    // It used to reach an `unreachable!` in the lowering, which is the worst answer a
    // language with this one's selling point could give.
    assert!(check("START { var.immut.arr.i64 (2) ['xs'] = [[*1* *2*]]; print.stdout['xs' \\n]; }").ok());
}

#[test]
fn a_size_may_say_grow_and_only_the_first_may() {
    assert!(check("START { var.mut.arr.i64 (grow) ['xs'] = [[*1*]]; }").ok());
    assert!(check("START { var.mut.arr.arr.i64 (grow grow) ['j'] = [[]]; }").ok());
    assert!(check("START { var.mut.arr.arr.i64 (2 grow) ['j'] = [[]]; }").ok());

    // Finding an element is `(i - 1) x stride + j`, and a stride is the sizes under a
    // dimension -- so the outermost is the only one whose size is never asked for.
    let rendered = errors("START { var.mut.arr.i64 (3 grow) ['xs'] = [[*1*]]; }");
    assert!(rendered.contains("only the first size of an allocation can grow."), "{rendered}");
    assert!(rendered.contains("Error code: E0480"), "{rendered}");
}

#[test]
fn only_something_that_grows_can_be_added_to() {
    let rendered = errors("START { var.mut.arr.i64 (3) ['xs'] = [[*1* *2* *3*]]; add ['xs'] = [*4*]; }");
    assert!(rendered.contains("`arr.i64 (3)` does not grow."), "{rendered}");
    assert!(rendered.contains("Error code: E0482"), "{rendered}");

    // A shape is part of a type, and a type does not change while a program runs.
    assert!(rendered.contains("a shape is part of a type"), "{rendered}");
}

#[test]
fn what_grows_under_something_can_only_be_written_empty() {
    let rendered = errors("START { var.mut.arr.arr.i64 (grow grow) ['j'] = [[*1* *2*]]; }");
    assert!(rendered.contains("nothing here says where one row ends."), "{rendered}");
    assert!(rendered.contains("Error code: E0484"), "{rendered}");
}

#[test]
fn count_counts_any_array_and_folds_where_it_can() {
    // A row of a jagged array is exactly the thing whose length nothing else can say.
    assert!(check("START {
    var.mut.arr.arr.i64 (grow grow) ['j'] = [[]];
    print.stdout[count['j'] str:* * count['j'[*1*]] \\n];
}").ok());

    let out = check("START { var.immut.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]]; var.immut.i64 ['n'] = [count['m']]; }");
    let quench_check::Stmt::Declare { value, .. } = &out.body()[1] else { panic!() };
    assert_eq!(*value, quench_check::Value::Number(6), "a fixed shape is known here");
}
