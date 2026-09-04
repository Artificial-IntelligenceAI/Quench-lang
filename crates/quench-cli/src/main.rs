//! The `quench` command.

use quench_diag::SourceFile;
use std::process::ExitCode;

const USAGE: &str = "\
quench — a language that would rather say what it means

    quench run <file>           compile it with the Dev JIT and run it
    quench walk <file>           run it on the interpreter instead
    quench check <file.qnl>     check it and stop
    quench build <file.qnl>     write the artefact, and stop
    quench words                every word the language provides, and where it stands
    quench --help               this

`run` and `walk` take source or an artefact. An artefact is compiled QIR, which
knows nothing about any machine — so one built here runs anywhere this does.
";

/// What a written-down program is called.
const ARTEFACT: &str = ".qnlo";

/// Run something that arrived rather than something that was compiled here.
///
/// The only difference a person sees is where an error comes from: a module built by
/// another version of Quench and a copy that stopped early look exactly alike, and both
/// are answered with the same two fixes.
fn run_artefact(what: &str, path: &str) -> ExitCode {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(why) => {
            eprintln!("cannot read {path}: {why}");
            return ExitCode::FAILURE;
        }
    };
    let module = match quench_qir::read(&bytes, path) {
        Ok(module) => module,
        Err(wrong) => {
            let file = SourceFile::new(path, "");
            eprint!("{}", quench_diag::report(&file, std::slice::from_ref(&wrong)));
            return ExitCode::FAILURE;
        }
    };
    match what {
        "walk" => match quench_interp::run(&module) {
            Ok(outcome) => report_outcome(outcome),
            Err(why) => {
                eprintln!("{why}");
                ExitCode::FAILURE
            }
        },
        "run" => match quench_dev::compile(&module) {
            Ok(compiled) => report_outcome(compiled.outcome()),
            Err(why) => {
                eprintln!("{why}");
                ExitCode::FAILURE
            }
        },
        other => {
            eprintln!("`{other}` wants source, and this is an artefact.\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

/// How a run ended, said the same way whichever engine ran it.
///
/// The same words from both is not tidiness — it is the claim the whole project rests
/// on, made visible at the one place a person actually reads.
fn report_outcome(outcome: quench_qir::Outcome) -> ExitCode {
    match outcome {
        quench_qir::Outcome::Returned(_) => ExitCode::SUCCESS,
        quench_qir::Outcome::Trapped(trap) => {
            eprintln!("the program stopped: {}", trap.describe());
            ExitCode::FAILURE
        }
    }
}

/// Every word the language gives meaning to, and where each may stand.
///
/// One line per word, `group`, a tab, then the word — readable by a person and parsable
/// by anything, which is the point. It exists because every list of these written down
/// anywhere else has been wrong: the statement keywords in a diagnostic, the provided
/// functions twice in an hour, the settings keys, and a website's copy of all of them.
/// This one is read out of the constants the compiler itself uses, so it cannot say
/// something the language does not do.
///
/// **None of these is reserved.** A name wears marks and a word does not, so
/// `var.immut.i64 ['loop']` is a variable called `'loop'` and always was.
fn words() -> String {
    let mut out = String::new();
    let mut group = |name: &str, words: &[&str]| {
        for word in words {
            out.push_str(name);
            out.push('\t');
            out.push_str(word);
            out.push('\n');
        }
    };
    group("statement", quench_parse::STATEMENTS);
    group("top level", quench_parse::TOP_LEVEL);
    group("after a block", quench_parse::AFTER_A_BLOCK);
    group("chain link", quench_check::CHAIN_LINKS);
    group("visibility", quench_check::Visibility::ALL);
    group("type", quench_check::Ty::NAMES);
    group("operator", quench_parse::OPERATORS);
    group("before a value", quench_parse::BEFORE_A_VALUE);
    group("literal", quench_check::LITERALS);
    let streams: Vec<&str> = quench_qir::Stream::ALL.iter().map(|s| s.name()).collect();
    group("stream", &streams);
    let provided: Vec<&str> = quench_check::PROVIDED.iter().map(|(word, _)| *word).collect();
    group("provided", &provided);
    out
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Asked for on its own, because a word list kept anywhere but here is a second copy
    // of one, and the second copy is the one that rots. See `words`.
    if args.first().is_some_and(|first| first == "words") {
        print!("{}", words());
        return ExitCode::SUCCESS;
    }

    let (what, path) = match args.as_slice() {
        [what, path] if !what.starts_with('-') => (what.as_str(), path.as_str()),
        _ => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
    };

    // An artefact is read rather than compiled, and read as something that arrived:
    // nothing in it is believed before it is checked. See
    // `notes/compile-once-run-anywhere.md`.
    if path.ends_with(ARTEFACT) {
        return run_artefact(what, path);
    }

    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(why) => {
            eprintln!("cannot read {path}: {why}");
            return ExitCode::FAILURE;
        }
    };

    // A `QNL-Config.toml` beside the source decides things the source does not say --
    // how division rounds, most of all -- so it is read before anything is compiled.
    let (settings, config_errors) = match std::fs::read_to_string("QNL-Config.toml") {
        Ok(text) => {
            let (settings, errors) = quench_conf::read(&text);
            if !errors.is_empty() {
                let file = SourceFile::new("QNL-Config.toml", &text);
                eprint!("{}", quench_diag::report(&file, &errors));
                return ExitCode::FAILURE;
            }
            (settings, errors)
        }
        Err(_) => (quench_conf::Settings::default(), Vec::new()),
    };
    let _ = config_errors;

    let lowered = quench_lower::lower_under(&source, settings);
    if !lowered.ok() {
        let file = SourceFile::new(path, &source);
        eprint!("{}", quench_diag::report(&file, &lowered.errors));
        return ExitCode::FAILURE;
    }
    let Some(module) = lowered.module else { return ExitCode::FAILURE };

    match what {
        "build" => {
            let out = format!("{}{ARTEFACT}", path.strip_suffix(".qnl").unwrap_or(path));
            let bytes = quench_qir::write(&module);
            match std::fs::write(&out, &bytes) {
                Ok(()) => {
                    println!("{out}, {} bytes.", bytes.len());
                    ExitCode::SUCCESS
                }
                Err(why) => {
                    eprintln!("cannot write {out}: {why}");
                    ExitCode::FAILURE
                }
            }
        }
        "check" => {
            println!("{path} is fine.");
            ExitCode::SUCCESS
        }
        "walk" => match quench_interp::run(&module) {
            Ok(outcome) => report_outcome(outcome),
            Err(why) => {
                eprintln!("{why}");
                ExitCode::FAILURE
            }
        },
        "run" => match quench_dev::compile(&module) {
            Ok(compiled) => report_outcome(compiled.outcome()),
            Err(why) => {
                eprintln!("{why}");
                ExitCode::FAILURE
            }
        },
        other => {
            eprintln!("`{other}` is not something quench does.\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
