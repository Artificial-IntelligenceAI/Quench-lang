//! The `examples/` directory, run rather than looked at.
//!
//! Five of the twelve files in it did not compile. They were written before `call`
//! became mandatory and still said `count['xs']` and `greet[*Tankun*]`, so every one of
//! them had been wrong since that rule landed — and nothing anywhere ran them, so
//! nothing said so. The README points a reader at `examples/hello.qnl` by name.
//!
//! An example is documentation that claims to be executable. This is what makes the
//! claim true.

use std::path::{Path, PathBuf};

fn directory() -> PathBuf {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../..")).join("examples")
}

/// Every `.qnl` in the directory, in a settled order so a failure names the same file
/// twice running.
fn examples() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(directory())
        .expect("the examples are beside the crates")
        .filter_map(|entry| {
            let path = entry.expect("a directory entry").path();
            (path.extension().is_some_and(|end| end == "qnl")).then_some(path)
        })
        .collect();
    found.sort();
    assert!(found.len() > 5, "the examples directory has gone missing");
    found
}

#[test]
fn every_example_checks_out() {
    for path in examples() {
        let source = std::fs::read_to_string(&path).expect("a readable example");
        let out = quench_lower::lower(&source);
        assert!(
            out.ok(),
            "{} does not compile:\n{}",
            path.display(),
            quench_diag::report(
                &quench_diag::SourceFile::new(&path.display().to_string(), &source),
                &out.errors,
            )
        );
    }
}

#[test]
fn every_example_runs_and_both_engines_say_the_same_thing() {
    // The same claim the README makes about `hello.qnl`, made about all of them. An
    // example that compiles and then stops is still an example that is wrong, and two
    // engines printing different things is the thing this project exists to catch.
    for path in examples() {
        let source = std::fs::read_to_string(&path).expect("a readable example");
        let module = quench_lower::lower(&source).module.expect("it compiled");

        let (mut out, mut err) = (Vec::new(), Vec::new());
        let ended = quench_interp::run_writing(
            &module,
            &mut quench_interp::Writing { out: &mut out, err: &mut err },
        )
        .expect("it runs");
        assert!(
            matches!(ended, quench_qir::Outcome::Returned(_)),
            "{} stopped: {ended:?}",
            path.display()
        );
        let walked = quench_dev::Printed {
            out: String::from_utf8(out).expect("text"),
            err: String::from_utf8(err).expect("text"),
        };

        let (_, compiled) =
            quench_dev::compile(&module).expect("it compiles").run_capturing();
        assert_eq!(walked, compiled, "{} was printed differently", path.display());
    }
}

#[test]
fn the_readme_shows_the_example_it_names() {
    // The README's first block and `examples/hello.qnl` are the same program written
    // twice, and the three commands under it name the file. Two copies of one thing is
    // one copy too many unless something holds them together.
    let readme = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../README.md"
    ))
    .expect("the README is beside the crates");
    let after = readme.split_once("## Hello, World").expect("a Hello, World section").1;
    let block = after
        .split_once("```quench")
        .expect("a program in it")
        .1
        .split_once("```")
        .expect("a closed fence")
        .0;

    let file = std::fs::read_to_string(directory().join("hello.qnl")).expect("hello.qnl");
    assert_eq!(
        block.trim(),
        file.trim(),
        "the README shows one program and `examples/hello.qnl` holds another"
    );
}
