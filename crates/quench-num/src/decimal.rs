//! IEEE 754 decimal — `d32` and `d64`.
//!
//! A number the way people write numbers: a coefficient of decimal digits and a power
//! of ten. `0.1` is *one tenth* here, exactly, the same as it is in [`crate::Exact`] —
//! but unlike an `e` it has a fixed number of digits, so `1 ÷ 3` rounds rather than
//! staying a third.
//!
//! # Why it is here rather than in the machine
//!
//! Hardware decimal exists on IBM POWER6 and later and on z/Architecture, and nowhere
//! else. So this is the software path, and on x86-64 and ARM64 it is the only path.
//! See `notes/decimal-is-a-delivery-question.md`.
//!
//! # Cohorts are kept
//!
//! `2.50` and `2.5` are the same *value* and not the same *number*: decimal keeps the
//! trailing zero, which is the whole reason a shop uses this format and not a binary
//! one. So arithmetic sets the exponent the standard says it should — the smaller of
//! the two for a sum, their total for a product — and rounding only happens when there
//! are more digits than the format holds.
//!
//! # The coefficient is a [`Big`]
//!
//! Aligning two exponents can want any number of digits, and a product of two
//! sixteen-digit numbers has thirty-two. Using the unbounded integer that was already
//! here and already tested means no width in this file is ever a question — it is
//! slower than packing into a `u128` and there is nothing to get wrong.

use crate::Big;
use std::cmp::Ordering;

/// Which of the two decimal formats, and everything that follows from it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Format {
    /// How many decimal digits the coefficient holds.
    pub digits: u32,
    /// The largest adjusted exponent. Past it is an infinity.
    pub top: i32,
    /// The smallest exponent any value may have. Below it is a signed nought.
    pub bottom: i32,
}

/// `d32` — seven digits.
pub const D32: Format = Format { digits: 7, top: 96, bottom: -101 };
/// `d64` — sixteen digits.
pub const D64: Format = Format { digits: 16, top: 384, bottom: -398 };

/// What a decimal is, when it is not a number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Class {
    Finite,
    Infinite,
    NotANumber,
}

/// One decimal number: a sign, a coefficient of digits, and a power of ten.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Decimal {
    pub negative: bool,
    coefficient: Big,
    exponent: i32,
    pub class: Class,
}

impl Decimal {
    pub fn zero(exponent: i32) -> Decimal {
        Decimal { negative: false, coefficient: Big::zero(), exponent, class: Class::Finite }
    }

    pub fn infinity(negative: bool) -> Decimal {
        Decimal { negative, coefficient: Big::zero(), exponent: 0, class: Class::Infinite }
    }

    pub fn not_a_number() -> Decimal {
        Decimal {
            negative: false,
            coefficient: Big::zero(),
            exponent: 0,
            class: Class::NotANumber,
        }
    }

    pub fn is_zero(&self) -> bool {
        self.class == Class::Finite && self.coefficient.is_zero()
    }

    /// `12`, `-3.75`, `0.1`, `2.5e3`. A decimal point is exact here and always was.
    pub fn parse(text: &str, format: Format) -> Option<Decimal> {
        let (negative, rest) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        if rest.is_empty() {
            return None;
        }

        // An exponent, if one was written.
        let (digits, written_exponent) = match rest.find(['e', 'E']) {
            Some(at) => {
                let (before, after) = rest.split_at(at);
                (before, after[1..].parse::<i32>().ok()?)
            }
            None => (rest, 0),
        };

        // And a point, whose position is the rest of the exponent.
        let (whole, fraction) = match digits.split_once('.') {
            Some((whole, fraction)) => (whole, fraction),
            None => (digits, ""),
        };
        if whole.is_empty() && fraction.is_empty() {
            return None;
        }
        let all = format!("{whole}{fraction}");
        if all.is_empty() || !all.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let coefficient = Big::parse(&all)?;
        let exponent = written_exponent.checked_sub(fraction.len() as i32)?;
        Some(settled(negative, coefficient, exponent, false, format))
    }

    pub fn add(&self, other: &Decimal, format: Format) -> Decimal {
        self.plus(other, false, format)
    }

    pub fn sub(&self, other: &Decimal, format: Format) -> Decimal {
        self.plus(other, true, format)
    }

    fn plus(&self, other: &Decimal, flip: bool, format: Format) -> Decimal {
        let other_negative = other.negative != flip;
        if let Some(odd) = strange_pair(self, other, other_negative, true) {
            return odd;
        }

        // Aligned to the smaller exponent, which is the one the standard prefers when
        // nothing has to be rounded away: `2.50 + 1.00` is `3.50` and not `3.5`.
        let exponent = self.exponent.min(other.exponent);
        let mine = scaled(&self.coefficient, self.exponent - exponent);
        let theirs = scaled(&other.coefficient, other.exponent - exponent);

        let (coefficient, negative) = if self.negative == other_negative {
            (mine.add(&theirs), self.negative)
        } else {
            match mine.cmp_abs(&theirs) {
                Ordering::Less => (theirs.sub(&mine), other_negative),
                _ => (mine.sub(&theirs), self.negative),
            }
        };
        // A sum of nought keeps a positive sign, except where both sides were negative.
        let negative = if coefficient.is_zero() {
            self.negative && other_negative
        } else {
            negative
        };
        settled(negative, coefficient, exponent, false, format)
    }

    pub fn mul(&self, other: &Decimal, format: Format) -> Decimal {
        if let Some(odd) = strange_pair(self, other, other.negative, false) {
            return odd;
        }
        let negative = self.negative != other.negative;
        let coefficient = self.coefficient.mul(&other.coefficient);
        settled(negative, coefficient, self.exponent + other.exponent, false, format)
    }

    /// Exactly, and then rounded — which is where a decimal stops being an `e`.
    pub fn div(&self, other: &Decimal, format: Format) -> Decimal {
        let negative = self.negative != other.negative;
        if self.class == Class::NotANumber || other.class == Class::NotANumber {
            return Decimal::not_a_number();
        }
        if self.class == Class::Infinite {
            // Infinity over infinity is the one division with no answer at all.
            return if other.class == Class::Infinite {
                Decimal::not_a_number()
            } else {
                Decimal::infinity(negative)
            };
        }
        if other.class == Class::Infinite {
            return Decimal::zero(0);
        }
        if other.coefficient.is_zero() {
            return if self.coefficient.is_zero() {
                Decimal::not_a_number()
            } else {
                Decimal::infinity(negative)
            };
        }
        if self.coefficient.is_zero() {
            return Decimal::zero(self.exponent - other.exponent);
        }

        // Enough digits to round from: one more than the format holds, plus a
        // remainder that says whether anything was left over.
        let wanted = format.digits as i32 + 1;
        let lift = (wanted - count_digits(&self.coefficient) as i32
            + count_digits(&other.coefficient) as i32)
            .max(0);
        let lifted = scaled(&self.coefficient, lift);
        let (quotient, rest) =
            lifted.div_rem(&other.coefficient).expect("a divisor of nought was refused above");
        let mut exponent = self.exponent - other.exponent - lift;

        // A division that came out exactly is walked back toward the exponent it would
        // have had if nothing needed lifting: `2.5 / 1` is `2.5` and not `2.500000`.
        // The standard calls that the ideal exponent, and stopping there is what keeps
        // a cohort from drifting every time something is divided by one.
        let mut quotient = quotient;
        if rest.is_zero() {
            let ideal = self.exponent - other.exponent;
            let ten = Big::from_u64(10);
            while exponent < ideal && !quotient.is_zero() {
                let (fewer, left) = quotient.div_rem(&ten).expect("ten is not nought");
                if !left.is_zero() {
                    break;
                }
                quotient = fewer;
                exponent += 1;
            }
        }
        settled(negative, quotient, exponent, !rest.is_zero(), format)
    }

    /// Which is larger, by value rather than by how it was written: `2.50` and `2.5`
    /// are neither.
    pub fn compare(&self, other: &Decimal) -> Option<Ordering> {
        if self.class == Class::NotANumber || other.class == Class::NotANumber {
            return None;
        }
        let mine = self.toward();
        let theirs = other.toward();
        if mine != 0 || theirs != 0 {
            return Some(mine.cmp(&theirs));
        }
        if self.is_zero() && other.is_zero() {
            return Some(Ordering::Equal);
        }
        if self.negative != other.negative {
            return Some(if self.negative { Ordering::Less } else { Ordering::Greater });
        }
        let exponent = self.exponent.min(other.exponent);
        // Two numbers whose exponents are further apart than either has digits cannot
        // need aligning to be told apart, and aligning them could want a great many.
        let far = (self.exponent - other.exponent).unsigned_abs() as usize;
        let (mine, theirs) = if far > 4096 {
            let mine = count_digits(&self.coefficient) as i32 + self.exponent;
            let theirs = count_digits(&other.coefficient) as i32 + other.exponent;
            return Some(if self.negative { theirs.cmp(&mine) } else { mine.cmp(&theirs) });
        } else {
            (
                scaled(&self.coefficient, self.exponent - exponent),
                scaled(&other.coefficient, other.exponent - exponent),
            )
        };
        let order = mine.cmp_abs(&theirs);
        Some(if self.negative { order.reverse() } else { order })
    }

    /// `-1`, `0` or `1` for a not-a-number-free sign of an infinity.
    fn toward(&self) -> i32 {
        match self.class {
            Class::Infinite if self.negative => -1,
            Class::Infinite => 1,
            _ => 0,
        }
    }
}

/// Whether either of a pair is strange, and what the answer is when one is.
fn strange_pair(
    left: &Decimal,
    right: &Decimal,
    right_negative: bool,
    adding: bool,
) -> Option<Decimal> {
    if left.class == Class::NotANumber || right.class == Class::NotANumber {
        return Some(Decimal::not_a_number());
    }
    match (left.class, right.class) {
        (Class::Infinite, Class::Infinite) => {
            if adding && left.negative != right_negative {
                // Infinity taken from infinity is the one sum with no answer.
                Some(Decimal::not_a_number())
            } else if adding {
                Some(Decimal::infinity(left.negative))
            } else {
                Some(Decimal::infinity(left.negative != right_negative))
            }
        }
        (Class::Infinite, _) => {
            if !adding && right.coefficient.is_zero() {
                return Some(Decimal::not_a_number());
            }
            Some(Decimal::infinity(left.negative != (!adding && right_negative)))
        }
        (_, Class::Infinite) => {
            if !adding && left.coefficient.is_zero() {
                return Some(Decimal::not_a_number());
            }
            Some(Decimal::infinity(if adding {
                right_negative
            } else {
                left.negative != right_negative
            }))
        }
        _ => None,
    }
}

/// A coefficient with the format's rounding, its overflow and its underflow applied.
///
/// Round to nearest, ties to even — the same rule as everywhere else in IEEE 754, and
/// the reason `2.5` and `3.5` both round to an even number of pence.
fn settled(
    negative: bool,
    coefficient: Big,
    exponent: i32,
    sticky: bool,
    format: Format,
) -> Decimal {
    let (mut coefficient, mut exponent, mut sticky) =
        (coefficient, exponent, sticky);

    // Too many digits: drop the extra ones and round on what went.
    let digits = count_digits(&coefficient);
    if digits > format.digits {
        let drop = digits - format.digits;
        let (kept, dropped) = split_off(&coefficient, drop);
        coefficient = round_up(kept, &dropped, drop, sticky);
        exponent += drop as i32;
        sticky = false;
    }

    // Too small to hold: fewer digits than the format's, down to none at all.
    if exponent < format.bottom {
        let drop = (format.bottom - exponent) as u32;
        if drop > count_digits(&coefficient) + 1 {
            return Decimal { negative, coefficient: Big::zero(), exponent: format.bottom, class: Class::Finite };
        }
        let (kept, dropped) = split_off(&coefficient, drop);
        coefficient = round_up(kept, &dropped, drop, sticky);
        exponent = format.bottom;
    }

    // Rounding can carry into one more digit than the format holds.
    if count_digits(&coefficient) > format.digits {
        let (kept, _) = split_off(&coefficient, 1);
        coefficient = kept;
        exponent += 1;
    }

    // Too large to hold: past the top, an infinity.
    let adjusted = exponent + count_digits(&coefficient) as i32 - 1;
    if !coefficient.is_zero() && adjusted > format.top {
        return Decimal::infinity(negative);
    }
    Decimal { negative, coefficient, exponent, class: Class::Finite }
}

/// A coefficient split into what is kept and what is dropped.
fn split_off(coefficient: &Big, drop: u32) -> (Big, Big) {
    let ten = power_of_ten(drop);
    coefficient.div_rem(&ten).expect("a power of ten is never nought")
}

/// The kept part, one higher when what was dropped says it should be.
fn round_up(kept: Big, dropped: &Big, drop: u32, sticky: bool) -> Big {
    let half = half_of_ten(drop);
    let order = dropped.cmp_abs(&half);
    let up = match order {
        Ordering::Greater => true,
        // Exactly half goes to whichever neighbour ends in an even digit — unless
        // something below the dropped digits was not nought, which breaks the tie.
        Ordering::Equal => sticky || is_odd(&kept),
        Ordering::Less => false,
    };
    if up { kept.add(&Big::from_u64(1)) } else { kept }
}

fn power_of_ten(n: u32) -> Big {
    Big::parse(&format!("1{}", "0".repeat(n as usize))).expect("digits")
}

fn half_of_ten(n: u32) -> Big {
    if n == 0 {
        return Big::zero();
    }
    Big::parse(&format!("5{}", "0".repeat(n as usize - 1))).expect("digits")
}

fn is_odd(n: &Big) -> bool {
    let (_, rest) = n.div_rem(&Big::from_u64(2)).expect("two is not nought");
    !rest.is_zero()
}

fn scaled(coefficient: &Big, by: i32) -> Big {
    if by <= 0 {
        return coefficient.clone();
    }
    coefficient.mul(&power_of_ten(by as u32))
}

/// How many decimal digits a coefficient has. Nought has one.
fn count_digits(n: &Big) -> u32 {
    if n.is_zero() {
        return 1;
    }
    n.to_string().trim_start_matches('-').len() as u32
}

/// How a decimal is shown, which is IEEE 754's own `to-scientific-string`.
///
/// The exponent decides the shape: a number whose digits sit near the point is written
/// plainly, and one whose exponent has carried it far away is written with an `e`. The
/// rule is the standard's rather than a preference, so that a value written by one
/// engine reads back the same in another.
impl std::fmt::Display for Decimal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.class {
            Class::NotANumber => return write!(f, "not-a-number"),
            Class::Infinite => {
                return write!(f, "{}infinity", if self.negative { "-" } else { "" })
            }
            Class::Finite => {}
        }
        if self.negative {
            write!(f, "-")?;
        }
        let digits = self.coefficient.to_string();
        let digits = digits.trim_start_matches('-');
        let adjusted = self.exponent + digits.len() as i32 - 1;

        // Written plainly when the point lands inside or just before the digits, and
        // with an exponent when it would take more than a handful of noughts to reach.
        if self.exponent <= 0 && adjusted >= -6 {
            let point = digits.len() as i32 + self.exponent;
            if point <= 0 {
                return write!(f, "0.{}{digits}", "0".repeat(-point as usize));
            }
            let (whole, fraction) = digits.split_at(point as usize);
            if fraction.is_empty() {
                return write!(f, "{whole}");
            }
            return write!(f, "{whole}.{fraction}");
        }

        let (first, rest) = digits.split_at(1);
        if rest.is_empty() {
            write!(f, "{first}E{}{adjusted}", if adjusted < 0 { "" } else { "+" })
        } else {
            write!(f, "{first}.{rest}E{}{adjusted}", if adjusted < 0 { "" } else { "+" })
        }
    }
}
