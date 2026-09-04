//! A binary float as wide as the answer needs, for the maths IEEE only recommends.
//!
//! `sqrt` and the rest are *required* to be correctly rounded, so every library has them
//! right and Quench can hand them to the machine. `sin`, `log`, `exp` and `pow` are only
//! **recommended**, which in practice means every library is a little bit wrong in its
//! own way — so three engines calling three C libraries would be three answers, and the
//! whole premise of this project is that they agree.
//!
//! The way out is the one thing Quench has and a C library does not: it does not have to
//! be fast. The reference engine is the one that does the least, and there are unbounded
//! integers already in the tree. So a transcendental is worked out *here*, at whatever
//! precision the answer turns out to need, and rounded once at the end.
//!
//! # How a wrong answer is ruled out
//!
//! Working at a hundred bits and rounding to fifty-three is not enough on its own,
//! because the true answer might sit so close to the halfway point between two `b64`s
//! that a hundred bits cannot say which side it is on. That is the table-maker's
//! dilemma, and the answer to it is Ziv's: work out an interval that certainly contains
//! the true value, and ask whether every number in it rounds the same way. If it does,
//! that is the answer and it is provably right. If it does not, double the precision and
//! ask again. It terminates because the true value is not exactly a halfway point —
//! except where it is, and those are the cases named in the functions themselves.
//!
//! So [`Wide`] carries a precision and every operation on it is correctly rounded to
//! that precision. What it costs is speed, and speed was never what the interpreter was
//! for.

use std::cmp::Ordering;

use crate::Big;

/// A number as `± mantissa × 2^exponent`, with the mantissa held to a set number of bits.
///
/// Normalised: the mantissa has exactly `bits` significant bits, or is nought. Two
/// numbers of different precisions can meet, and the answer is worked out at the wider
/// of the two so that nothing is thrown away before it has to be.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Wide {
    negative: bool,
    mantissa: Big,
    exponent: i64,
    bits: u64,
}

impl Wide {
    pub fn zero(bits: u64) -> Wide {
        Wide { negative: false, mantissa: Big::zero(), exponent: 0, bits }
    }

    pub fn is_zero(&self) -> bool {
        self.mantissa.is_zero()
    }

    pub fn is_negative(&self) -> bool {
        self.negative
    }

    pub fn bits(&self) -> u64 {
        self.bits
    }

    /// A whole number, exactly, held to this many bits.
    pub fn whole(n: i64, bits: u64) -> Wide {
        let negative = n < 0;
        Wide::made(negative, Big::from_u64(n.unsigned_abs()), 0, bits)
    }

    /// A `b64`, exactly — every one of them is a whole number times a power of two, so
    /// nothing is lost coming in. Not-a-number and infinity are the caller's business.
    pub fn from_f64(x: f64, bits: u64) -> Wide {
        let raw = x.to_bits();
        let negative = raw >> 63 == 1;
        let biased = ((raw >> 52) & 0x7FF) as i64;
        let fraction = raw & 0x000F_FFFF_FFFF_FFFF;
        let (mantissa, exponent) = if biased == 0 {
            (fraction, -1074)
        } else {
            (fraction | 0x0010_0000_0000_0000, biased - 1075)
        };
        Wide::made(negative, Big::from_u64(mantissa), exponent, bits)
    }

    /// Rounded to the nearest `b64`, ties to even.
    ///
    /// This is where the width finally narrows, and it is the only place it does. An
    /// answer that reaches here has already been proved to round the same way from every
    /// point in its uncertainty, so this rounding is the only one that happens.
    pub fn to_f64(&self) -> f64 {
        if self.is_zero() {
            return if self.negative { -0.0 } else { 0.0 };
        }
        let top = self.mantissa.bits() as i64 + self.exponent; // one past the leading bit
        // Far outside what a `b64` reaches, answered without arithmetic. Not only for
        // speed: scaling toward an exponent of ten billion takes ten million steps, so
        // this is the difference between an answer and a program that never finishes.
        // The bounds are loose on purpose -- anything near the edge goes the long way,
        // where the rounding is done properly.
        if top > 1100 {
            return if self.negative { f64::NEG_INFINITY } else { f64::INFINITY };
        }
        if top < -1200 {
            return if self.negative { -0.0 } else { 0.0 };
        }
        // Where the significand's last bit falls, for a normal `b64`.
        let mut lowest = top - 53;
        // Subnormal: the exponent cannot go below this, so bits are lost instead.
        if lowest < -1074 {
            lowest = -1074;
        }
        let shift = lowest - self.exponent;
        let (kept, round, sticky) = if shift <= 0 {
            (self.mantissa.shifted_up(shift.unsigned_abs()), false, false)
        } else {
            let by = shift as u64;
            (
                self.mantissa.shifted_down(by),
                self.mantissa.bit(by - 1),
                by >= 2 && self.mantissa.any_below(by - 1),
            )
        };
        let mut kept = kept;
        if round && (sticky || kept.bit(0)) {
            kept = kept.add(&Big::from_u64(1));
        }
        let value = scaled_f64(&kept, lowest);
        if self.negative { -value } else { value }
    }

    pub fn negated(&self) -> Wide {
        Wide { negative: !self.negative, ..self.clone() }
    }

    pub fn abs(&self) -> Wide {
        Wide { negative: false, ..self.clone() }
    }

    /// The same number held to a different number of bits, rounded if it narrows.
    pub fn to_bits_of(&self, bits: u64) -> Wide {
        Wide::made(self.negative, self.mantissa.clone(), self.exponent, bits)
    }

    pub fn add(&self, other: &Wide) -> Wide {
        let bits = self.bits.max(other.bits);
        if self.is_zero() && other.is_zero() {
            // Two noughts make a positive one unless both were negative, which is what
            // rounding to nearest says and is the only case where the sign is a choice.
            return Wide { negative: self.negative && other.negative, ..Wide::zero(bits) };
        }
        if self.is_zero() {
            return other.to_bits_of(bits);
        }
        if other.is_zero() {
            return self.to_bits_of(bits);
        }
        // Lined up on the lower exponent, which is exact: shifting a mantissa up loses
        // nothing, and the rounding happens once at the end.
        let exponent = self.exponent.min(other.exponent);
        let mine = self.mantissa.shifted_up((self.exponent - exponent).unsigned_abs());
        let theirs = other.mantissa.shifted_up((other.exponent - exponent).unsigned_abs());
        let (mine, theirs) = (
            if self.negative { mine.negated() } else { mine },
            if other.negative { theirs.negated() } else { theirs },
        );
        let sum = mine.add(&theirs);
        Wide::made(sum.is_negative(), sum.abs(), exponent, bits)
    }

    pub fn sub(&self, other: &Wide) -> Wide {
        self.add(&other.negated())
    }

    pub fn mul(&self, other: &Wide) -> Wide {
        let bits = self.bits.max(other.bits);
        Wide::made(
            self.negative != other.negative,
            self.mantissa.mul(&other.mantissa),
            self.exponent + other.exponent,
            bits,
        )
    }

    /// Correctly rounded, by working out enough bits of the quotient to round on and a
    /// sticky bit saying whether anything was left over.
    pub fn div(&self, other: &Wide) -> Wide {
        let bits = self.bits.max(other.bits);
        if self.is_zero() || other.is_zero() {
            return Wide::zero(bits);
        }
        // Two extra bits: one to round on, one to be sticky.
        let want = bits + 2;
        let have = self.mantissa.bits() as i64 - other.mantissa.bits() as i64;
        let lift = (want as i64 - have).max(0) as u64;
        let top = self.mantissa.shifted_up(lift);
        let (quotient, rest) = top.div_rem(&other.mantissa).expect("a divisor above nought");
        // The leftover cannot survive as a bit, so it survives as one bit: anything at
        // all below the last kept place makes the answer bigger than what is written.
        let quotient = if rest.is_zero() {
            quotient
        } else {
            quotient.shifted_up(1).add(&Big::from_u64(1))
        };
        let shift = if rest.is_zero() { 0 } else { 1 };
        Wide::made(
            self.negative != other.negative,
            quotient,
            self.exponent - other.exponent - lift as i64 - shift,
            bits,
        )
    }

    /// Multiplied by two that many times, which is exact and costs nothing.
    pub fn scaled(&self, by: i64) -> Wide {
        if self.is_zero() {
            return self.clone();
        }
        Wide { exponent: self.exponent + by, ..self.clone() }
    }

    /// A whole number of any size, exactly, held to this many bits.
    pub fn from_big(n: &Big, bits: u64) -> Wide {
        Wide::made(n.is_negative(), n.abs(), 0, bits)
    }

    /// The whole-number part, ignoring the sign. What a test uses to read the digits of
    /// a value off against a constant somebody else published.
    pub fn floor_abs(&self) -> Big {
        if self.is_zero() || self.mantissa.bits() as i64 + self.exponent <= 0 {
            return Big::zero();
        }
        if self.exponent >= 0 {
            self.mantissa.shifted_up(self.exponent as u64)
        } else {
            self.mantissa.shifted_down(self.exponent.unsigned_abs())
        }
    }

    /// The square root, correctly rounded to this value's width.
    ///
    /// A square root of a binary float is a square root of a whole number and a halving
    /// of the exponent, so the whole thing is one integer square root once the exponent
    /// has been made even. Enough bits are asked for that the root comes out two wider
    /// than the answer needs, and whatever the root left over becomes the sticky bit —
    /// so the rounding at the end is the ordinary one and is correct.
    pub fn sqrt(&self) -> Wide {
        if self.is_zero() || self.negative {
            return Wide { negative: self.negative, ..Wide::zero(self.bits) };
        }
        let want = (self.bits + 2) * 2;
        let have = self.mantissa.bits();
        // Lift by an even number, so that halving the exponent stays whole.
        let mut lift = want.saturating_sub(have);
        let mut exponent = self.exponent;
        if (exponent - lift as i64) % 2 != 0 {
            lift += 1;
        }
        let lifted = self.mantissa.shifted_up(lift);
        exponent -= lift as i64;
        let (root, rest) = lifted.sqrt_floor();
        // A root that did not come out exactly is *above* what is written, and one bit
        // is all that has to survive of it.
        let (root, exponent) = if rest {
            (root.shifted_up(1).add(&Big::from_u64(1)), exponent / 2 - 1)
        } else {
            (root, exponent / 2)
        };
        Wide::made(false, root, exponent, self.bits)
    }

    pub fn cmp_abs(&self, other: &Wide) -> Ordering {
        if self.is_zero() || other.is_zero() {
            return self.is_zero().cmp(&other.is_zero()).reverse();
        }
        let mine = self.mantissa.bits() as i64 + self.exponent;
        let theirs = other.mantissa.bits() as i64 + other.exponent;
        if mine != theirs {
            return mine.cmp(&theirs);
        }
        let exponent = self.exponent.min(other.exponent);
        self.mantissa
            .shifted_up((self.exponent - exponent).unsigned_abs())
            .cmp_abs(&other.mantissa.shifted_up((other.exponent - exponent).unsigned_abs()))
    }

    /// Normalised to exactly `bits` significant bits, rounding to nearest with ties to
    /// even — the same rule the hardware uses, applied at a width the hardware has not
    /// got.
    fn made(negative: bool, mantissa: Big, exponent: i64, bits: u64) -> Wide {
        if mantissa.is_zero() {
            // A nought keeps its sign. `-0` and `0` are the same number and not the
            // same answer: `sin` of one is one and of the other is the other, and a
            // reciprocal tells them apart in the loudest way there is.
            return Wide { negative, ..Wide::zero(bits) };
        }
        let have = mantissa.bits();
        let (mantissa, exponent) = match have.cmp(&bits) {
            Ordering::Equal => (mantissa, exponent),
            Ordering::Less => {
                let up = bits - have;
                (mantissa.shifted_up(up), exponent - up as i64)
            }
            Ordering::Greater => {
                let down = have - bits;
                let round = mantissa.bit(down - 1);
                let sticky = down >= 2 && mantissa.any_below(down - 1);
                let mut kept = mantissa.shifted_down(down);
                if round && (sticky || kept.bit(0)) {
                    kept = kept.add(&Big::from_u64(1));
                    // Rounding up can carry into a new bit, which costs one more shift.
                    if kept.bits() > bits {
                        return Wide {
                            negative,
                            mantissa: kept.shifted_down(1),
                            exponent: exponent + down as i64 + 1,
                            bits,
                        };
                    }
                }
                (kept, exponent + down as i64)
            }
        };
        Wide { negative, mantissa, exponent, bits }
    }
}

/// `mantissa × 2^exponent` as an `f64`, where the mantissa already fits in 53 bits.
fn scaled_f64(mantissa: &Big, exponent: i64) -> f64 {
    let Some(whole) = mantissa.to_u64() else { return f64::INFINITY };
    let value = whole as f64;
    // `powi` would round twice. Multiplying by a power of two is exact, so this is done
    // in steps small enough that each one is.
    let mut out = value;
    let mut left = exponent;
    while left > 0 {
        let step = left.min(1000);
        out *= f64::from_bits(((1023 + step) as u64) << 52);
        left -= step;
    }
    while left < 0 {
        let step = (-left).min(1000);
        out /= f64::from_bits(((1023 + step) as u64) << 52);
        left += step;
    }
    out
}
