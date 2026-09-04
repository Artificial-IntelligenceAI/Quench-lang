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
//! `remainder` is here now, and it is the odd one: its answer is *exact*, never rounded.
//! `x - y × n`, where `n` is the integer nearest `x / y` with ties going to the even
//! one, is representable in the same format as `x` — always, with no rounding anywhere.
//! Which is why it can be checked rather than believed: `tests/remainders.rs` works the
//! same answer out in rational arithmetic, where nothing rounds at all, and demands the
//! two agree.

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

/// One operation of `copysign`, `min`, `max` or `remainder`.
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
    /// `minimum` of IEEE 754-2019: a not-a-number wins, the way it does everywhere else
    /// in floating point. Which of these two a program gets is `[defaults] min-max`,
    /// and they are separate operations here rather than one with a mode, because every
    /// semantic setting in Quench is carried as a separate instruction.
    MinimumSpreading,
    /// `maximum`, the same the other way up.
    MaximumSpreading,
    /// IEEE `remainder`: `x − y × n`, where `n` is the integer nearest `x / y` and a tie
    /// goes to the even one. Not `%`, which takes `n` toward zero and can give an answer
    /// as large as `y`; this one is never larger than half of `y`, and can differ from
    /// `x` in sign.
    ///
    /// Exact. There is no rounding in it, which is unusual enough to be worth saying:
    /// every other float operation here answers with the nearest representable thing,
    /// and this one answers with the thing.
    Remainder,
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
        Paired::Remainder => remainder64(a, b),
        Paired::MinimumSpreading => {
            if a.is_nan() || b.is_nan() { f64::NAN } else { smaller64(a, b) }
        }
        Paired::MaximumSpreading => {
            if a.is_nan() || b.is_nan() { f64::NAN } else { larger64(a, b) }
        }
    }
}

pub fn paired32(op: Paired, a: f32, b: f32) -> f32 {
    match op {
        Paired::CopySign => a.copysign(b),
        Paired::Minimum => smaller32(a, b),
        Paired::Maximum => larger32(a, b),
        Paired::Remainder => remainder32(a, b),
        Paired::MinimumSpreading => {
            if a.is_nan() || b.is_nan() { f32::NAN } else { smaller32(a, b) }
        }
        Paired::MaximumSpreading => {
            if a.is_nan() || b.is_nan() { f32::NAN } else { larger32(a, b) }
        }
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

/// `x − y × n` with `n` the nearest integer to `x / y`, ties to even.
///
/// Built on `%`, which is `fmod` and is itself exact: it takes `n` toward zero, so what
/// is left is between nought and `y`. Nudging that into the half of `y` either side of
/// nought is one comparison and one subtraction, and the subtraction is exact because
/// both sides are within a factor of two of each other -- Sterbenz's lemma, which is the
/// reason this can be written in floats at all rather than in integers.
fn remainder64(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() || x.is_infinite() || y == 0.0 {
        return f64::NAN;
    }
    if y.is_infinite() {
        return x;
    }
    let left = x % y;
    let size = y.abs();
    let over = left.abs();
    // `size - over` rather than `2 × over` or `size / 2`: doubling can overflow and
    // halving can lose a subnormal, and this cannot do either.
    let rest = size - over;
    if over > rest || (over == rest && odd_quotient(x, y, left)) {
        return if left > 0.0 { left - size } else { left + size };
    }
    left
}

fn remainder32(x: f32, y: f32) -> f32 {
    // Worked out in the wider format and handed back narrow. Exact either way: the
    // answer is representable in binary32 whenever the operands are, so widening cannot
    // move it and narrowing cannot round it.
    remainder64(f64::from(x), f64::from(y)) as f32
}

/// Whether the `n` that `%` implied was odd, which is what settles a tie.
///
/// `(x − left) / y` is that `n` exactly when the division is exact, and it is: `x − left`
/// is a whole multiple of `y` by construction. What can go wrong is the multiple being
/// too large to hold, so only the last bit is asked for, by taking the whole thing
/// modulo two.
fn odd_quotient(x: f64, y: f64, left: f64) -> bool {
    let whole = (x - left) / y;
    // Beyond 2^53 every representable float is even, so a quotient that large is even.
    if whole.abs() >= 9_007_199_254_740_992.0 {
        return false;
    }
    (whole as i64) % 2 != 0
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
