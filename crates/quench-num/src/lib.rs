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
pub mod decimal;
pub mod maths;
pub mod transcend;
pub mod wide;
pub mod exact;

pub use big::Big;
pub use decimal::{Decimal, Format, D32, D64};
pub use maths::{Alone, Paired};
pub use wide::Wide;
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

/// A binary16, as its sixteen bits.
///
/// Round to nearest, ties to even, like everything else in IEEE 754. Written as two
/// plain conversions rather than one clever one, because the subnormals are where a
/// clever one goes wrong and they are a fifteenth of every value there is.
pub fn to_b16_bits(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 31) as u16) << 15;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;

    // A not-a-number stays one, keeping enough of its payload to still be one.
    if exponent == 0xff {
        if mantissa == 0 {
            return sign | 0x7c00;
        }
        return sign | 0x7e00 | ((mantissa >> 13) as u16 & 0x1ff);
    }
    // Every `f32` subnormal is far below the smallest binary16 there is.
    if exponent == 0 {
        return sign;
    }

    let e = exponent - 127;
    if e > 15 {
        return sign | 0x7c00;
    }
    let significand = u64::from(mantissa) | (1 << 23);

    if e >= -14 {
        // Normal there as well: eleven bits kept of twenty-four, so thirteen go.
        let mut kept = nearest(significand, 13);
        let mut biased = e + 15;
        // Rounding can carry into the next exponent, which is the whole reason this
        // is checked after rather than assumed before.
        if kept == 0x800 {
            biased += 1;
            kept = 0x400;
        }
        if biased > 30 {
            return sign | 0x7c00;
        }
        return sign | ((biased as u16) << 10) | (kept as u16 & 0x3ff);
    }

    // Subnormal there: the value is a whole number of `2^-24`, and that is how many.
    let drop = (-e - 1) as u32;
    let kept = nearest(significand, drop);
    sign | kept as u16
}

/// Which of two neighbours a significand rounds to, dropping that many bits.
fn nearest(significand: u64, drop: u32) -> u64 {
    if drop == 0 {
        return significand;
    }
    if drop >= 64 {
        return 0;
    }
    let half = 1u64 << (drop - 1);
    let rest = significand & ((1u64 << drop) - 1);
    let mut kept = significand >> drop;
    // Ties to even: exactly half goes to whichever neighbour ends in a nought.
    if rest > half || (rest == half && kept & 1 == 1) {
        kept += 1;
    }
    kept
}

/// And back, into the `f32` a `b16` is carried in.
pub fn from_b16_bits(h: u16) -> f32 {
    let sign = u32::from(h >> 15) << 31;
    let exponent = (h >> 10) & 0x1f;
    let mantissa = u32::from(h & 0x03ff);

    if exponent == 0 {
        if mantissa == 0 {
            return f32::from_bits(sign);
        }
        // Subnormal there, normal here: shift the leading one up into place.
        let mut m = mantissa;
        let mut e = -1i32;
        while m & 0x400 == 0 {
            m <<= 1;
            e -= 1;
        }
        return f32::from_bits(sign | (((114 + e) as u32) << 23) | ((m & 0x3ff) << 13));
    }
    if exponent == 0x1f {
        return f32::from_bits(sign | 0x7f80_0000 | (mantissa << 13));
    }
    f32::from_bits(sign | ((u32::from(exponent) + 112) << 23) | (mantissa << 13))
}

/// The nearest binary16 to an `f32`, handed back in an `f32`.
///
/// A `b16` is carried in a 32-bit register holding a value binary16 can represent
/// exactly. This is what puts it back in that set after every operation, and it is the
/// whole of what makes a `b16` a `b16` — one implementation, called by every engine,
/// because neither Rust nor every Cranelift backend has a half to be identical about.
pub fn to_b16(x: f32) -> f32 {
    from_b16_bits(to_b16_bits(x))
}

/// How a `b32` is shown. Shortest round-trip, always with a point in it.
pub fn show_f32(x: f32) -> String {
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

/// `to_b16` when asked for, and the value untouched when not.
///
/// For whoever is holding a width rather than a type — the program generator, most of
/// all, which picks one per seed.
pub fn to_b16_or_f32(x: f32, half: bool) -> f32 {
    if half { to_b16(x) } else { x }
}
