//! Whole programs, and the errors they get when they are not.

use quench_diag::SourceFile;
use quench_parse::{parse, Piece, Stmt};

fn report(source: &str) -> String {
    let out = parse(source);
    quench_diag::report(&SourceFile::new("src/main.qnl", source), &out.errors)
}

#[test]
fn hello_world() {
    let source = "START {\nprint.stdout[str:*Hello, World!*];\n}\n";
    let out = parse(source);
    assert!(out.ok(), "{}", report(source));

    let start = out.program.start.expect("a program starts somewhere");
    assert_eq!(&source[start.word.start..start.word.end], "START");
    assert_eq!(start.body.len(), 1);

    let Stmt::Print(print) = &start.body[0] else { panic!("a print") };
    assert_eq!(print.pieces.len(), 1);
    let Piece::Written { ty: Some(ty), mark } = &print.pieces[0] else { panic!("typed") };
    assert_eq!(&source[ty.start..ty.end], "str");
    assert_eq!(&source[mark.start..mark.end], "*Hello, World!*");
}

#[test]
fn a_declaration_and_a_print() {
    let source = "\
START {
var.immut.str ['greeting'] = [*Hello*];
print.stdout['greeting' \\n];
}
";
    let out = parse(source);
    assert!(out.ok(), "{}", report(source));
    let body = out.program.start.unwrap().body;
    assert_eq!(body.len(), 2);
    assert!(matches!(body[0], Stmt::Var(_)));
    assert!(matches!(body[1], Stmt::Print(_)));
}

#[test]
fn several_names_and_several_values() {
    let source = "START {\nvar.immut.str ['s', 'ss'] = [*line one* \\n *line two*, *idk* \\n *Claude*];\n}\n";
    let out = parse(source);
    assert!(out.ok(), "{}", report(source));
    let Stmt::Var(var) = &out.program.start.as_ref().unwrap().body[0] else { panic!() };
    assert_eq!(var.names.len(), 2);
    assert_eq!(var.values.len(), 2);
    assert_eq!(var.values[0].terms.len(), 3, "text, escape, text");
    assert_eq!(var.values[1].terms.len(), 3);
    assert!(!var.values[0].has_operators(), "nothing between them but juxtaposition");
    assert_eq!(var.chain.len(), 3, "`var`, `immut` and `str`");
}

#[test]
fn the_chain_is_kept_link_by_link() {
    let source = "START {\nvar.mut.b16 ['x'] = [*1000*];\n}\n";
    let out = parse(source);
    assert!(out.ok(), "{}", report(source));
    let Stmt::Var(var) = &out.program.start.as_ref().unwrap().body[0] else { panic!() };
    let links: Vec<&str> = var.chain.iter().map(|s| &source[s.start..s.end]).collect();
    assert_eq!(links, ["var", "mut", "b16"], "so a diagnostic can point at one of them");
}

#[test]
fn counts_that_do_not_match_point_at_both_lists() {
    let source = "START {\nvar.immut.str ['a', 'b'] = [*one*];\n}\n";
    let out = parse(source);
    assert_eq!(out.errors.len(), 1, "{:#?}", out.errors);
    let rendered = report(source);
    assert!(rendered.contains("two names declared, and one value given."), "{rendered}");
    assert!(rendered.contains("Error code: E0105"), "{rendered}");
    // The point of the error: both lists are shown, not just the one that ended early.
    assert!(rendered.contains("two names"), "{rendered}");
    assert!(rendered.contains("one value"), "{rendered}");
    assert!(rendered.contains("a missing comma joins two of them into one"), "{rendered}");
}

#[test]
fn a_bare_written_value_in_a_print_is_told_what_is_missing() {
    let source = "START {\nprint.stdout[*Hello*];\n}\n";
    let rendered = report(source);
    assert!(rendered.contains("does not say what it is"), "{rendered}");
    assert!(rendered.contains("`str:*Hello*` if it is text"), "{rendered}");
}

#[test]
fn a_typed_value_in_a_declaration_is_saying_it_twice() {
    let source = "START {\nvar.immut.str ['a'] = [str:*Hello*];\n}\n";
    let rendered = report(source);
    assert!(rendered.contains("says its type twice"), "{rendered}");
    assert!(rendered.contains("Error code: E0107"), "{rendered}");
}

#[test]
fn a_missing_semicolon_is_reported_where_it_should_have_been() {
    let source = "START {\nprint.stdout[str:*a*]\n}\n";
    let rendered = report(source);
    assert!(rendered.contains("a statement wants `;` here"), "{rendered}");
}

#[test]
fn four_mistakes_report_as_four() {
    // Each line is wrong on its own, and the semicolon is what lets the parser believe
    // it has found the start of the next one.
    let source = "\
START {
var.immut.str ['a', 'b'] = [*one*];
print.stdout[*bare*];
wobble ['x'];
var.immut.str ['c'] = [str:*twice*];
}
";
    let out = parse(source);
    let codes: Vec<&str> = out.errors.iter().map(|e| e.code.as_str()).collect();
    // `wobble ['x'];` is not among them: a bare word before a bracket is a call, so the
    // parser understands it perfectly and the checker is what says nothing is called
    // that -- which is a better sentence than the parser could have written.
    assert_eq!(codes, ["E0105", "E0106", "E0107"], "{:#?}", out.errors);
}

#[test]
fn one_mistake_does_not_become_three() {
    // Recovery that invents errors is worse than stopping, since a reader cannot tell
    // which of them was the real one.
    let source = "START {\nprint.stdout[str:*a* ;\nprint.stdout[str:*b*];\n}\n";
    let out = parse(source);
    assert!(out.errors.len() <= 2, "{:#?}", out.errors);
}

#[test]
fn a_file_with_nothing_in_it_is_not_an_error_yet() {
    // It has no `START`, which is a thing to complain about later, when there is a
    // compiler to complain on behalf of. The parser's job is the shape.
    let out = parse("");
    assert!(out.ok(), "{:#?}", out.errors);
    assert!(out.program.start.is_none());
}

#[test]
fn a_variable_at_the_top_of_a_file_is_pointed_at_const() {
    // The rule is constants outside and variables inside, and this is the one place a
    // reader can meet it. So the error says which of the two they wanted.
    let source = "var.immut.str ['a'] = [*x*];\nSTART {\nprint.stdout[str:*a*];\n}\n";
    let rendered = report(source);
    assert!(rendered.contains("a variable cannot be at the top of a file."), "{rendered}");
    assert!(rendered.contains("constants live outside a function and variables live inside one"), "{rendered}");
    assert!(rendered.contains("`const.<visibility>.<type>`"), "{rendered}");
}

#[test]
fn the_worked_error_renders_whole() {
    // Both labels are on one line, so the line is shown once with the carets stacked --
    // saying it twice was what the renderer used to do, and reads as a stutter.
    let source = "START {\nvar.immut.str ['a', 'b'] = [*one*];\n}\n";
    let expected = "\
Hello, I think there may be thing(s) wrong with your code. I'm sorry, if I'm wrong.

file: src/main.qnl, line: 2, column: 29 (src/main.qnl:2:29)

two names declared, and one value given.

  2 | var.immut.str ['a', 'b'] = [*one*];
    |                ~~~~~~~~ two names
    |                             ^^^^^ one value

Error code: E0105
Rule(s) broken: a declaration gives one value for each name, in the same order
Tip(s): a value runs until a comma, so a missing comma joins two of them into one.
Suggested fix(s): add the missing value, or remove the name

1 error.
";
    assert_eq!(report(source), expected, "\n--- got ---\n{}", report(source));
}

#[test]
fn a_block_that_is_never_closed_points_at_the_brace() {
    // Not at the end of the file. That is where it was noticed, not where it went
    // wrong, and in a long file the difference is the whole message.
    let source = "START {\nprint.stdout[str:*a*];\n";
    let out = parse(source);
    assert_eq!(out.errors.len(), 1, "{:#?}", out.errors);
    assert_eq!(out.errors[0].code, "E0109");

    let rendered = report(source);
    assert!(rendered.contains("line: 1"), "points at line 1, not the end: {rendered}");
    assert!(rendered.contains("this `{` has no partner"), "{rendered}");
    // And what it did parse is kept, so the statement inside is not lost as well.
    assert_eq!(out.program.start.unwrap().body.len(), 1);
}

#[test]
fn a_block_ends_where_its_brace_does() {
    // The closing brace is what lets a file hold something after `START`, which is
    // the whole reason it is there.
    let source = "START {\nprint.stdout[str:*a*];\n}\nwobble\n";
    let out = parse(source);
    let codes: Vec<&str> = out.errors.iter().map(|e| e.code.as_str()).collect();
    assert_eq!(codes, ["E0102"], "the block closed, and `wobble` is a separate complaint");
    assert_eq!(out.program.start.unwrap().body.len(), 1);
}

#[test]
fn a_mistake_inside_a_block_does_not_eat_the_brace() {
    let source = "START {\nwobble = *3*;\n}\n";
    let out = parse(source);
    assert_eq!(out.errors.len(), 1, "{:#?}", out.errors);
    assert_eq!(out.errors[0].code, "E0104", "just the one, and the block still closed");
}

#[test]
fn a_counting_loop_keeps_its_chain_and_its_bounds() {
    let source = "START {\nloop.temp.range.i64 ['i'] = [*1*, *5*] {\nprint.stdout['i' \\n];\n}\n}\n";
    let out = parse(source);
    assert!(out.ok(), "{}", report(source));
    let Stmt::Loop(repeat) = &out.program.start.as_ref().unwrap().body[0] else { panic!() };
    let links: Vec<&str> = repeat.chain.iter().map(|s| &source[s.start..s.end]).collect();
    assert_eq!(links, ["temp", "range", "i64"], "so a diagnostic can point at one of them");
    let quench_parse::LoopKind::Range { name, from, to } = &repeat.kind else { panic!() };
    assert_eq!(&source[name.start..name.end], "'i'");
    assert_eq!(from.terms.len(), 1);
    assert_eq!(to.terms.len(), 1);
    assert_eq!(repeat.body.len(), 1);
}

#[test]
fn a_while_loop_wears_no_brackets_around_its_question() {
    // The same shape as `if`, for the same reason: `[ ]` holds a list everywhere else.
    let source = "START {\nloop.while 'd' > *0* {\nbreak;\n}\n}\n";
    let out = parse(source);
    assert!(out.ok(), "{}", report(source));
    let Stmt::Loop(repeat) = &out.program.start.as_ref().unwrap().body[0] else { panic!() };
    let quench_parse::LoopKind::While(condition) = &repeat.kind else { panic!() };
    assert_eq!(condition.terms.len(), 2, "`'d'` and `*0*`");
    assert!(matches!(repeat.body[0], Stmt::Break(_)));
}

#[test]
fn a_bare_word_before_a_bracket_is_a_call() {
    // And a quoted one before it is an index. Names being quoted is what settles this.
    let source = "START {\nvar.immut.i64 ['n'] = [count['xs']];\n}\n";
    let out = parse(source);
    assert!(out.ok(), "{}", report(source));
    let Stmt::Var(var) = &out.program.start.as_ref().unwrap().body[0] else { panic!() };
    let quench_parse::Term::Call(call) = &var.values[0].terms[0] else { panic!() };
    assert_eq!(&source[call.name.start..call.name.end], "count");
    assert_eq!(call.args.len(), 1);
}

#[test]
fn a_function_keeps_its_chain_its_parameters_and_its_body() {
    let source = "fn.export.i64 ['add'] [immut.i64 'a', immut.i64 'b'] {\ngive ['a' + 'b'];\n}\n";
    let out = parse(source);
    assert!(out.ok(), "{}", report(source));
    let quench_parse::Item::Func(func) = &out.program.items[0] else { panic!() };
    let links: Vec<&str> = func.chain.iter().map(|s| &source[s.start..s.end]).collect();
    assert_eq!(links, ["fn", "export", "i64"], "so a diagnostic can point at one of them");
    assert_eq!(&source[func.name.start..func.name.end], "'add'");
    assert_eq!(func.params.len(), 2);
    let first: Vec<&str> = func.params[0].chain.iter().map(|s| &source[s.start..s.end]).collect();
    assert_eq!(first, ["immut", "i64"], "a declaration's chain with `var` taken off");
    assert!(matches!(func.body[0], Stmt::Give(_)));
}

#[test]
fn a_function_that_takes_nothing_still_writes_the_brackets() {
    let source = "fn.file.nothing ['tick'] [] {\nprint.stdout[\\n];\n}\n";
    let out = parse(source);
    assert!(out.ok(), "{}", report(source));
    let quench_parse::Item::Func(func) = &out.program.items[0] else { panic!() };
    assert!(func.params.is_empty(), "`[]` says it out loud rather than by omission");
}

#[test]
fn a_constant_is_a_declaration_written_somewhere_else() {
    // Same syntax, same code, same errors -- only the keyword and the place differ.
    let source = "const.export.i64 ['LIMIT'] = [*100*];\n";
    let out = parse(source);
    assert!(out.ok(), "{}", report(source));
    let quench_parse::Item::Const(declaration) = &out.program.items[0] else { panic!() };
    let links: Vec<&str> = declaration.chain.iter().map(|s| &source[s.start..s.end]).collect();
    assert_eq!(links, ["const", "export", "i64"]);
    assert_eq!(declaration.names.len(), 1);
}

#[test]
fn a_call_separates_its_arguments_with_commas() {
    // Juxtaposition builds one value out of pieces, which is why it cannot also
    // separate two of them.
    let source = "START {\nprint.stdout[add[*1* + *2*, *3*]];\n}\n";
    let out = parse(source);
    assert!(out.ok(), "{}", report(source));
    let Stmt::Print(print) = &out.program.start.as_ref().unwrap().body[0] else { panic!() };
    let quench_parse::Piece::Call(call) = &print.pieces[0] else { panic!() };
    assert_eq!(call.args.len(), 2);
    assert_eq!(call.args[0].terms.len(), 2, "`*1*` and `*2*`");
    assert!(call.args[0].has_operators());
}
