//! The README, run rather than read.
//!
//! Prose goes stale quietly, and this one did: an error example that produced a
//! different error entirely, a `b16` that is not built, a status row saying nothing
//! frees memory when both engines collect. Every one of those was a claim a machine
//! could have checked, and now does.
//!
//! Three rules, and each is a rule a reader can apply too:
//!
//! 1. **Every ```` ```quench ```` block is a program**, and it runs — unless a
//!    ```` ```text ```` block follows it, which means it is there to fail and rule two
//!    owns it.
//! 2. **A ```` ```text ```` block right after one is exactly what it says**, character
//!    for character — the diagnostic when the program does not compile, and what it
//!    prints when it does. Which of the two it is follows from the program, so there is
//!    nothing for a writer to mark and nothing to get wrong.
//! 3. **An inline snippet that is a whole statement is one.** Anything ending in `;`
//!    or `}` gets compiled; anything with an ellipsis in it is an illustration and is
//!    left alone. A name it never declared is allowed to be undeclared — the snippet is
//!    claiming a shape, and `'name'` in one is a stand-in rather than a promise.
//!
//! What is left over is prose, which nothing can check and which is therefore the only
//! thing to read carefully.

use quench_diag::SourceFile;

fn readme() -> String {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md");
    std::fs::read_to_string(path).expect("the README is beside the crates")
}

/// Every fenced block, with its tag, in the order they appear.
///
/// Indented ones too, and that was not always so: a fence inside a bullet is indented
/// to sit under it, and this used to insist on column nought. So three of the README's
/// programs -- every one written as part of a bullet, which is most of the interesting
/// ones -- were never compiled and never run. A block nothing checks is prose with
/// syntax highlighting.
fn blocks(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let indent = line.len() - line.trim_start().len();
        let Some(tag) = line.trim_start().strip_prefix("```") else { continue };
        let tag = tag.trim().to_string();
        let mut body = String::new();
        for line in lines.by_ref() {
            if line.trim_start().starts_with("```") {
                break;
            }
            // Back out the indent the bullet put on, so the program is the program.
            body.push_str(if line.len() >= indent { &line[indent..] } else { line.trim_start() });
            body.push('\n');
        }
        out.push((tag, body));
    }
    out
}

/// Everything between single backticks.
fn inline(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        // Skip a fence rather than reading it as three empty snippets.
        if rest[open..].starts_with("```") {
            let after = &rest[open + 3..];
            match after.find("```") {
                Some(close) => rest = &after[close + 3..],
                None => break,
            }
            continue;
        }
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else { break };
        out.push(after[..close].to_string());
        rest = &after[close + 1..];
    }
    out
}

fn refused(source: &str) -> Option<String> {
    let out = quench_lower::lower(source);
    if out.ok() {
        return None;
    }
    Some(quench_diag::report(&SourceFile::new("src/main.qnl", source), &out.errors))
}

/// The same, forgiving a name the snippet was never going to declare.
fn refused_on_shape(source: &str) -> Option<String> {
    let out = quench_lower::lower(source);
    // A snippet claims a *shape*, so the things a shape cannot carry are forgiven: a
    // name it never declared (E0413), and a file it never had. `import ['maths'];` is a
    // true line about a program whose other files are not in the line -- which files a
    // program has comes from `QNL-Config.toml`, and one snippet is not a program.
    if out.errors.iter().all(|e| e.code == "E0413" || e.code == "E0516") {
        return None;
    }
    Some(quench_diag::report(&SourceFile::new("src/main.qnl", source), &out.errors))
}

#[test]
fn every_quench_block_in_the_readme_is_a_program() {
    let found = blocks(&readme());
    for (n, (tag, body)) in found.iter().enumerate() {
        if tag != "quench" {
            continue;
        }
        // One that is followed by its output is there to fail, and the test below
        // checks it fails in exactly the way the README says it does.
        if found.get(n + 1).map(|(tag, _)| tag.as_str()) == Some("text") {
            continue;
        }
        if let Some(why) = refused(body) {
            panic!("block {n} of the README does not compile:\n\n{body}\n{why}");
        }
    }
}

#[test]
fn a_text_block_after_a_quench_one_is_what_that_program_says() {
    // Which is how the README shows off its errors, and how one of them came to show
    // off an error the compiler had stopped producing.
    //
    // "Says" is two things, and which one is decided by the program rather than by a
    // marker somebody has to remember: a program that does not compile says a
    // diagnostic, and one that does says whatever it prints. The convention could only
    // express the first until the Hello, World example needed the second -- and a
    // README that cannot show a program's output at the top of it is a README with the
    // wrong rule, not an example with the wrong shape.
    let text = readme();
    let blocks = blocks(&text);
    let (mut refusals, mut outputs) = (0, 0);
    for pair in blocks.windows(2) {
        let [(first, source), (second, shown)] = pair else { continue };
        if first != "quench" || second != "text" {
            continue;
        }
        let out = quench_lower::lower(source);
        let compiled_ok = out.ok();
        let Some(module) = out.module.filter(|_| compiled_ok) else {
            let said =
                quench_diag::report(&SourceFile::new("src/main.qnl", source), &out.errors);
            assert_eq!(said.trim_end(), shown.trim_end(), "\n--- the README says ---\n{shown}\n--- and it says ---\n{said}");
            refusals += 1;
            continue;
        };

        // Both engines, because the README's claim about the two of them agreeing is
        // one this can check rather than repeat.
        let (mut written, mut wrong) = (Vec::new(), Vec::new());
        quench_interp::run_writing(
            &module,
            &mut quench_interp::Outside {
                read: &mut std::io::empty(),
                out: &mut written,
                err: &mut wrong,
                arguments: &[],
            },
        )
        .expect("it runs");
        let walked = quench_dev::Printed {
            out: String::from_utf8(written).expect("text"),
            err: String::from_utf8(wrong).expect("text"),
        };
        let (_, compiled) =
            quench_dev::compile(&module).expect("it compiles").run_capturing();
        assert_eq!(walked, compiled, "the engines printed different things:\n{source}");
        assert_eq!(
            walked.out.trim_end(),
            shown.trim_end(),
            "\n--- the README says ---\n{shown}\n--- and it printed ---\n{}",
            walked.out
        );
        outputs += 1;
    }
    assert!(refusals > 0, "no program-and-diagnostic pair found; has the README stopped showing an error?");
    assert!(outputs > 0, "no program-and-output pair found; has the README stopped showing what one prints?");
}

#[test]
fn an_inline_snippet_that_is_a_whole_statement_is_one() {
    // `var.mut.b16 ['x'] = [*1000*];` sat in the README after `b16` stopped being
    // something the checker would take. This is the rule that catches that.
    let mut checked = 0;
    for snippet in inline(&readme()) {
        let trimmed = snippet.trim();
        if trimmed.contains('…') || trimmed.contains("<") {
            continue;
        }
        let whole = trimmed.ends_with(';') || trimmed.ends_with('}');
        if !whole || trimmed.len() < 4 {
            continue;
        }
        // A top-level thing stands alone; a statement needs somewhere to stand.
        let source = if trimmed.starts_with("fn.")
            || trimmed.starts_with("const.")
            || trimmed.starts_with("import ")
            || trimmed.starts_with("module.")
        {
            format!("{trimmed}\nSTART {{ }}\n")
        } else {
            format!("START {{\n    {trimmed}\n}}\n")
        };
        if let Some(why) = refused_on_shape(&source) {
            panic!("the README writes `{trimmed}`, and it does not check out:\n\n{why}");
        }
        checked += 1;
    }
    assert!(checked > 3, "only {checked} snippets were checked; has the rule stopped matching?");
}
