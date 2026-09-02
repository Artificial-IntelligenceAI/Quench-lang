//! `QNL-Config.toml` — what a project has decided, once, for every file in it.
//!
//! # Why this is read here rather than by a library
//!
//! Two reasons, and the second is the real one.
//!
//! A full TOML library is a large dependency for a handful of settings, and a project
//! that will not put a collector on a device which does not need one should not put a
//! deserialiser in a toolchain that does not need one either. That is Luarust's argument
//! and it still holds.
//!
//! The better reason is that **this file decides how every source file in the project is
//! built**, so a mistake in it deserves the same error a mistake in a source file gets:
//! the rule that was broken, the line, and the fix. A library says `invalid value at
//! line 4`, which is the wrong voice for the most consequential file in the project.
//!
//! # Two kinds of setting
//!
//! Settings do not all cost the same, and the expensive ones are not the ones that look
//! expensive. A setting that changes **what gets delivered** costs nothing to test,
//! because the answer is the same either way. A setting that changes **what a program
//! answers** multiplies the oracle, because three engines must agree under *each* value
//! of it rather than once overall. See `notes/every-knob-is-a-multiplier.md`.
//!
//! Which pile a setting is in is written on it below, because it is the thing most
//! worth knowing before adding another.

use quench_diag::{Diagnostic, Span};

/// How a division rounds, and which way its remainder leans.
///
/// **Semantic.** One setting for both, because they are one division: every convention
/// answers `a = q × b + r`, and choosing `q` decides `r`. Settling them separately is
/// how a language ends up with a remainder that does not match its quotient.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Division {
    /// Toward zero, and the remainder follows the dividend. `-7 / 2` is `-3` remainder
    /// `-1`. What every processor does, so it costs nothing.
    #[default]
    Truncated,
    /// Toward negative infinity, and the remainder follows the divisor. `-7 / 2` is `-4`
    /// remainder `1`. What a mathematician means, and what makes `x % n` always land in
    /// `0..n`. Costs a comparison and a correction on every division.
    Floored,
}

/// Which engine runs a program.
///
/// **Delivery.** Every engine gives the same answer — that is the entire point of the
/// oracle — so this changes how a program runs and never what it says.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Engine {
    #[default]
    DevJit,
    Interpreter,
}

/// What the project file said, with anything it did not mention left at its default.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Settings {
    /// `[defaults] division`
    pub division: Division,
    /// `[run] engine`
    pub engine: Engine,
}

/// Read a `QNL-Config.toml`, reporting everything wrong with it rather than the first
/// thing. A setting that is not understood leaves its default in place.
pub fn read(text: &str) -> (Settings, Vec<Diagnostic>) {
    let mut settings = Settings::default();
    let mut errors = Vec::new();
    let mut section = String::new();
    let mut at = 0usize;

    for line in text.split_inclusive('\n') {
        let start = at;
        at += line.len();

        let content = match line.split_once('#') {
            Some((before, _)) => before,
            None => line,
        };
        let trimmed = content.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Where this piece of the line actually sits in the file.
        let span_of = |needle: &str| -> Span {
            let offset = content.find(needle).unwrap_or(0);
            Span::new(start + offset, start + offset + needle.len())
        };

        if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let name = name.trim();
            if !matches!(name, "defaults" | "run") {
                errors.push(
                    Diagnostic::new("E0701", format!("`[{name}]` is not a section this reads."))
                        .primary(span_of(trimmed), "here")
                        .rule("the sections are `[defaults]`, for what a program means, and `[run]`, for how it runs")
                        .fix("remove it, or move its settings into a section that exists"),
                );
            }
            section = name.to_string();
            continue;
        }

        let Some((key, value)) = trimmed.split_once('=') else {
            errors.push(
                Diagnostic::new("E0702", "this line is neither a section nor a setting.")
                    .primary(span_of(trimmed), "here")
                    .rule("a setting is `key = \"value\"`, and a section is `[name]`")
                    .fix("write it as `key = \"value\"`"),
            );
            continue;
        };

        let key = key.trim();
        let value = value.trim().trim_matches('"');

        match (section.as_str(), key) {
            ("defaults", "division") => match value {
                "truncated" => settings.division = Division::Truncated,
                "floored" => settings.division = Division::Floored,
                _ => errors.push(bad_value(span_of(value), key, value, &["truncated", "floored"])),
            },
            ("run", "engine") => match value {
                "dev-jit" => settings.engine = Engine::DevJit,
                "interpreter" => settings.engine = Engine::Interpreter,
                _ => errors.push(bad_value(span_of(value), key, value, &["dev-jit", "interpreter"])),
            },
            ("", _) => errors.push(
                Diagnostic::new("E0703", format!("`{key}` is before any section."))
                    .primary(span_of(key), "here")
                    .rule("every setting belongs to a section, so that what it affects is written down")
                    .fix("put a `[defaults]` or `[run]` line above it"),
            ),
            (section, _) => errors.push(
                Diagnostic::new("E0704", format!("`{key}` is not a setting `[{section}]` has."))
                    .primary(span_of(key), "here")
                    .rule("a setting that is not understood is refused rather than ignored, since a project that set it meant something by it")
                    .tip(match section {
                        "defaults" => "`[defaults]` holds `division`.",
                        "run" => "`[run]` holds `engine`.",
                        _ => "that section holds nothing yet.",
                    })
                    .fix("check the spelling, or remove the line"),
            ),
        }
    }

    (settings, errors)
}

fn bad_value(span: Span, key: &str, given: &str, allowed: &[&str]) -> Diagnostic {
    let list = allowed.iter().map(|a| format!("`{a}`")).collect::<Vec<_>>().join(" or ");
    Diagnostic::new("E0705", format!("`{given}` is not something `{key}` can be."))
        .primary(span, "here")
        .rule(format!("`{key}` is {list}"))
        .fix(format!("use {list}"))
}
