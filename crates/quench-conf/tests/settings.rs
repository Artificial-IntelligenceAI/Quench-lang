//! Reading the file that decides how every other file is built.

use quench_conf::{read, Division, Engine, Settings};
use quench_diag::SourceFile;

fn errors(text: &str) -> String {
    let errors = read(text).errors;
    quench_diag::report(&SourceFile::new("QNL-Config.toml", text), &errors)
}

#[test]
fn a_file_that_says_nothing_leaves_everything_alone() {
    let quench_conf::Config { settings, errors, .. } = read("");
    assert!(errors.is_empty());
    assert_eq!(settings, Settings::default());
    assert_eq!(settings.division, Division::Truncated, "what every processor does");
}

#[test]
fn it_reads_what_it_is_given() {
    let quench_conf::Config { settings, errors, .. } =
        read("[defaults]\ndivision = \"floored\"\n\n[run]\nengine = \"interpreter\"\n");
    assert!(errors.is_empty());
    assert_eq!(settings.division, Division::Floored);
    assert_eq!(settings.engine, Engine::Interpreter);
}

#[test]
fn comments_and_blank_lines_are_not_settings() {
    let quench_conf::Config { settings, errors: errs, .. } =
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
    assert!(out.contains("`[defaults]` holds"), "{out}");
    assert!(out.contains("`characters`") && out.contains("`min-max`"), "{out}");
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
    let errs = read("[nope]\n[defaults]\ndivision = \"sideways\"\nwobble = \"1\"\n").errors;
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
    let quench_conf::Config { settings, errors, .. } = quench_conf::read("[defaults]\nlogic = \"asks-both\"\n");
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(settings.logic, quench_conf::Logic::AsksBoth);

    let quench_conf::Config { settings, errors, .. } = quench_conf::read("[defaults]\nlogic = \"stops-early\"\n");
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(settings.logic, quench_conf::Logic::StopsEarly);

    // Stopping early is the default, because it is what makes a guard a guard.
    assert_eq!(quench_conf::Settings::default().logic, quench_conf::Logic::StopsEarly);

    let errors = quench_conf::read("[defaults]\nlogic = \"lazy\"\n").errors;
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, "E0705");
}

#[test]
fn no_number_says_what_a_float_does_when_it_has_none() {
    let quench_conf::Config { settings, errors, .. } = quench_conf::read("[defaults]\nno-number = \"stops\"\n");
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(settings.no_number, quench_conf::NoNumber::Stops);

    // Carrying on is the default, because `b64` *is* IEEE 754 binary64 and `infinity`
    // and `not-a-number` are values of that type rather than accidents of it.
    assert_eq!(quench_conf::Settings::default().no_number, quench_conf::NoNumber::CarriesOn);

    let errors = quench_conf::read("[defaults]\nno-number = \"ieee\"\n").errors;
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, "E0705");
}

#[test]
fn every_setting_the_reader_knows_is_a_setting_it_lists() {
    // The list a reader is shown and the keys the reader accepts were two separate
    // things, and they drifted: `characters` was added to one and not the other, and the
    // test that checked the sentence passed because it was checking the same stale
    // words. So the sentence is generated from the list, and this holds the list against
    // what the parser actually does.
    for (section, key) in quench_conf::KEYS {
        let text = format!("[{section}]\n{key} = \"an-answer-nobody-offers\"\n");
        let errors = quench_conf::read(&text).errors;
        let codes: Vec<&str> = errors.iter().map(|e| e.code.as_str()).collect();
        // The claim is that the reader *knows* the key -- so it complains about the
        // value rather than about the key. Which code it uses for a bad value is the
        // value's business: one of a fixed set is E0705, and `files` takes a list and
        // says so its own way.
        assert!(
            !codes.contains(&"E0704"),
            "`{key}` in `[{section}]` is listed and the reader does not know it: {codes:?}"
        );
        assert_eq!(
            codes.len(),
            1,
            "`{key}` in `[{section}]` should complain once about the value, and said {codes:?}"
        );
    }

    // And the other way: a key that is not in the list is refused, and the refusal names
    // the ones that are.
    let errors = quench_conf::read("[defaults]\nwobble = \"yes\"\n").errors;
    let rendered = format!("{:?}", errors);
    assert!(rendered.contains("E0704"), "{rendered}");
    for (section, key) in quench_conf::KEYS {
        if *section == "defaults" {
            assert!(rendered.contains(key), "the refusal did not mention `{key}`: {rendered}");
        }
    }
}

#[test]
fn a_program_says_which_files_it_is_made_of() {
    let read = quench_conf::read(
        "[program.files]\nmain = \"main.qnl\"\nmaths = \"src/maths.qnl\"\n",
    );
    assert!(read.errors.is_empty(), "{:#?}", read.errors);
    assert_eq!(
        read.files,
        [
            ("main".to_string(), "main.qnl".to_string()),
            ("maths".to_string(), "src/maths.qnl".to_string()),
        ]
    );

    // The name is chosen, not taken from the filename -- which is the whole point of
    // it, since a filename is not an interface.
    let renamed = quench_conf::read("[program.files]\nmaths = \"src/arithmetic-v2.qnl\"\n");
    assert_eq!(renamed.files, [("maths".to_string(), "src/arithmetic-v2.qnl".to_string())]);

    // And it holds whatever a name holds, because it is written between marks at every
    // `import`.
    let odd = quench_conf::read("[program.files]\n\"a name with spaces\" = \"odd.qnl\"\n");
    assert!(odd.errors.is_empty(), "{:#?}", odd.errors);
    assert_eq!(odd.files, [("a name with spaces".to_string(), "odd.qnl".to_string())]);

    // Saying nothing means the program is the one file it was given.
    assert!(quench_conf::read("").files.is_empty());

    for wrong in ["maths", "maths =", "= \"x.qnl\"", "maths = x.qnl", "\"unclosed = \"x.qnl\""] {
        let text = format!("[program.files]\n{wrong}\n");
        let codes: Vec<String> =
            quench_conf::read(&text).errors.iter().map(|e| e.code.clone()).collect();
        assert_eq!(codes, ["E0706"], "`{wrong}` should be refused: {codes:?}");
    }

    // One name is one file, and one file is one name.
    for twice in [
        "[program.files]\nmaths = \"a.qnl\"\nmaths = \"b.qnl\"\n",
        "[program.files]\none = \"a.qnl\"\ntwo = \"a.qnl\"\n",
    ] {
        let codes: Vec<String> =
            quench_conf::read(twice).errors.iter().map(|e| e.code.clone()).collect();
        assert_eq!(codes, ["E0707"], "{twice}");
    }

    // And the section is a section now, which the sentence naming them has to know.
    let unknown = quench_conf::read("[nope]\n").errors;
    let rendered = quench_diag::report(&SourceFile::new("QNL-Config.toml", "[nope]\n"), &unknown);
    assert!(rendered.contains("`[program.files]`"), "{rendered}");
}

#[test]
fn the_files_section_is_not_a_settings_section() {
    // Every other section has a fixed set of keys, and `KEYS` is held against the
    // parser so neither can drift. `[program.files]` is the exception on purpose: its
    // keys are the writer's names, so it is not in `KEYS` and must not be, or the guard
    // above would demand a bad-value error from a key that has no fixed values.
    assert!(
        !quench_conf::KEYS.iter().any(|(section, _)| *section == "program.files"),
        "`[program.files]` holds names rather than settings, so it is not in `KEYS`"
    );
    // And any key at all is taken there, which is what that means.
    for name in ["maths", "a", "🔥"] {
        let text = format!("[program.files]\n\"{name}\" = \"x.qnl\"\n");
        let read = quench_conf::read(&text);
        assert!(read.errors.is_empty(), "`{name}` should be a name: {:#?}", read.errors);
        assert_eq!(read.files, [(name.to_string(), "x.qnl".to_string())]);
    }
}
