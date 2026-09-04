//! The `quench` command.

use quench_diag::SourceFile;
use std::process::ExitCode;

const USAGE: &str = "\
quench — a language that would rather say what it means

    quench run <file>           compile it with the Dev JIT and run it
    quench walk <file>          run it on the interpreter instead
    quench check <file.qnl>     check it and stop
    quench build <file.qnl>     write the artefact, and stop
    quench words                every word the language provides, and where it stands
    quench words --count        how many there are — a word may stand in two groups
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

/// The two numbers, because there are two and one line each is not obvious.
///
/// `quench words` prints one line per word *per group*, which is the useful shape and
/// is also a trap: `wc -l` is what anybody reaches for to count a language's words, and
/// it is the wrong number the moment one word stands in two places. `module` is the
/// first that does — it names the construct and the boundary the construct makes, the
/// way `file` names a boundary — so the list is 90 lines and 89 words. Both sessions
/// working on this repo read the line count and believed it.
fn counted_words() -> String {
    let listed = words();
    let places = listed.lines().count();
    let mut every: Vec<&str> =
        listed.lines().filter_map(|line| line.split_once('\t').map(|(_, word)| word)).collect();
    every.sort_unstable();
    every.dedup();
    let mut groups: Vec<&str> =
        listed.lines().filter_map(|line| line.split_once('\t').map(|(group, _)| group)).collect();
    groups.sort_unstable();
    groups.dedup();
    format!(
        "words\t{}\nplaces\t{places}\ngroups\t{}\n",
        every.len(),
        groups.len()
    )
}

/// Every file of one program, laid end to end.
struct Program {
    /// The whole text, with the files in order and a newline between them.
    whole: String,
    /// Each file: where it begins in `whole`, the file itself, and its module name.
    files: Vec<(usize, SourceFile, String)>,
    parts: Vec<quench_check::Part>,
}

/// Read every file the program is made of.
///
/// `[program] files` says what those are. When it says nothing, the program is the one
/// file it was given -- which is what every program was until it could be more.
fn gather(path: &str, listed: &[(String, String)]) -> Result<Program, String> {
    // The name a file's declarations go into is *chosen*, in `[program.files]`, rather
    // than taken from the filename -- so renaming a file does not rename a module, a
    // file may sit in a directory without the directory leaking into the name, and two
    // files may share a stem. When the program is one file there is nobody to name it
    // to, so it is `main`.
    let named: Vec<(String, String)> = if listed.is_empty() {
        vec![("main".to_string(), path.to_string())]
    } else {
        if !listed.iter().any(|(_, file)| same_file(file, path)) {
            let all: Vec<&str> = listed.iter().map(|(_, file)| file.as_str()).collect();
            return Err(format!(
                "`{path}` is not one of the files `[program.files]` lists.\n\
                 it lists {}.",
                all.join(", ")
            ));
        }
        listed.to_vec()
    };

    let mut whole = String::new();
    let mut files = Vec::new();
    let mut parts = Vec::new();
    for (name, at_path) in &named {
        let text = std::fs::read_to_string(at_path)
            .map_err(|why| format!("cannot read {at_path}: {why}"))?;
        let at = whole.len();
        whole.push_str(&text);
        // A newline between, so the last line of one file and the first of the next are
        // two lines however the file ended.
        whole.push('\n');
        files.push((at, SourceFile::new(at_path, text), name.clone()));
        parts.push(quench_check::Part { at, name: name.clone() });
    }
    Ok(Program { whole, files, parts })
}

/// Whether two written paths name the same file, as far as a person meant them to.
fn same_file(one: &str, other: &str) -> bool {
    one == other
        || std::path::Path::new(one).file_name() == std::path::Path::new(other).file_name()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Asked for on its own, because a word list kept anywhere but here is a second copy
    // of one, and the second copy is the one that rots. See `words`.
    if args.first().is_some_and(|first| first == "words") {
        let counting = args.get(1).is_some_and(|next| next == "--count");
        print!("{}", if counting { counted_words() } else { words() });
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

    // A `QNL-Config.toml` beside the source decides things the source does not say --
    // how division rounds, most of all -- so it is read before anything is compiled.
    // It also says what the program is *made of*, which is why it comes first.
    let (settings, listed) = match std::fs::read_to_string("QNL-Config.toml") {
        Ok(text) => {
            let read = quench_conf::read(&text);
            if !read.errors.is_empty() {
                let file = SourceFile::new("QNL-Config.toml", &text);
                eprint!("{}", quench_diag::report(&file, &read.errors));
                return ExitCode::FAILURE;
            }
            (read.settings, read.files)
        }
        Err(_) => (quench_conf::Settings::default(), Vec::new()),
    };

    let program = match gather(path, &listed) {
        Ok(program) => program,
        Err(why) => {
            eprintln!("{why}");
            return ExitCode::FAILURE;
        }
    };
    let sources = quench_diag::Sources::of(
        program.files.iter().map(|(at, file, _)| (*at, file)).collect(),
    );

    let lowered = quench_lower::lower_across(&program.whole, &program.parts, settings);
    if !lowered.ok() {
        eprint!("{}", quench_diag::report_across(&sources, &lowered.errors));
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
