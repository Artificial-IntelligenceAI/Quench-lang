//! The `quench` command.

use quench_diag::SourceFile;
use std::process::ExitCode;

const USAGE: &str = "\
quench — a language that would rather say what it means

    quench run <file>           compile it with the Dev JIT and run it
    quench walk <file>           run it on the interpreter instead
    quench check <file.qnl>     check it and stop
    quench build <file.qnl>     write the artefact, and stop
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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
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
