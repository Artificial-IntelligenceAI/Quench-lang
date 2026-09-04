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
    assert_eq!(out.locals()[0].ty, quench_check::Ty::Int { bits: 64, signed: true });
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
fn a_type_that_is_not_a_type_does_not_hide_the_rest_of_the_file() {
    // A declaration whose type is a typo is abandoned -- there is nothing to declare
    // it as -- but everything after it is still checked. Stopping there would leave a
    // reader fixing one mistake at a time and running again to find the next.
    let source = "\
START {
    var.immut.b17 ['x'] = [*1*];
    var.immut.str ['y'] = [*a*];
    var.immut.i64 ['y'] = [*2*];
    print.stdout['nope'];
}
";
    let found = codes(source);
    assert!(found.contains(&"E0402".to_string()), "{}", errors(source));
    assert!(found.contains(&"E0201".to_string()), "{}", errors(source));
    assert!(found.contains(&"E0413".to_string()), "{}", errors(source));
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
fn a_word_that_is_not_a_type_says_so_wherever_it_is_written() {
    // There used to be a second answer here -- "not built yet", for a type Quench meant
    // to have and did not. Every type on that list is built, so the only thing left to
    // say is that a word is not one, and it is said the same way in all three places a
    // type can be written.
    for source in [
        "START { var.immut.b17 ['x'] = [*1*]; }",
        "START { print.stdout[b17:*1*]; }",
        "START { loop.temp.range.b17 ['i'] = [*1*, *2*] { } }",
        "fn.file.b17 ['f'] [] { give [*1*]; } START { }",
    ] {
        let rendered = errors(source);
        assert!(rendered.contains("`b17` is not a type"), "{source}\n{rendered}");
        assert!(rendered.contains("Error code: E0402"), "{source}\n{rendered}");
    }
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
    assert!(rendered.contains("`hello` is not an `i64`"), "{rendered}");

    // And the same digits are a different thing under each whole-number type: a `u8`
    // holds two hundred and an `i8` does not, which is a mistake about the type rather
    // than about the number, and the message says which.
    assert!(check("START { var.immut.u8 ['n'] = [*200*]; }").ok());
    let too_big = errors("START { var.immut.i8 ['n'] = [*200*]; }");
    assert!(too_big.contains("`200` does not fit in an `i8`"), "{too_big}");
    assert!(too_big.contains("`i8` holds -128 to 127"), "{too_big}");
    let negative = errors("START { var.immut.u8 ['n'] = [*-1*]; }");
    assert!(negative.contains("holds no negative number"), "{negative}");

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
fn pieces_side_by_side_join_whether_or_not_they_are_known() {
    // Which is what juxtaposition already meant. This used to be refused as "needing
    // the collector" -- it needs allocation, which exists, not collection, which does
    // not: a built piece of text leaks like every array already does.
    assert!(check("START { var.immut.str ['a'] = [*x*]; var.immut.str ['b'] = ['a' *y*]; }").ok());
    assert!(check("START { var.immut.str ['a'] = [*x*]; var.immut.str ['b'] = ['a']; }").ok());

    // Nothing converts on its own, here as everywhere else.
    let rendered = errors("START { var.immut.i64 ['n'] = [*1*]; var.immut.str ['b'] = [*x* 'n']; }");
    assert!(rendered.contains("this is an `i64`, and text is made of text."), "{rendered}");
    assert!(rendered.contains("showing is not joining"), "{rendered}");
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
    // One spelling, because `^` is on the keyboard. `xx` existed for the days when it
    // was not settled that `**` could never be the one.
    assert!(check("START { var.immut.i64 ['n'] = [*2* ^ *8*]; }").ok());
    assert!(check("START { var.immut.e ['n'] = [*2* ^ *-1*]; }").ok(), "an `e` takes a negative one");
}

#[test]
fn every_operator_the_language_has_is_built() {
    // This used to be the list of ones that were not. It is empty now.
    for source in [
        "START { var.immut.i64 ['n'] = [*7* + *1* - *2* x *3* / *4*]; }",
        "START { var.immut.i64 ['n'] = [*7* mod *3*]; }",
        "START { var.immut.i64 ['n'] = [*2* ^ *8*]; }",
        "START { var.immut.bool ['b'] = [*true* and *false*]; }",
        "START { var.immut.bool ['b'] = [*true* or *false*]; }",
        "START { var.immut.bool ['b'] = [not *true*]; }",
        "START { var.immut.bool ['b'] = [*1* <== *2*]; }",
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
    assert!(rendered.contains("is not an `i64`"), "{rendered}");
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
    let out = check("START { var.immut.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]]; var.immut.i64 ['n'] = [call count['m']]; }");
    assert!(out.ok());
    let quench_check::Stmt::Declare { value, .. } = &out.body()[1] else { panic!() };
    assert_eq!(
        *value,
        quench_check::Value::Number { value: 6, bits: 64, signed: true },
        "every element, however many dimensions"
    );

    assert_eq!(codes("START { var.immut.i64 ['n'] = [*1*]; var.immut.i64 ['c'] = [call count['n']]; }"), ["E0457"]);
    // Without `call` in front of it this is not a call at all, and the parser says so
    // before any of it is looked up.
    assert_eq!(codes("START { var.immut.i64 ['c'] = [size['n']]; }"), ["E0109"]);
    assert_eq!(codes("START { var.immut.i64 ['c'] = [call size['n']]; }"), ["E0455"]);
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
    assert_eq!(codes(&format!("{one}START {{ var.immut.i64 ['x'] = [call 'twice'[*1*, *2*]]; }}")), ["E0470"]);
    assert_eq!(codes(&format!("{one}START {{ var.immut.i64 ['x'] = [call 'twice'[*a*]]; }}")), ["E0407"]);
    assert_eq!(codes(&format!("{one}START {{ var.immut.str ['x'] = [call 'twice'[*1*]]; }}")), ["E0406"]);
    assert!(check(&format!("{one}START {{ var.immut.i64 ['x'] = [call 'twice'[*21*]]; }}")).ok());
}

#[test]
fn a_function_written_underneath_can_still_be_called() {
    // Signatures are collected before any body is read, which is what lets two
    // functions call each other and one call itself.
    assert!(check("\
fn.file.bool ['even'] [immut.i64 'n'] {
    if 'n' == *0* { give [*true*]; } else { give [call 'odd'['n' - *1*]]; }
}
fn.file.bool ['odd'] [immut.i64 'n'] {
    if 'n' == *0* { give [*false*]; } else { give [call 'even'['n' - *1*]]; }
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
    assert!(check("START { var.immut.bool ['a'] = [*true*]; var.immut.bool ['s'] = ['a' !== 'a']; }").ok());
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
    assert!(rendered.contains("only the first size of an allocation can say `grow`."), "{rendered}");
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
    print.stdout[call count['j'] str:* * call count['j'[*1*]] \\n];
}").ok());

    let out = check("START { var.immut.arr.i64 (2 3) ['m'] = [[*1* *2* *3* *4* *5* *6*]]; var.immut.i64 ['n'] = [call count['m']]; }");
    let quench_check::Stmt::Declare { value, .. } = &out.body()[1] else { panic!() };
    assert_eq!(
        *value,
        quench_check::Value::Number { value: 6, bits: 64, signed: true },
        "a fixed shape is known here"
    );
}

#[test]
fn a_name_holds_whatever_a_line_holds() {
    // The marks do the delimiting, so there is no identifier grammar to break. Emoji,
    // scripts that are not Latin, spaces, and the mark itself escaped.
    let source = "\
START {
    var.immut.str ['🔥'] = [*ไฟ*];
    var.immut.i64 ['ผลลัพธ์'] = [*42*];
    var.immut.str ['a name with spaces'] = [*works*];
    var.immut.str ['it\\'s'] = [*escaped*];
    print.stdout['🔥' 'ผลลัพธ์' 'a name with spaces' 'it\\'s'];
}
";
    assert!(codes(source).is_empty(), "{}", errors(source));
}


#[test]
fn a_call_says_that_it_is_one() {
    // Without `call`, a name before a bracket is an index and a bare word before one is
    // nothing at all -- so a reader never has to find a declaration to know whether a
    // line hands control somewhere else.
    let one = "fn.file.i64 ['double'] [immut.i64 'n'] { give ['n' x *2*]; }\n";
    let bare = errors(&format!("{one}START {{ var.immut.i64 ['x'] = [double[*2*]]; }}"));
    assert!(bare.contains("`double` is not something to index"), "{bare}");
    assert!(bare.contains("call double[…]"), "{bare}");
    assert!(bare.contains("Error code: E0109"), "{bare}");

    // A name between marks without `call` is an index, and this one is not an array.
    let indexed = errors(&format!("{one}START {{ var.immut.i64 ['x'] = ['double'[*2*]]; }}"));
    assert!(indexed.contains("is not declared"), "{indexed}");

    assert!(check(&format!("{one}START {{ var.immut.i64 ['x'] = [call 'double'[*2*]]; }}")).ok());
}

#[test]
fn marks_after_call_say_who_made_the_thing_being_called() {
    // `call count[...]` came with Quench and `call 'count'[...]` did not, so nothing the
    // language provides has to be held back from a writer who wanted that name.
    let source = "\
fn.file.i64 ['count'] [immut.i64 'n'] { give ['n' + *1*]; }
START {
    var.immut.arr.i64 (2) ['xs'] = [*7* *9*];
    var.immut.i64 ['mine'] = [call 'count'[*1*]];
    var.immut.i64 ['theirs'] = [call count['xs']];
    print.stdout['mine' 'theirs'];
}
";
    assert!(codes(source).is_empty(), "{}", errors(source));

    // And each says the other kind is the one it is not.
    let missing = errors("START { var.immut.i64 ['x'] = [call 'nowt'[*1*]]; }");
    assert!(missing.contains("a function the writer declared"), "{missing}");
    let unknown = errors("START { var.immut.i64 ['x'] = [call nowt[*1*]]; }");
    assert!(unknown.contains("something the language provides"), "{unknown}");
}

#[test]
fn a_function_and_a_variable_may_share_a_name() {
    // Nothing at a use site is ambiguous any more: `call 'total'[*1*]` is the function
    // and `'total'[*1*]` is the array, and both say which on the line where they are.
    let source = "\
fn.file.i64 ['total'] [immut.i64 'n'] { give ['n' x *100*]; }
START {
    var.immut.arr.i64 (2) ['total'] = [*7* *9*];
    var.immut.i64 ['called'] = [call 'total'[*3*]];
    var.immut.i64 ['indexed'] = ['total'[*2*]];
    print.stdout['called' 'indexed'];
}
";
    assert!(codes(source).is_empty(), "{}", errors(source));
}

#[test]
fn a_function_is_named_with_whatever_any_other_name_holds() {
    // There was a rule here that a function's name had to be writable as a bare word,
    // because a call was one. A call says `call` and wears marks now, so the rule went.
    let source = "\
fn.file.i64 ['\u{1f525}'] [immut.i64 'n'] { give ['n' x *100*]; }
fn.file.nothing ['a name with spaces'] [immut.i64 'n'] { print.stdout['n']; }
fn.file.i64 ['\u{e17}\u{e27}\u{e35}\u{e04}\u{e39}\u{e13}'] [immut.i64 'n'] { give ['n' x *2*]; }
START {
    var.immut.i64 ['a'] = [call '\u{1f525}'[*3*]];
    var.immut.i64 ['b'] = [call '\u{e17}\u{e27}\u{e35}\u{e04}\u{e39}\u{e13}'[*21*]];
    call 'a name with spaces'[*7*];
    print.stdout['a' 'b'];
}
";
    assert!(codes(source).is_empty(), "{}", errors(source));
}

#[test]
fn an_index_is_read_as_a_number_whatever_is_around_it() {
    // A value with an operator in it is read under the chain's type, which is how
    // `*200*` is a `u8` in one line and a mistake in the next. An index is not part of
    // that: it is counted, so it is a whole number wherever it is written, and this
    // used to refuse its own `*1*` under any chain that was not a whole number itself.
    for chain in ["b64", "b32", "b16", "d64", "e"] {
        let source = format!(
            "START {{ var.immut.arr.{chain} (2) ['xs'] = [*1* *2*]; \
             var.immut.{chain} ['sum'] = ['xs'[*1*] + 'xs'[*2*]]; print.stdout['sum']; }}"
        );
        assert!(codes(&source).is_empty(), "{}", errors(&source));
    }

    // And a real mistake in an index still says so.
    let wrong = errors("START { var.immut.arr.i64 (2) ['xs'] = [*1* *2*]; var.immut.i64 ['n'] = ['xs'[*a*] + *1*]; }");
    assert!(wrong.contains("Error code: E0407"), "{wrong}");
}

#[test]
fn a_constant_that_is_not_an_array_is_not_called_one() {
    // The rule is about what a constant can hold, and it caught scalars too -- while
    // telling them they were arrays.
    let scalar = errors("const.file.d64 ['RATE'] = [*0.0825*]; START { }");
    assert!(scalar.contains("a constant `d64` is not built yet"), "{scalar}");
    assert!(scalar.contains("a `d64` is a handle"), "{scalar}");
    assert!(!scalar.contains("array"), "{scalar}");

    let array = errors("const.file.arr.e (2) ['R'] = [*1/2* *1/3*]; START { }");
    assert!(array.contains("a constant array of `e` is not built yet"), "{array}");
    assert!(array.contains("an `e` is a handle"), "{array}");
}

#[test]
fn reading_text_says_which_type_it_is_about() {
    // Text says nothing about what it holds, so the type cannot be worked out and has
    // to be asked for. See `notes/checking-comes-first.md`.
    let rendered = errors("START { var.immut.i64 ['n'] = [call as['x']]; }");
    assert!(rendered.contains("`as` says which type it is about."), "{rendered}");
    assert!(rendered.contains("Error code: E0496"), "{rendered}");

    let two = errors("START { var.immut.i64 ['n'] = [call as.i64.b64['x']]; }");
    assert!(two.contains("`as` is about one type, and this names 2."), "{two}");

    let unknown = errors("START { var.immut.i64 ['n'] = [call is.frog['x']]; }");
    assert!(unknown.contains("there is no type called `frog`."), "{unknown}");
    // The list comes from `Ty::NAMES` rather than from a second copy of it, which is
    // the only way it cannot go stale.
    for name in quench_check::Ty::NAMES {
        assert!(unknown.contains(&format!("`{name}`")), "{name} missing:\n{unknown}");
    }
}

#[test]
fn text_is_not_read_out_of_text() {
    let rendered = errors("START { var.immut.str ['s'] = [call as.str['x']]; }");
    assert!(rendered.contains("`as.str` reads text out of text."), "{rendered}");
    assert!(rendered.contains("Error code: E0497"), "{rendered}");
}

#[test]
fn reading_text_reads_one_piece_of_it() {
    let rendered = errors("START { var.immut.i64 ['n'] = [call as.i64['x', 'y']]; }");
    assert!(rendered.contains("`as` reads one piece of text."), "{rendered}");
    assert!(rendered.contains("Error code: E0498"), "{rendered}");
}

#[test]
fn a_chain_belongs_only_to_the_two_that_cannot_work_the_type_out() {
    // Everything else the language provides is told what it is by what it is given.
    let provided = errors("START { var.immut.i64 ['n'] = [call count.i64['x']]; }");
    assert!(provided.contains("`count` carries no chain."), "{provided}");
    assert!(provided.contains("Error code: E0499"), "{provided}");

    // A marked name does carry a chain now -- it is how a module path is written -- but
    // the links of one path all say the same thing about who made it.
    let mine = errors("START { var.immut.i64 ['n'] = [call 'mine'.i64['x']]; }");
    assert!(mine.contains("this path is marked in one place and bare in another."), "{mine}");
    assert!(mine.contains("Error code: E0499"), "{mine}");
}

#[test]
fn is_and_as_are_words_the_language_provides() {
    // Which means they are bare after `call`, and a writer may still have their own
    // `'is'` between marks without a collision.
    assert!(quench_check::PROVIDED.iter().any(|(word, _)| *word == "is"));
    assert!(quench_check::PROVIDED.iter().any(|(word, _)| *word == "as"));
    let both = check(
        "\
fn.file.i64 ['as'] [immut.i64 'n'] { give ['n']; }
START {
    var.immut.i64 ['a'] = [call 'as'[*1*]];
    var.immut.i64 ['b'] = [call as.i64[*2*]];
    print.stdout['a' 'b'];
}
",
    );
    assert!(both.ok());
}

#[test]
fn a_hole_belongs_to_the_function_that_opened_it() {
    let outside = errors("START { var.immut.any ['x'] = [*1*]; }");
    assert!(outside.contains("`any` is a hole, and there is no function here to fill it."), "{outside}");
    assert!(outside.contains("Error code: E0501"), "{outside}");

    // Inside one, it is a type like any other -- which is how a body holds one of
    // whatever it was handed.
    let inside = check(
        "\
fn.file.number ['doubled'] [immut.number 'n'] {
    var.immut.number ['twice'] = ['n' + 'n'];
    give ['twice'];
}
START { print.stdout[call 'doubled'[i64:*2*]]; }
",
    );
    assert!(inside.ok());
}

#[test]
fn a_function_has_one_hole() {
    let two = errors(
        "fn.file.any ['f'] [immut.number 'a'] { give ['a']; } START { print.stdout[str:*x*]; }",
    );
    assert!(two.contains("this function opened `any`, and this says `number`."), "{two}");
    assert!(two.contains("Error code: E0500"), "{two}");

    // And one call fills it once, so two arguments of one hole are two of one type.
    let mixed = errors(
        "\
fn.file.bool ['same'] [immut.any 'a', immut.any 'b'] { give ['a' == 'b']; }
START { print.stdout[call 'same'[i64:*1*, str:*x*]]; }
",
    );
    assert!(mixed.contains("has one hole, and this call gives it two types."), "{mixed}");
    assert!(mixed.contains("Error code: E0506"), "{mixed}");
}

#[test]
fn what_a_hole_may_do_is_what_every_type_filling_it_may_do() {
    // `any` may be a `str`, and a `str` does not order.
    let ordering = errors(
        "fn.file.bool ['f'] [immut.any 'a', immut.any 'b'] { give ['a' < 'b']; } START { print.stdout[str:*x*]; }",
    );
    assert!(ordering.contains("`<` works on numbers, and `any` is not known to be one."), "{ordering}");
    assert!(ordering.contains("Error code: E0502"), "{ordering}");

    // And `number` is every number, so it does not get the two that some numbers refuse.
    for op in ["mod", "^"] {
        let source = format!(
            "fn.file.number ['f'] [immut.number 'a'] {{ give ['a' {op} 'a']; }} START {{ print.stdout[str:*x*]; }}"
        );
        let rendered = errors(&source);
        assert!(rendered.contains("does not work on every number"), "{rendered}");
        assert!(rendered.contains("Error code: E0503"), "{rendered}");
    }
}

#[test]
fn a_number_hole_takes_a_number() {
    let rendered = errors(
        "fn.file.number ['f'] [immut.number 'a'] { give ['a']; } START { print.stdout[call 'f'[str:*x*]]; }",
    );
    assert!(rendered.contains("`'f'` takes a `number`, and this is a `str`."), "{rendered}");
    assert!(rendered.contains("Error code: E0508"), "{rendered}");
}

#[test]
fn a_hole_is_worked_out_from_the_arguments_and_so_has_to_be_in_them() {
    let rendered = errors(
        "fn.file.any ['f'] [immut.i64 'a'] { give [call 'f'[i64:*1*]]; } START { print.stdout[str:*x*]; }",
    );
    assert!(rendered.contains("nothing here says what `'f'`'s `any` is."), "{rendered}");
    assert!(rendered.contains("Error code: E0507"), "{rendered}");
}

#[test]
fn a_written_value_at_a_hole_says_its_own_type() {
    // The ordinary rule, arriving where the chain genuinely cannot say.
    let rendered = errors(
        "fn.file.any ['f'] [immut.any 'a'] { give ['a']; } START { print.stdout[call 'f'[*1*]]; }",
    );
    assert!(rendered.contains("this written value is what says which type the hole is"), "{rendered}");
    assert!(rendered.contains("Error code: E0509"), "{rendered}");
}

#[test]
fn a_pattern_asked_for_endlessly_is_refused_rather_than_waited_for() {
    // A hole handed an array of itself is a wider type every time round, so the list of
    // copies never ends. Rust stops at a depth for the same reason.
    let rendered = errors(
        "\
fn.file.i64 ['deeper'] [immut.any 'x'] {
    var.immut.arr.any (2) ['pair'] = [['x' 'x']];
    give [call 'deeper'[share 'pair']];
}
START { print.stdout[call 'deeper'[i64:*1*]]; }
",
    );
    assert!(rendered.contains("is asked for at more types than this can write out."), "{rendered}");
    assert!(rendered.contains("Error code: E0504"), "{rendered}");
}

#[test]
fn the_hole_words_are_words_the_language_lists() {
    // The guard that caught six stale lists in a day, pointed at this one.
    for word in quench_check::Hole::ALL {
        assert!(
            quench_check::CHAIN_LINKS.contains(word),
            "`{word}` is a hole word and `quench words` has never heard of it"
        );
    }
}

#[test]
fn a_length_left_unsaid_is_one_that_arrived() {
    // `grow` and `any` are both "not a number", and they grant opposite things. An
    // array that grows is one a program makes and fills; one whose length is `any` is
    // one it was handed, and it may not assume the thing it was handed grows.
    let added = errors(
        "fn.file.i64 ['f'] [mut.arr.i64 (any) 'xs'] { add ['xs'] = [*1*]; give [*0*]; } START { print.stdout[str:*x*]; }",
    );
    assert!(added.contains("does not grow."), "{added}");

    let written = errors("START { var.immut.arr.i64 (any) ['xs'] = [[*1* *2*]]; print.stdout['xs']; }");
    assert!(written.contains("is one that arrived, not one written here."), "{written}");
    assert!(written.contains("Error code: E0510"), "{written}");

    // And at any depth, or `[[]]` would make two arrays nothing could ever fill.
    let nested = errors("START { var.mut.arr.arr.i64 (2 any) ['m'] = [[]]; print.stdout['m']; }");
    assert!(nested.contains("Error code: E0510"), "{nested}");

    // Only the first size, the same rule `grow` follows and for the same reason.
    let inner = errors("START { var.immut.arr.i64 (2 any) ['m'] = [[*1* *2*]]; print.stdout['m']; }");
    assert!(inner.contains("only the first size of an allocation can say `any`."), "{inner}");
    assert!(inner.contains("Error code: E0480"), "{inner}");
}

#[test]
fn an_unsaid_length_is_accepted_from_and_not_given_to() {
    // An `arr.i64 (3)` is an `arr.i64 (any)`, because `any` claims to know nothing and
    // a length of three does not contradict that. Nothing goes the other way.
    let taking = check(
        "\
fn.file.i64 ['first of'] [immut.arr.i64 (any) 'xs'] { give ['xs'[*1*]]; }
START {
    var.immut.arr.i64 (3) ['ns'] = [[*1* *2* *3*]];
    var.mut.arr.i64 (grow) ['gs'] = [[*4*]];
    print.stdout[call 'first of'[share 'ns'] call 'first of'[share 'gs']];
}
",
    );
    assert!(taking.ok());

    let giving = errors(
        "\
fn.file.i64 ['fixed'] [immut.arr.i64 (3) 'xs'] { give ['xs'[*1*]]; }
fn.file.i64 ['loose'] [immut.arr.i64 (any) 'ys'] { give [call 'fixed'[share 'ys']]; }
START { print.stdout[str:*x*]; }
",
    );
    assert!(giving.contains("`arr.i64 (any)`, and it is being given to an `arr.i64 (3)`"), "{giving}");
}

#[test]
fn a_module_decides_which_names_reach_which_code() {
    // `module` reaches this module and everything nested inside it, so a child sees its
    // ancestors and the file does not see in.
    let hidden = errors(
        "module.file ['m'] { fn.module.i64 ['hidden'] [] { give [*1*]; } } START { print.stdout[call 'm'.'hidden'[]]; }",
    );
    assert!(hidden.contains("`'m.hidden'` says `module`, and this is written in the top of the file."), "{hidden}");
    assert!(hidden.contains("Error code: E0511"), "{hidden}");

    // And a child seeing its ancestor is the case modules exist for.
    let inward = check(
        "\
module.file ['maths'] {
    fn.module.i64 ['reduce'] [] { give [*1*]; }
    module.file ['trig'] {
        fn.export.i64 ['sin'] [] { give [call 'reduce'[]]; }
    }
}
START { print.stdout[call 'maths'.'trig'.'sin'[]]; }
",
    );
    assert!(inward.ok(), "a child must see its ancestors");

    // `parent` reaches the module around the declaring one, and no further.
    let up = errors(
        "module.file ['a'] { module.file ['b'] { fn.parent.i64 ['up'] [] { give [*1*]; } } } START { print.stdout[call 'a'.'b'.'up'[]]; }",
    );
    assert!(up.contains("says `parent`"), "{up}");
}

#[test]
fn the_two_narrow_words_want_a_boundary_that_is_there() {
    // Refused where they are written, the way `any` outside a function is, rather than
    // quietly widened into the next rung up.
    let no_module = errors("fn.module.i64 ['x'] [] { give [*1*]; } START { print.stdout[call 'x'[]]; }");
    assert!(no_module.contains("`module` says a boundary that is not here."), "{no_module}");
    assert!(no_module.contains("Error code: E0512"), "{no_module}");

    // At one level deep the module around this one *is* the file, and `parent` would
    // then be a second spelling of `file`.
    let no_parent = errors("module.file ['m'] { fn.parent.i64 ['x'] [] { give [*1*]; } } START { print.stdout[call 'm'.'x'[]]; }");
    assert!(no_parent.contains("`parent` says a boundary that is not here."), "{no_parent}");
}

#[test]
fn a_module_holds_declarations_and_a_program_begins_once() {
    let start = errors("module.file ['m'] { START { print.stdout[str:*x*]; } }");
    assert!(start.contains("`START` is not something a module holds."), "{start}");
    assert!(start.contains("Error code: E0103"), "{start}");
}

#[test]
fn every_link_of_a_path_says_the_same_thing_about_who_made_it() {
    let mixed = errors(
        "module.file ['m'] { fn.file.i64 ['x'] [] { give [*1*]; } } START { print.stdout[call 'm'.x[]]; }",
    );
    assert!(mixed.contains("this path is marked in one place and bare in another."), "{mixed}");
    assert!(mixed.contains("Error code: E0499"), "{mixed}");
}

#[test]
fn the_ladder_is_five_and_the_list_is_not_a_second_copy() {
    // The guard that caught six stale lists in a day, pointed at the one that just grew.
    assert_eq!(quench_check::Visibility::ALL, ["module", "parent", "file", "program", "export"]);
    let rendered = errors("fn.i64 ['x'] [] { give [*1*]; } START { print.stdout[call 'x'[]]; }");
    for word in quench_check::Visibility::ALL {
        assert!(rendered.contains(&format!("`{word}`")), "`{word}` missing from the message:\n{rendered}");
    }
}

#[test]
fn a_module_says_who_may_see_it_like_everything_else_at_the_top() {
    // It was the only top-level thing that did not, which made it the one exception to
    // a rule the language states outright.
    let silent = errors("module ['m'] { fn.file.i64 ['x'] [] { give [*1*]; } } START { print.stdout[call 'm'.'x'[]]; }");
    assert!(silent.contains("does not say who can see it."), "{silent}");

    // And saying it means something: a module may be an implementation detail rather
    // than part of what the module around it offers.
    let hidden = errors(
        "\
module.file ['maths'] {
    module.module ['trig'] { fn.export.i64 ['x'] [] { give [*1*]; } }
}
START { print.stdout[call 'maths'.'trig'.'x'[]]; }
",
    );
    assert!(hidden.contains("the module `'maths.trig'` says `module`"), "{hidden}");
    assert!(hidden.contains("Error code: E0513"), "{hidden}");

    // What is inside may say something wider and still not reach further than the
    // module around it does -- `'x'` above says `export` and is unreachable anyway.
    let inside = check(
        "\
module.file ['maths'] {
    module.module ['trig'] { fn.export.i64 ['x'] [] { give [*1*]; } }
    fn.export.i64 ['sin'] [] { give [call 'trig'.'x'[]]; }
}
START { print.stdout[call 'maths'.'sin'[]]; }
",
    );
    assert!(inside.ok(), "the module around it may still reach in");
}

#[test]
fn a_constant_is_reached_through_a_path_too() {
    let out = check(
        "\
module.file ['text'] { const.export.str ['MARK'] = [*!*]; }
START {
    var.immut.str ['m'] = ['text'.'MARK'];
    print.stdout['m'];
}
",
    );
    assert!(out.ok(), "{}", errors("module.file ['text'] { const.export.str ['MARK'] = [*!*]; } START { var.immut.str ['m'] = ['text'.'MARK']; print.stdout['m']; }"));

    // The same ladder, and the same walk outward to find it.
    let hidden = errors(
        "module.file ['t'] { const.module.i64 ['S'] = [*7*]; } START { print.stdout['t'.'S']; }",
    );
    assert!(hidden.contains("`'t.S'` says `module`"), "{hidden}");

    let missing = errors("START { print.stdout['a'.'B']; }");
    assert!(missing.contains("there is nothing called `'a.B'`."), "{missing}");

    // A path is uniformly marked, in a value as much as at a call.
    let mixed = errors("module.file ['t'] { const.export.i64 ['S'] = [*7*]; } START { print.stdout['t'.S]; }");
    assert!(mixed.contains("this path is marked in one place and bare in another."), "{mixed}");
}

/// A program of several files, laid end to end the way the compiler lays them out.
fn across(files: &[(&str, &str)]) -> (String, Vec<quench_check::Part>) {
    let mut whole = String::new();
    let mut parts = Vec::new();
    for (name, text) in files {
        parts.push(quench_check::Part { at: whole.len(), name: (*name).to_string() });
        whole.push_str(text);
        whole.push('\n');
    }
    (whole, parts)
}

fn spanning(files: &[(&str, &str)]) -> Vec<String> {
    let (whole, parts) = across(files);
    quench_check::check_across(&whole, &parts).errors.iter().map(|e| e.message.clone()).collect()
}

fn crosses(files: &[(&str, &str)]) -> bool {
    let (whole, parts) = across(files);
    quench_check::check_across(&whole, &parts).ok()
}

#[test]
fn a_name_crosses_a_file_when_the_file_says_it_uses_it() {
    assert!(crosses(&[
        ("maths", "fn.export.b64 ['sin'] [immut.b64 'x'] { give ['x']; }"),
        ("main", "import ['maths'];\nSTART { print.stdout[call 'maths'.'sin'[*1.0*]]; }"),
    ]));

    // Without the import it does not, and the message says the fix rather than
    // pretending the name does not exist.
    let said = spanning(&[
        ("maths", "fn.export.b64 ['sin'] [immut.b64 'x'] { give ['x']; }"),
        ("main", "START { print.stdout[call 'maths'.'sin'[*1.0*]]; }"),
    ]);
    assert!(
        said.iter().any(|m| m.contains("`'maths'` is a file of this program, and this file does not import it.")),
        "{said:?}"
    );
}

#[test]
fn file_finally_means_something() {
    // Until a program could be more than one file there was nowhere for this to be
    // false. `program` and `export` still cannot be told apart -- that wants a second
    // *program* using this one as a library.
    let said = spanning(&[
        ("maths", "fn.file.b64 ['hidden'] [immut.b64 'x'] { give ['x']; }"),
        ("main", "import ['maths'];\nSTART { print.stdout[call 'maths'.'hidden'[*1.0*]]; }"),
    ]);
    assert!(said.iter().any(|m| m.contains("says `file`")), "{said:?}");

    for word in ["program", "export"] {
        assert!(
            crosses(&[
                ("maths", &format!("fn.{word}.b64 ['x'] [immut.b64 'n'] {{ give ['n']; }}")),
                ("main", "import ['maths'];\nSTART { print.stdout[call 'maths'.'x'[*1.0*]]; }"),
            ]),
            "`{word}` should cross a file"
        );
    }
}

#[test]
fn an_import_names_a_file_of_this_program_once_and_is_not_this_file() {
    let missing = spanning(&[("main", "import ['nope'];\nSTART { print.stdout[str:*x*]; }")]);
    assert!(missing.iter().any(|m| m.contains("`'nope'` is not a file of this program.")), "{missing:?}");

    let itself = spanning(&[("main", "import ['main'];\nSTART { print.stdout[str:*x*]; }")]);
    assert!(itself.iter().any(|m| m.contains("`'main'` is this file.")), "{itself:?}");

    let twice = spanning(&[
        ("maths", "fn.export.i64 ['x'] [] { give [*1*]; }"),
        ("main", "import ['maths'];\nimport ['maths'];\nSTART { print.stdout[str:*x*]; }"),
    ]);
    assert!(twice.iter().any(|m| m.contains("`'maths'` is imported twice.")), "{twice:?}");

    let inside = spanning(&[
        ("maths", "fn.export.i64 ['x'] [] { give [*1*]; }"),
        ("main", "module.file ['m'] { import ['maths']; }\nSTART { print.stdout[str:*x*]; }"),
    ]);
    assert!(
        inside.iter().any(|m| m.contains("an `import` belongs to a file, not to a module inside one.")),
        "{inside:?}"
    );
}

#[test]
fn two_files_may_each_hold_a_name() {
    assert!(crosses(&[
        ("a", "fn.export.i64 ['size'] [] { give [*1*]; }"),
        ("b", "fn.export.i64 ['size'] [] { give [*2*]; }"),
        ("main", "import ['a'];\nimport ['b'];\nSTART { print.stdout[call 'a'.'size'[] call 'b'.'size'[]]; }"),
    ]));
}
