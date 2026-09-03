//! Reading the file that decides how every other file is built.

use quench_conf::{read, Division, Engine, Settings};
use quench_diag::SourceFile;

fn errors(text: &str) -> String {
    let (_, errors) = read(text);
    quench_diag::report(&SourceFile::new("QNL-Config.toml", text), &errors)
}

#[test]
fn a_file_that_says_nothing_leaves_everything_alone() {
    let (settings, errors) = read("");
    assert!(errors.is_empty());
    assert_eq!(settings, Settings::default());
    assert_eq!(settings.division, Division::Truncated, "what every processor does");
}

#[test]
fn it_reads_what_it_is_given() {
    let (settings, errors) =
        read("[defaults]\ndivision = \"floored\"\n\n[run]\nengine = \"interpreter\"\n");
    assert!(errors.is_empty());
    assert_eq!(settings.division, Division::Floored);
    assert_eq!(settings.engine, Engine::Interpreter);
}

#[test]
fn comments_and_blank_lines_are_not_settings() {
    let (settings, errs) =
        read("# how this project divides\n[defaults]\n\ndivision = \"floored\"  # toward -inf\n");
    assert!(errs.is_empty(), "{errs:#?}");
    assert_eq!(settings.division, Division::Floored);
}

#[test]
fn a_value_it_does_not_know_says_what_the_values_are() {
    let out = errors("[defaults]\ndivision = \"euclidean\"\n");
    assert!(out.contains("`euclidean` is not something `division` can be."), "{out}");
    assert!(out.contains("`truncated` or `floored`"), "{out}");
    assert!(out.contains("Error code: E0705"), "{out}");
}

#[test]
fn a_setting_it_does_not_know_is_refused_rather_than_ignored() {
    // Ignoring it would be worse: a project that wrote it meant something by it, and a
    // silent default is the difference between a program that is wrong and one that
    // says so.
    let out = errors("[defaults]\ndivison = \"floored\"\n");
    assert!(out.contains("`divison` is not a setting `[defaults]` has."), "{out}");
    assert!(out.contains("`[defaults]` holds `division`, `logic` and `overflow`."), "{out}");
}

#[test]
fn a_section_it_does_not_know_says_which_exist() {
    let out = errors("[bulid]\nembed-source = \"false\"\n");
    assert!(out.contains("`[bulid]` is not a section this reads."), "{out}");
    assert!(out.contains("`[defaults]`"), "{out}");
}

#[test]
fn a_setting_before_any_section_belongs_to_nothing() {
    let out = errors("division = \"floored\"\n[defaults]\n");
    assert!(out.contains("is before any section"), "{out}");
    assert!(out.contains("what it affects is written down"), "{out}");
}

#[test]
fn a_line_that_is_neither_says_so() {
    let out = errors("[defaults]\njust some words\n");
    assert!(out.contains("neither a section nor a setting"), "{out}");
}

#[test]
fn it_reports_everything_wrong_and_not_the_first_thing() {
    let (_, errs) = read("[nope]\n[defaults]\ndivision = \"sideways\"\nwobble = \"1\"\n");
    let codes: Vec<&str> = errs.iter().map(|e| e.code.as_str()).collect();
    assert_eq!(codes, ["E0701", "E0705", "E0704"], "{errs:#?}");
}

#[test]
fn the_error_points_at_the_line_it_is_about() {
    let text = "[defaults]\ndivision = \"truncated\"\n\n[run]\nengine = \"wobble\"\n";
    let out = errors(text);
    assert!(out.contains("line: 5"), "the fifth line is the wrong one: {out}");
    assert!(out.contains("`dev-jit` or `interpreter`"), "{out}");
}

#[test]
fn logic_says_whether_the_right_side_is_asked() {
    let (settings, errors) = quench_conf::read("[defaults]\nlogic = \"asks-both\"\n");
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(settings.logic, quench_conf::Logic::AsksBoth);

    let (settings, errors) = quench_conf::read("[defaults]\nlogic = \"stops-early\"\n");
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(settings.logic, quench_conf::Logic::StopsEarly);

    // Stopping early is the default, because it is what makes a guard a guard.
    assert_eq!(quench_conf::Settings::default().logic, quench_conf::Logic::StopsEarly);

    let (_, errors) = quench_conf::read("[defaults]\nlogic = \"lazy\"\n");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, "E0705");
}
