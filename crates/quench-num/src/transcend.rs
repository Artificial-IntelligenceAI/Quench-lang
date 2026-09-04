//! The maths IEEE 754 only recommends, worked out until the answer is certain.
//!
//! Every one of these is computed in [`Wide`] at a precision chosen by the answer rather
//! than by the caller. The series are the ordinary ones; what is unusual is that they are
//! run at whatever width it takes and then rounded exactly once, and that the rounding is
//! only accepted when it can be *proved*.
//!
//! # Ziv's strategy, which is the whole of the correctness argument
//!
//! Every operation in `Wide` is correctly rounded to its working width, so an answer
//! computed at `p` bits is within a small number of ulps of the truth — call it `slack`
//! ulps, counted conservatively from how many operations went into it. If the value and
//! the value plus-or-minus that slack all round to the same `b64`, then the true value
//! does too, whatever it is, and the answer is certain. If they do not, the true value is
//! sitting near a halfway point, so the precision doubles and the question is asked
//! again.
//!
//! It terminates because a transcendental function is irrational at every rational
//! argument bar the ones named in each function below, so the true value is never exactly
//! a halfway point and enough precision always separates it from one.

use crate::Wide;

/// How much working width to start with beyond a `b64`'s fifty-three bits, and how far to
/// go before giving up. Nothing has been seen to need the second round, let alone the
/// fourth; the ceiling is there so that a mistake in a series shows up as a panic during
/// a test rather than as a program that never finishes.
const START: u64 = 96;
const CEILING: u64 = 4096;

/// `e^x`, correctly rounded.
pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x == f64::INFINITY {
        return f64::INFINITY;
    }
    if x == f64::NEG_INFINITY {
        return 0.0;
    }
    // The one exact case, and the reason `exp(0)` never reaches the series: it would
    // converge on one and never be *proved* to be one, because one is a halfway point
    // for nothing but is exactly representable and the slack would straddle it.
    if x == 0.0 {
        return 1.0;
    }
    // Past these the answer is not a `b64` at all, and the series would spend a long
    // time arriving at that.
    if x > 710.0 {
        return f64::INFINITY;
    }
    if x < -746.0 {
        return 0.0;
    }
    certain(|bits| exp_wide(&Wide::from_f64(x, bits), bits), 4)
}

/// The natural logarithm, correctly rounded.
pub fn ln(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x == f64::INFINITY {
        return f64::INFINITY;
    }
    // Exact, and for the same reason as `exp(0)`.
    if x == 1.0 {
        return 0.0;
    }
    certain(|bits| ln_wide(&Wide::from_f64(x, bits), bits), 5)
}

/// `x^y`, as `e^(y ln x)`, correctly rounded.
///
/// The exceptional cases come first and are IEEE's, not this file's: `pow(1, anything)`
/// is one and `pow(anything, 0)` is one, **including** when the other side is a
/// not-a-number, which is the rule everybody finds surprising and the standard is
/// nonetheless clear about.
pub fn pow(x: f64, y: f64) -> f64 {
    if x == 1.0 || y == 0.0 {
        return 1.0;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if y == f64::INFINITY {
        return match x.abs().partial_cmp(&1.0) {
            Some(std::cmp::Ordering::Greater) => f64::INFINITY,
            _ => 0.0,
        };
    }
    if y == f64::NEG_INFINITY {
        return match x.abs().partial_cmp(&1.0) {
            Some(std::cmp::Ordering::Greater) => 0.0,
            _ => f64::INFINITY,
        };
    }
    // A negative base is only defined for a whole exponent, and then the sign is the
    // exponent's parity. Everything else about it is the positive case.
    let whole = y.fract() == 0.0 && y.abs() < 9.007_199_254_740_992e15;
    if x < 0.0 && !whole {
        return f64::NAN;
    }
    if x == 0.0 {
        let negative = x.is_sign_negative() && whole && (y as i64) % 2 != 0;
        return if y < 0.0 {
            if negative { f64::NEG_INFINITY } else { f64::INFINITY }
        } else if negative {
            -0.0
        } else {
            0.0
        };
    }
    if x.is_infinite() {
        let negative = x < 0.0 && whole && (y as i64) % 2 != 0;
        return if (y > 0.0) == (x.abs() > 1.0) {
            if negative { f64::NEG_INFINITY } else { f64::INFINITY }
        } else if negative {
            -0.0
        } else {
            0.0
        };
    }

    // Where the answer is certainly outside a `b64` there is nothing for a series to
    // find out. `ln` at ordinary precision is far more accurate than the thousand-fold
    // margin being tested here, so this cannot mistake a representable answer for one
    // that is not.
    let rough = ln(x.abs()) * y;
    if rough > 1024.0 {
        return if x < 0.0 && whole && (y as i64) % 2 != 0 {
            f64::NEG_INFINITY
        } else {
            f64::INFINITY
        };
    }
    if rough < -1200.0 {
        return if x < 0.0 && whole && (y as i64) % 2 != 0 { -0.0 } else { 0.0 };
    }

    let flip = x < 0.0 && (y as i64) % 2 != 0;
    let answer = certain(
        |bits| {
            let logged = ln_wide(&Wide::from_f64(x.abs(), bits), bits);
            exp_wide(&logged.mul(&Wide::from_f64(y, bits)), bits)
        },
        // Two series and a multiplication between them, so the slack from the first is
        // carried through the second and multiplied by however large the exponent is.
        12,
    );
    if flip { -answer } else { answer }
}

/// Run the calculation wider and wider until the rounding is not in doubt.
///
/// `slack` is how many ulps of the working width the answer might be out by, counted
/// generously — being wrong about it costs another round and nothing else, while being
/// wrong the other way would cost correctness.
fn certain(mut compute: impl FnMut(u64) -> Wide, slack: i64) -> f64 {
    let mut bits = START;
    while bits <= CEILING {
        let value = compute(bits);
        if let Some(settled) = rounds_the_same_way(&value, slack) {
            return settled;
        }
        bits *= 2;
    }
    panic!("no precision up to {CEILING} bits settled the rounding");
}

/// The `b64` this rounds to, if everything within `slack` ulps of it rounds there too.
///
/// The interval is the honest part: the value could be anywhere in it, so an answer is
/// only given when the whole interval agrees. Where it does not, the true value is beside
/// a halfway point and more bits are the only way through.
fn rounds_the_same_way(value: &Wide, slack: i64) -> Option<f64> {
    let step = ulp(value);
    let wobble = step.mul(&Wide::whole(slack, value.bits()));
    let low = value.sub(&wobble).to_f64();
    let high = value.add(&wobble).to_f64();
    let middle = value.to_f64();
    // Bits rather than values, so that the two noughts are not mistaken for each other.
    if low.to_bits() == high.to_bits() && low.to_bits() == middle.to_bits() {
        return Some(middle);
    }
    None
}

/// One unit in the last place of the working width — how far apart two neighbouring
/// numbers are up here, which is what "within a few ulps" is counted in.
fn ulp(value: &Wide) -> Wide {
    if value.is_zero() {
        return Wide::from_f64(f64::MIN_POSITIVE, value.bits());
    }
    Wide::whole(1, value.bits()).mul(&value.abs()).scaled(-(value.bits() as i64) + 1)
}

/// `e^x` in the working width, by range reduction and then a Taylor series.
///
/// `x = k ln2 + r` with `|r| ≤ ln2 / 2`, so `e^x = 2^k e^r` and the series only ever sees
/// an argument below a half — where the terms fall away fast enough that a hundred bits
/// takes twenty-odd of them rather than hundreds.
fn exp_wide(x: &Wide, bits: u64) -> Wide {
    let work = bits + 32;
    let x = x.to_bits_of(work);
    let ln2 = ln_two(work);
    // `k` is the nearest whole number to `x / ln2`, worked out wide and read as an
    // integer, which it comfortably is: `|x|` is under 746 and `ln2` is near 0.69.
    let k = x.div(&ln2).to_f64().round() as i64;
    let r = x.sub(&Wide::whole(k, work).mul(&ln2));

    let mut term = Wide::whole(1, work);
    let mut sum = term.clone();
    for n in 1..=(work as i64) {
        term = term.mul(&r).div(&Wide::whole(n, work));
        if term.is_zero() {
            break;
        }
        let next = sum.add(&term);
        // The terms shrink; once one is too small to move the sum, the rest are too.
        if next == sum {
            break;
        }
        sum = next;
    }
    sum.scaled(k).to_bits_of(bits)
}

/// The natural logarithm in the working width.
///
/// `x = m 2^e` with `m` in `[1, 2)`, so `ln x = e ln2 + ln m`, and `ln m` is
/// `2 atanh((m-1)/(m+1))` — a series in a value below a third, which converges quickly
/// where the plain `ln(1+t)` series would crawl.
fn ln_wide(x: &Wide, bits: u64) -> Wide {
    let work = bits + 32;
    let x = x.to_bits_of(work);
    let (m, e) = split(&x, work);

    let one = Wide::whole(1, work);
    let t = m.sub(&one).div(&m.add(&one));
    let square = t.mul(&t);

    let mut power = t.clone();
    let mut sum = t;
    for n in 1..=(work as i64) {
        power = power.mul(&square);
        let term = power.div(&Wide::whole(2 * n + 1, work));
        if term.is_zero() {
            break;
        }
        let next = sum.add(&term);
        if next == sum {
            break;
        }
        sum = next;
    }
    let ln_m = sum.mul(&Wide::whole(2, work));
    ln_m.add(&Wide::whole(e, work).mul(&ln_two(work))).to_bits_of(bits)
}

/// `x` as `m × 2^e` with `m` in `[1, 2)`.
fn split(x: &Wide, bits: u64) -> (Wide, i64) {
    let one = Wide::whole(1, bits);
    let two = Wide::whole(2, bits);
    let mut m = x.abs();
    let mut e = 0i64;
    while m.cmp_abs(&two) != std::cmp::Ordering::Less {
        m = m.scaled(-1);
        e += 1;
    }
    while m.cmp_abs(&one) == std::cmp::Ordering::Less {
        m = m.scaled(1);
        e -= 1;
    }
    (m, e)
}

/// `ln 2`, to the working width, as `2 atanh(1/3)`.
///
/// Worked out rather than written down, because a constant copied from somewhere is a
/// constant nobody checked, and this one has to be right to however many bits the answer
/// turns out to need.
fn ln_two(bits: u64) -> Wide {
    let work = bits + 16;
    let third = Wide::whole(1, work).div(&Wide::whole(3, work));
    let square = third.mul(&third);
    let mut power = third.clone();
    let mut sum = third;
    for n in 1..=(work as i64) {
        power = power.mul(&square);
        let term = power.div(&Wide::whole(2 * n + 1, work));
        if term.is_zero() {
            break;
        }
        let next = sum.add(&term);
        if next == sum {
            break;
        }
        sum = next;
    }
    sum.mul(&Wide::whole(2, work)).to_bits_of(bits)
}

// Opened up for `tests/transcendence.rs`, which checks the series against digits
// somebody else published rather than against another run of the same series.
pub fn ln_two_for_tests(bits: u64) -> Wide {
    ln_two(bits)
}

pub fn exp_for_tests(x: f64, bits: u64) -> Wide {
    exp_wide(&Wide::from_f64(x, bits), bits)
}

pub fn ln_for_tests(x: f64, bits: u64) -> Wide {
    ln_wide(&Wide::from_f64(x, bits), bits)
}
