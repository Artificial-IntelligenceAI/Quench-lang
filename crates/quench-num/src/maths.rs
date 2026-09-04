//! The half of a maths library that IEEE 754 actually specifies.
//!
//! The standard *requires* these to be correctly rounded, which is what makes them safe
//! to have: every engine must produce identical bits, so there is nothing here for a
//! differential oracle to catch and nothing for two implementations to disagree about.
//!
//! What is deliberately absent is the other half — `sin`, `cos`, `log`, `exp`, `pow`,
//! `atan2`. IEEE only *recommends* correct rounding for those and no library delivers
//! it, so three engines calling three C libraries is three answers. They arrive when
//! somebody writes them once, here, the way `Exact` and `Decimal` are written once.
//!
//! `remainder` is absent for a smaller reason: it is required, but computing it as
//! `x - y × round(x / y)` is wrong whenever `x / y` overflows or loses precision, and a
//! correct implementation is repeated subtraction with exponent bookkeeping. A maths
//! function that is subtly wrong in both engines at once is the one bug this project
//! cannot see, so it waits for somebody to write it properly.

/// One operation of `sqrt`, `abs`, `floor`, `ceil`, `round` or `trunc`.
///
/// Named rather than numbered so that a reader of the IR sees which it is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alone {
    /// Correctly rounded, and required to be — the only one here that is not exact.
    Sqrt,
    /// The sign bit cleared. Exact, and works on a not-a-number too.
    Abs,
    /// Toward negative infinity.
    Floor,
    /// Toward positive infinity.
    Ceiling,
    /// To the nearest whole number, ties to even. **Not** Rust's `round`, which breaks
    /// ties away from zero — IEEE says `roundToIntegralTiesToEven`, so `*2.5*` is `2`.
    Round,
    /// Toward zero.
    Truncate,
}

/// One operation of `copysign`, `min` or `max`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Paired {
    /// The magnitude of the first and the sign of the second.
    CopySign,
    /// `minimumNumber` of IEEE 754-2019: a not-a-number loses to a real number, and two
    /// noughts are ordered so that `-0` is the smaller.
    ///
    /// 754-2008 had one pair of these and 754-2019 has four, because the old ones
    /// handled not-a-number in a way nobody wanted. This is the one that treats a
    /// not-a-number as missing data rather than as contagion.
    Minimum,
    /// `maximumNumber`, the same the other way up.
    Maximum,
}

pub fn alone64(op: Alone, x: f64) -> f64 {
    match op {
        Alone::Sqrt => x.sqrt(),
        Alone::Abs => x.abs(),
        Alone::Floor => x.floor(),
        Alone::Ceiling => x.ceil(),
        Alone::Round => x.round_ties_even(),
        Alone::Truncate => x.trunc(),
    }
}

pub fn alone32(op: Alone, x: f32) -> f32 {
    match op {
        Alone::Sqrt => x.sqrt(),
        Alone::Abs => x.abs(),
        Alone::Floor => x.floor(),
        Alone::Ceiling => x.ceil(),
        Alone::Round => x.round_ties_even(),
        Alone::Truncate => x.trunc(),
    }
}

pub fn paired64(op: Paired, a: f64, b: f64) -> f64 {
    match op {
        Paired::CopySign => a.copysign(b),
        Paired::Minimum => smaller64(a, b),
        Paired::Maximum => larger64(a, b),
    }
}

pub fn paired32(op: Paired, a: f32, b: f32) -> f32 {
    match op {
        Paired::CopySign => a.copysign(b),
        Paired::Minimum => smaller32(a, b),
        Paired::Maximum => larger32(a, b),
    }
}

/// Multiply and add with one rounding rather than two, which is what makes it worth
/// having: `a × b + c` rounds twice and this rounds once.
pub fn fused64(a: f64, b: f64, c: f64) -> f64 {
    a.mul_add(b, c)
}

pub fn fused32(a: f32, b: f32, c: f32) -> f32 {
    a.mul_add(b, c)
}

// Written out rather than deferred to `f64::min`, because Rust's returns the *other*
// operand when one is a not-a-number and says nothing about which nought it prefers.
// Both of those are answers a program can see, so both are spelled here.
fn smaller64(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return b;
    }
    if b.is_nan() {
        return a;
    }
    if a == b {
        // Two noughts compare equal and are not the same number to look at.
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b { a } else { b }
}

fn larger64(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return b;
    }
    if b.is_nan() {
        return a;
    }
    if a == b {
        return if a.is_sign_positive() { a } else { b };
    }
    if a > b { a } else { b }
}

fn smaller32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return b;
    }
    if b.is_nan() {
        return a;
    }
    if a == b {
        return if a.is_sign_negative() { a } else { b };
    }
    if a < b { a } else { b }
}

fn larger32(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return b;
    }
    if b.is_nan() {
        return a;
    }
    if a == b {
        return if a.is_sign_positive() { a } else { b };
    }
    if a > b { a } else { b }
}
