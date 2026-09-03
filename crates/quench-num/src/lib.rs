//! The numbers that do not fit in a register.
//!
//! Everything else Quench has is a fixed number of bits — a `b64` is exactly 64, an
//! `i32` exactly 32 — and lives in a register, never touching the heap. `e` is not: it
//! is an exact rational, and adding two of them multiplies their denominators with
//! nothing to bound how large the result gets.
//!
//! `e` exists for numbers that are absurdly large *and* exactly represented, which is
//! what decides the shape of everything here. See `notes/e-is-big-and-exact.md`.

pub mod big;
pub mod exact;

pub use big::Big;
pub use exact::{Exact, Trouble};

/// Why a power had no answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoPower {
    /// A whole number raised to a negative power is not a whole number.
    Negative,
    /// An exact number raised to a fraction is not exact — the square root of two is
    /// the oldest number known not to be a ratio.
    Fractional,
    /// The answer, or the exponent asking for it, is larger than can be held.
    TooLarge,
}

/// `base ^ exponent`, by squaring.
///
/// One implementation, called by every engine, for the same reason the exact
/// arithmetic is: the answer must not depend on who is running it.
pub fn power_i64(base: i64, exponent: i64, wrapping: bool) -> Result<i64, NoPower> {
    if exponent < 0 {
        // Which is a real number and not a whole one, and this is a whole-number type.
        return Err(NoPower::Negative);
    }
    let mut left = exponent as u64;
    let (mut base, mut answer) = (base, 1i64);
    while left > 0 {
        if left & 1 == 1 {
            answer = match (wrapping, answer.checked_mul(base)) {
                (_, Some(n)) => n,
                (true, None) => answer.wrapping_mul(base),
                (false, None) => return Err(NoPower::TooLarge),
            };
        }
        left >>= 1;
        if left > 0 {
            base = match (wrapping, base.checked_mul(base)) {
                (_, Some(n)) => n,
                (true, None) => base.wrapping_mul(base),
                (false, None) => return Err(NoPower::TooLarge),
            };
        }
    }
    Ok(answer)
}

impl Exact {
    /// `base ^ exponent`, exactly.
    ///
    /// A negative exponent is fine here and is the difference from `i64`: two to the
    /// minus one is a half, and a half is a number this type holds. What is refused is
    /// a *fractional* exponent, whose answer is generally not a ratio at all.
    pub fn power(&self, exponent: &Exact) -> Result<Exact, NoPower> {
        if !exponent.is_whole() {
            return Err(NoPower::Fractional);
        }
        let Some(times) = exponent.numerator().abs().to_u64() else {
            return Err(NoPower::TooLarge);
        };
        // Anything past this would not finish, and a program that does not finish is
        // worse than one that says why it stopped.
        if times > 1_000_000 {
            return Err(NoPower::TooLarge);
        }
        let mut base = self.clone();
        let mut answer = Exact::one();
        let mut left = times;
        while left > 0 {
            if left & 1 == 1 {
                answer = answer.mul(&base);
            }
            left >>= 1;
            if left > 0 {
                base = base.mul(&base);
            }
        }
        if exponent.is_negative() {
            return answer.reciprocal().map_err(|_| NoPower::Fractional);
        }
        Ok(answer)
    }
}

/// How a `b64` is shown, written once so that no engine can have its own idea of it.
///
/// The shortest text that reads back as the same 64 bits, and always with a point in
/// it — `1.0` rather than `1`, so that what is shown says which type it came from.
/// Every library that prints a float differs in the last digit or the exponent, which
/// is exactly the kind of difference three engines must not be allowed to have.
pub fn show_f64(x: f64) -> String {
    if x.is_nan() {
        return "not-a-number".to_string();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-infinity" } else { "infinity" }.to_string();
    }
    let shown = format!("{x}");
    if shown.contains(['.', 'e']) {
        return shown;
    }
    format!("{shown}.0")
}
