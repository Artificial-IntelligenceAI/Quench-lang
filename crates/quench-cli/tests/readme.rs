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
//!    for character.
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
fn blocks(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(tag) = line.strip_prefix("```") else { continue };
        let tag = tag.trim().to_string();
        let mut body = String::new();
        for line in lines.by_ref() {
            if line.starts_with("```") {
                break;
            }
            body.push_str(line);
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
    if out.errors.iter().all(|e| e.code == "E0413") {
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
    let text = readme();
    let blocks = blocks(&text);
    let mut checked = 0;
    for pair in blocks.windows(2) {
        let [(first, source), (second, shown)] = pair else { continue };
        if first != "quench" || second != "text" {
            continue;
        }
        let out = quench_lower::lower(source);
        let said = quench_diag::report(&SourceFile::new("src/main.qnl", source), &out.errors);
        assert_eq!(said.trim_end(), shown.trim_end(), "\n--- the README says ---\n{shown}\n--- and it says ---\n{said}");
        checked += 1;
    }
    assert!(checked > 0, "no program-and-output pair found; has the README stopped showing one?");
}

#[test]
fn an_inline_snippet_that_is_a_whole_statement_is_one() {
    // `var.mut.b16 ['x'] = [*1000*];` sat in the README for weeks after `b16` stopped
    // being something the checker would take. This is the rule that catches that.
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
        let source = if trimmed.starts_with("fn.") || trimmed.starts_with("const.") {
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
