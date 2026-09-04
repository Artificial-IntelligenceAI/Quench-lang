//! Reading a number out of text, which is the same question at two different times.
//!
//! A literal is text the compiler reads: `*42*` is an `i64` because these functions say
//! it is. `call as.i64['line']` is text the *program* reads, and it has to mean exactly
//! the same thing — otherwise there are two answers to "is this an `i64`", one given
//! while compiling and one while running, and a writer would have to learn both.
//!
//! So there is one implementation and both callers use it. The checker calls these to
//! read a written value; [`quench_qir::Host::TextReads`] and its `TextAs…` neighbours
//! call them while a program runs. What `as.i64` accepts is, by construction, the text
//! an `i64` could have been written with.
//!
//! The pair `is` and `as` agree for the same reason and by the same trick the `Print`
//! and `Say` hosts already use: one function works the answer out, and one caller asks
//! whether there was one while the other asks what it was.

use crate::{Decimal, Exact, Format};

/// What text turned out to be, read as a whole-number type.
///
/// Three answers rather than two because the checker says different things about each:
/// `*hello*` is not a number at all, and `*200*` is a number that an `i8` cannot hold.
/// A program reading text at runtime treats both as a refusal, and the writer who asked
/// `is` first never sees either.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Whole {
    /// Normalised into an `i64` slot the way every whole number in Quench rides:
    /// sign-extended when signed, zero-extended when not.
    Read(i64),
    /// A whole number, and outside what this many bits hold.
    Outside,
    /// Not a whole number.
    NotOne,
}

/// The lowest and highest a whole-number type holds.
///
/// The high end of a `u64` does not fit in an `i64`, and is carried as the bits of one
/// — which is what the whole type does, so nothing is lost by saying it here too.
pub fn whole_range(bits: u8, signed: bool) -> (i64, i64) {
    if signed {
        let high = if bits >= 64 { i64::MAX } else { (1i64 << (bits - 1)) - 1 };
        (-high - 1, high)
    } else {
        let high = if bits >= 64 { -1i64 } else { (1i64 << bits) - 1 };
        (0, high)
    }
}

/// Text read as `i8` through `i64`, or `u8` through `u64`.
pub fn read_whole(text: &str, bits: u8, signed: bool) -> Whole {
    let (low, high) = whole_range(bits, signed);
    if signed {
        match text.parse::<i64>() {
            Ok(n) if n < low || n > high => Whole::Outside,
            Ok(n) => Whole::Read(n),
            // A number too large for an `i64` is still a number, and saying so is the
            // difference between "that is not whole" and "that does not fit".
            Err(_) if text.parse::<u64>().is_ok() => Whole::Outside,
            Err(_) => Whole::NotOne,
        }
    } else {
        match text.parse::<u64>() {
            Ok(n) if n > high as u64 => Whole::Outside,
            Ok(n) => Whole::Read(n as i64),
            Err(_) if text.parse::<i64>().is_ok() => Whole::Outside,
            Err(_) => Whole::NotOne,
        }
    }
}

/// Text read as `b16`, `b32` or `b64`, giving the bits of one.
///
/// `infinity` and `not-a-number` are answers a program can reach and not things it can
/// write, which is the rule a literal already follows — so text saying either of them
/// is text that is not a float, here and in a source file alike.
///
/// A `b16` is rounded to the nearest binary16 rather than refused, because that is what
/// asking for a `b16` means: the type has three digits and the text may have twenty.
pub fn read_float(text: &str, width: u8) -> Option<u64> {
    match width {
        64 => text.parse::<f64>().ok().filter(|x| x.is_finite()).map(f64::to_bits),
        32 => text
            .parse::<f32>()
            .ok()
            .filter(|x| x.is_finite())
            .map(|x| u64::from(x.to_bits())),
        _ => text
            .parse::<f32>()
            .ok()
            .filter(|x| x.is_finite())
            .map(|x| u64::from(crate::to_b16(x).to_bits())),
    }
}

/// Text read as an `e`. `12`, `-3/4` and `0.1` are all exact.
pub fn read_exact(text: &str) -> Option<Exact> {
    Exact::parse(text)
}

/// Text read as a `d32` or a `d64`, rounded to that many significant digits.
pub fn read_decimal(text: &str, format: Format) -> Option<Decimal> {
    Decimal::parse(text, format)
}

/// Text read as a `bool`, which is the two words a program can write and no others.
///
/// Not `yes`, not `1`, not `True`. A literal is `true` or `false` and this is the same
/// question — the whole point of the file.
pub fn read_bool(text: &str) -> Option<bool> {
    match text {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}
