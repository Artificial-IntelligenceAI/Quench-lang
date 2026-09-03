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

/// How hard the optimiser tries, and therefore how long it may take.
///
/// **Delivery**, but a special sort of it. It cannot change what a program answers —
/// every level must give the same result, and that is exactly what the oracle checks.
/// What it changes is *what the compiler does*, so unlike `embed-source` it is worth
/// sweeping: a bug that only appears at one level is a real bug, found only if
/// something compiled at that level.
///
/// # It means less than it looks like it means
///
/// Two of the four ways of running a program do not consult it at all, because their
/// level is decided by their job rather than by a preference:
///
/// - the **Dev JIT** is always [`Optimise::None`]. Being the engine that did the least
///   is what makes it the one to believe when the others disagree, and a setting able
///   to change that would take it away;
/// - the **Hot JIT** decides for itself, per function, by watching which ones are worth
///   it. That is what makes it the hot JIT.
///
/// So this is really about **ahead-of-time output**, which by default takes everything
/// it can and as long as it likes: nobody is waiting at a keyboard for a binary that is
/// about to be shipped, and it is the only engine whose compile time is spent once and
/// whose run time is spent by everyone. The setting exists so that someone in a hurry —
/// a build in CI that only has to exist — can ask for less.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Optimise {
    /// Compile fast and transform nothing.
    None,
    /// Make it fast. The default, because the engine this setting is really for is the
    /// one that ships.
    #[default]
    Speed,
    /// Make it fast, and prefer the smaller of two ways of doing that.
    SpeedAndSize,
}

/// What happens when a number does not fit.
///
/// **Semantic** — the most so of any setting here. `9223372036854775807 + 1` is the
/// smallest number under `wrap` and a stop under `trap`, so the same program answers
/// differently, and every engine must agree under *each*.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Overflow {
    /// Round the answer into the type, as the processor does. Fast, and occasionally
    /// the reason a program is quietly wrong for a year.
    #[default]
    Wrap,
    /// Stop, in the same place and for the same reason in every engine.
    Trap,
}

/// Whether `and` and `or` ask their right side once their left side has settled it.
///
/// **Semantic**, and it did not used to be. Until a program could call a function,
/// nothing inside an expression could *do* anything, so both answers gave byte-for-byte
/// identical programs and there was nothing here to choose. A call changed that: the
/// right side can print, and now the same source is two programs.
///
/// The reason to stop early is not speed. Quench stops rather than having undefined
/// behaviour, so `['n' != *0* and *100* / 'n' > *5*]` under [`Logic::AsksBoth`] does not
/// merely waste a division — it **stops the program** whenever `'n'` is nought. Guarding
/// a thing with the test that makes it safe is the idiom, and one of these two answers
/// does not have it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Logic {
    /// `and` asks its right side only when the left was true, and `or` only when the
    /// left was false. What every language with these operators does, and what makes a
    /// guard a guard.
    #[default]
    StopsEarly,
    /// Both sides, always, whatever the left one said. A branch fewer, and no guards.
    AsksBoth,
}

/// What a float does when there is no number to give back.
///
/// **Semantic**, and the same shape as `overflow`: `*1.0* / *0.0*` is `infinity` under
/// one and a stop under the other, so the same program answers differently and every
/// engine must agree under each.
///
/// The default is [`NoNumber::CarriesOn`] because `b64` *is* IEEE 754 binary64, and
/// `infinity` and `not-a-number` are values of that type rather than accidents of it.
/// Asking to stop is asking for something narrower than the type, which is a thing to
/// opt into rather than out of.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum NoNumber {
    /// `infinity`, `-infinity` or `not-a-number`, and the program carries on. What the
    /// processor does, so it costs nothing.
    #[default]
    CarriesOn,
    /// Stop, in the same place and for the same reason in every engine. Costs a check
    /// after every operation that could produce one.
    Stops,
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
    /// `[defaults] logic`
    pub logic: Logic,
    /// `[defaults] no-number`
    pub no_number: NoNumber,
    /// `[run] engine`
    pub engine: Engine,
    /// `[build] optimise`
    pub optimise: Optimise,
    /// `[defaults] overflow`
    pub overflow: Overflow,
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
            if !matches!(name, "defaults" | "run" | "build") {
                errors.push(
                    Diagnostic::new("E0701", format!("`[{name}]` is not a section this reads."))
                        .primary(span_of(trimmed), "here")
                        .rule("the sections are `[defaults]` for what a program means, `[build]` for what gets delivered, and `[run]` for how it runs")
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
            ("defaults", "logic") => match value {
                "stops-early" => settings.logic = Logic::StopsEarly,
                "asks-both" => settings.logic = Logic::AsksBoth,
                _ => errors
                    .push(bad_value(span_of(value), key, value, &["stops-early", "asks-both"])),
            },
            ("defaults", "no-number") => match value {
                "carries-on" => settings.no_number = NoNumber::CarriesOn,
                "stops" => settings.no_number = NoNumber::Stops,
                _ => errors
                    .push(bad_value(span_of(value), key, value, &["carries-on", "stops"])),
            },
            ("defaults", "overflow") => match value {
                "wrap" => settings.overflow = Overflow::Wrap,
                "trap" => settings.overflow = Overflow::Trap,
                _ => errors.push(bad_value(span_of(value), key, value, &["wrap", "trap"])),
            },
            ("build", "optimise") => match value {
                "none" => settings.optimise = Optimise::None,
                "speed" => settings.optimise = Optimise::Speed,
                "speed-and-size" => settings.optimise = Optimise::SpeedAndSize,
                _ => errors.push(bad_value(
                    span_of(value),
                    key,
                    value,
                    &["none", "speed", "speed-and-size"],
                )),
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
                        "defaults" => "`[defaults]` holds `division`, `logic`, `no-number` and `overflow`.",
                        "build" => "`[build]` holds `optimise`.",
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
