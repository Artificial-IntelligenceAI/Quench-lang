//! The `quench` command.

use quench_diag::SourceFile;
use std::process::ExitCode;

const USAGE: &str = "\
quench — a language that would rather say what it means

    quench run <file.qnl>       compile it with the Dev JIT and run it
    quench walk <file.qnl>      run it on the interpreter instead
    quench check <file.qnl>     check it and stop
    quench --help               this
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (what, path) = match args.as_slice() {
        [what, path] if !what.starts_with('-') => (what.as_str(), path.as_str()),
        _ => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
    };

    let source = match std::fs::read_to_string(path) {
        Ok(source) => source,
        Err(why) => {
            eprintln!("cannot read {path}: {why}");
            return ExitCode::FAILURE;
        }
    };

    let lowered = quench_lower::lower(&source);
    if !lowered.ok() {
        let file = SourceFile::new(path, &source);
        eprint!("{}", quench_diag::report(&file, &lowered.errors));
        return ExitCode::FAILURE;
    }
    let Some(module) = lowered.module else { return ExitCode::FAILURE };

    match what {
        "check" => {
            println!("{path} is fine.");
            ExitCode::SUCCESS
        }
        "walk" => match quench_interp::run(&module) {
            Ok(quench_interp::Outcome::Returned(_)) => ExitCode::SUCCESS,
            Ok(quench_interp::Outcome::Trapped(trap)) => {
                eprintln!("the program stopped: {trap:?}");
                ExitCode::FAILURE
            }
            Err(why) => {
                eprintln!("{why}");
                ExitCode::FAILURE
            }
        },
        "run" => match quench_dev::compile(&module) {
            Ok(compiled) => {
                compiled.run();
                ExitCode::SUCCESS
            }
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
