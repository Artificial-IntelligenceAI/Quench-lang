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
    assert_eq!(out.locals.len(), 1);
    assert_eq!(out.locals[0].name, "count");
    assert_eq!(out.locals[0].ty, quench_check::Ty::I64);
    assert!(out.locals[0].mutable);
}

#[test]
fn declaring_a_name_twice_is_the_error_this_project_was_pitched_on() {
    let source = "\
START {
    var.str ['name'] = [*Tankun*];
    var.i64 ['name'] = [*1000*];
}
";
    let rendered = errors(source);
    assert!(rendered.contains("`'name'` is declared twice."), "{rendered}");
    assert!(rendered.contains("declared here first, as `str`"), "{rendered}");
    assert!(rendered.contains("and declared again here, as `i64`"), "{rendered}");
    assert!(rendered.contains("Error code: E0201"), "{rendered}");
    // The name is what collided, so the carets are under the name and not the chain.
    assert!(rendered.contains("|              ~~~~~~ declared here first"), "{rendered}");
}

#[test]
fn a_type_that_is_not_built_does_not_hide_a_name_declared_twice() {
    // Two separate mistakes on one line, and both get said. Reporting only the type
    // would leave the reader to discover the collision on their own after fixing it.
    let source = "START { var.str ['x'] = [*a*]; var.b16 ['x'] = [*1*]; }";
    let found = codes(source);
    assert!(found.contains(&"E0405".to_string()), "{}", errors(source));
    assert!(found.contains(&"E0201".to_string()), "{}", errors(source));
}

#[test]
fn errors_come_out_in_the_order_they_appear_in_the_file() {
    // Otherwise a reader jumps around the file to follow their own mistakes.
    let source = "\
START {
    var.b17 ['a'] = [*1*];
    print['nope'];
    var.i64 ['b'] = [*hello*];
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
    let source = "START { var.str ['greeting'] = [*Hello*]; print['greetng' \\n]; }";
    let rendered = errors(source);
    assert!(rendered.contains("`'greetng'` is not declared."), "{rendered}");
    assert!(rendered.contains("did you mean `'greeting'`?"), "{rendered}");
}

#[test]
fn a_name_that_is_nothing_like_anything_is_not_guessed_at() {
    // A suggestion that is not the answer costs the reader a second look, so nothing is
    // offered unless it is within one edit.
    let source = "START { var.str ['greeting'] = [*Hello*]; print['wobble' \\n]; }";
    let rendered = errors(source);
    assert!(rendered.contains("is not declared"), "{rendered}");
    assert!(!rendered.contains("did you mean"), "{rendered}");
    assert!(rendered.contains("declare it above, with `var`"), "{rendered}");
}

#[test]
fn a_type_that_is_meant_to_exist_and_a_type_that_is_not_are_different_errors() {
    // `b16` is a type Quench means to have. `b17` is a typo. A reader deserves to know
    // which of those happened.
    let not_built = errors("START { var.b16 ['x'] = [*1*]; }");
    assert!(not_built.contains("`b16` is not built yet"), "{not_built}");
    assert!(not_built.contains("Error code: E0405"), "{not_built}");

    let nonsense = errors("START { var.b17 ['x'] = [*1*]; }");
    assert!(nonsense.contains("`b17` is not a type"), "{nonsense}");
    assert!(nonsense.contains("Error code: E0402"), "{nonsense}");
}

#[test]
fn nothing_converts_on_its_own() {
    let source = "START { var.i64 ['n'] = [*1*]; var.str ['s'] = ['n']; }";
    let rendered = errors(source);
    assert!(rendered.contains("this is `i64`, and it is being given to a `str`"), "{rendered}");
    assert!(rendered.contains("two types meet only where something says they should"), "{rendered}");
}

#[test]
fn a_written_value_is_read_by_the_type_it_is_given_to() {
    let rendered = errors("START { var.i64 ['n'] = [*hello*]; }");
    assert!(rendered.contains("`hello` is not a whole number"), "{rendered}");
    assert!(rendered.contains("-9223372036854775808"), "{rendered}");

    // And the same characters are perfectly good text.
    assert!(check("START { var.str ['s'] = [*hello*]; }").ok());
    // While these are a fine number and fine text both.
    assert!(check("START { var.i64 ['n'] = [*1000*]; }").ok());
    assert!(check("START { var.str ['s'] = [*1000*]; }").ok());
}

#[test]
fn juxtaposition_builds_text_and_says_so_when_it_cannot() {
    let rendered = errors("START { var.i64 ['n'] = [*1* \\n *2*]; }");
    assert!(rendered.contains("a number is one written value, not several"), "{rendered}");
    assert!(rendered.contains("`str` is the type where a value is a list of pieces"), "{rendered}");
}

#[test]
fn mut_goes_before_the_type() {
    let rendered = errors("START { var.i64.mut ['n'] = [*1*]; }");
    assert!(rendered.contains("`mut` comes before the type"), "{rendered}");
    assert!(rendered.contains("`var.mut.<type>`"), "{rendered}");
}

#[test]
fn a_declaration_always_says_what_it_declares() {
    let rendered = errors("START { var ['n'] = [*1*]; }");
    assert!(rendered.contains("does not say what it is declaring"), "{rendered}");
    assert!(rendered.contains("a written value means nothing without one"), "{rendered}");
}

#[test]
fn a_name_inside_a_longer_value_needs_something_that_is_not_built() {
    // Joining a name to text builds a *new* value, which needs the collector. Copying a
    // whole one does not, and works.
    let rendered = errors("START { var.str ['a'] = [*x*]; var.str ['b'] = ['a' *y*]; }");
    assert!(rendered.contains("cannot be one piece of a longer value yet"), "{rendered}");
    assert!(rendered.contains("needs the collector"), "{rendered}");

    assert!(check("START { var.str ['a'] = [*x*]; var.str ['b'] = ['a']; }").ok());
}

#[test]
fn everything_wrong_is_reported_and_not_the_first_thing() {
    let source = "\
START {
    var.b17 ['a'] = [*1*];
    var.i64 ['b'] = [*hello*];
    print['nope'];
}
";
    assert_eq!(codes(source), ["E0402", "E0407", "E0413"], "{}", errors(source));
}
