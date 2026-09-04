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

/// `sin`, correctly rounded.
pub fn sin(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    // The one exact value, and it keeps the sign it came with: `sin(-0)` is `-0`.
    if x == 0.0 {
        return x;
    }
    certain(|bits| circular(x, bits, false), 6)
}

/// `cos`, correctly rounded.
pub fn cos(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x == 0.0 {
        return 1.0;
    }
    certain(|bits| circular(x, bits, true), 6)
}

/// `tan`, correctly rounded.
pub fn tan(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    if x == 0.0 {
        return x;
    }
    certain(
        |bits| {
            let s = circular(x, bits + 32, false);
            let c = circular(x, bits + 32, true);
            s.div(&c).to_bits_of(bits)
        },
        // A division on top of two series, and near a pole the division magnifies
        // whatever error the cosine had.
        16,
    )
}

/// `atan`, correctly rounded.
pub fn atan(x: f64) -> f64 {
    if x.is_nan() {
        return f64::NAN;
    }
    if x == 0.0 {
        return x;
    }
    if x.is_infinite() {
        let half = certain(|bits| pi(bits).scaled(-1), 3);
        return if x > 0.0 { half } else { -half };
    }
    certain(|bits| arctan(&Wide::from_f64(x, bits + 32), bits + 32).to_bits_of(bits), 6)
}

/// `atan2`: the angle to the point, with the quadrant taken from both signs rather than
/// from the ratio, which is the whole reason it exists.
pub fn atan2(y: f64, x: f64) -> f64 {
    if y.is_nan() || x.is_nan() {
        return f64::NAN;
    }
    let quarter = |sign: f64| certain(|bits| pi(bits).scaled(-1), 3) * sign;
    if y == 0.0 {
        // A nought on top: the answer is nought or half a turn, and which one is the
        // *sign* of the bottom, not its size. Signed zero earns its keep here.
        return if x.is_sign_negative() {
            if y.is_sign_negative() { -std::f64::consts::PI } else { std::f64::consts::PI }
        } else {
            y
        };
    }
    if x == 0.0 {
        return quarter(if y > 0.0 { 1.0 } else { -1.0 });
    }
    if x.is_infinite() && y.is_infinite() {
        let eighth = certain(|bits| pi(bits).scaled(-2), 3);
        let turn = if x > 0.0 { eighth } else { eighth * 3.0 };
        return if y > 0.0 { turn } else { -turn };
    }
    if y.is_infinite() {
        return quarter(if y > 0.0 { 1.0 } else { -1.0 });
    }
    if x.is_infinite() {
        let half = certain(|bits| pi(bits), 3);
        return if x > 0.0 {
            if y > 0.0 { 0.0 } else { -0.0 }
        } else if y > 0.0 {
            half
        } else {
            -half
        };
    }
    certain(
        |bits| {
            let work = bits + 32;
            let ratio = Wide::from_f64(y, work).div(&Wide::from_f64(x, work));
            let angle = arctan(&ratio, work);
            if x > 0.0 {
                angle.to_bits_of(bits)
            } else if y > 0.0 {
                angle.add(&pi(work)).to_bits_of(bits)
            } else {
                angle.sub(&pi(work)).to_bits_of(bits)
            }
        },
        10,
    )
}

/// `sin` or `cos` at the working width, by taking the argument down into an eighth of a
/// turn and running the series there.
///
/// The reduction is where a library usually gives up: `x` can be enormous, and knowing
/// which eighth of a turn it lands in means knowing π to as many bits as `x` has
/// exponent. A library cannot afford that. This one keeps π in a `Big` and asks for as
/// many bits as it needs, so a sine of ten to the three hundredth is as exact as a sine
/// of a half.
fn circular(x: f64, bits: u64, cosine: bool) -> Wide {
    let above = (x.abs().log2().max(0.0) as u64) + 8;
    let work = bits + above + 32;
    let value = Wide::from_f64(x, work);
    let half_pi = pi(work).scaled(-1);

    // Which quarter turn it falls in, kept as a whole number of any size rather than
    // squeezed through an `f64`. That was the bug: `1e100 / (π/2)` needs three hundred
    // and thirty bits to name and an `f64` has fifty-three, so the quarter it landed in
    // was a guess and the sine of it was nonsense.
    //
    // The sign comes off first, because sine and cosine both know what to do with it and
    // reducing a positive number is one fewer thing to get wrong.
    let sign = value.is_negative();
    let size = value.abs();
    let half = Wide::whole(1, work).div(&Wide::whole(2, work));
    let quarters = size.div(&half_pi).add(&half).floor_abs();
    let k = i64::from(quarters.bit(1)) * 2 + i64::from(quarters.bit(0));
    let r = size.sub(&Wide::from_big(&quarters, work).mul(&half_pi));

    // `sin(-x)` is `-sin(x)` and `cos(-x)` is `cos(x)`, so the sign is put back at the
    // end rather than carried through the reduction.
    let flip = sign && !cosine;

    // Past a quarter turn the two swap and the signs go round: what is left is one
    // series on an argument no bigger than a quarter turn.
    let (want_cos, negate) = match (cosine, k) {
        (false, 0) => (false, false),
        (false, 1) => (true, false),
        (false, 2) => (false, true),
        (false, _) => (true, true),
        (true, 0) => (true, false),
        (true, 1) => (false, true),
        (true, 2) => (true, true),
        (true, _) => (false, false),
    };
    let answer = if want_cos { cos_series(&r, work) } else { sin_series(&r, work) };
    let answer = if negate != flip { answer.negated() } else { answer };
    answer.to_bits_of(bits)
}

/// `sin r` for a small `r`, by its Taylor series.
fn sin_series(r: &Wide, bits: u64) -> Wide {
    let square = r.mul(r);
    let mut term = r.clone();
    let mut sum = term.clone();
    for n in 1..=(bits as i64) {
        term = term.mul(&square).div(&Wide::whole(2 * n * (2 * n + 1), bits)).negated();
        let next = sum.add(&term);
        if term.is_zero() || next == sum {
            break;
        }
        sum = next;
    }
    sum
}

fn cos_series(r: &Wide, bits: u64) -> Wide {
    let square = r.mul(r);
    let mut term = Wide::whole(1, bits);
    let mut sum = term.clone();
    for n in 1..=(bits as i64) {
        term = term.mul(&square).div(&Wide::whole(2 * n * (2 * n - 1), bits)).negated();
        let next = sum.add(&term);
        if term.is_zero() || next == sum {
            break;
        }
        sum = next;
    }
    sum
}

/// `atan` at the working width.
///
/// The plain series crawls as the argument approaches one, so the argument is walked down
/// instead: `atan t = atan c + atan((t − c) / (1 + c t))` takes a fixed bite of `atan c`
/// out of the angle each time, and a sixteenth is small enough that what is left runs
/// fast and few enough bites that the walk is short.
fn arctan(x: &Wide, bits: u64) -> Wide {
    let negative = x.is_negative();
    let mut t = x.abs();
    let one = Wide::whole(1, bits);

    // Above one, turn it upside down: `atan t = π/2 − atan(1/t)`.
    let flipped = t.cmp_abs(&one) == std::cmp::Ordering::Greater;
    if flipped {
        t = one.div(&t);
    }

    let sixteenth = one.div(&Wide::whole(16, bits));
    let step = atan_series(&sixteenth, bits);
    let mut bites = 0i64;
    while t.cmp_abs(&sixteenth) == std::cmp::Ordering::Greater {
        t = t.sub(&sixteenth).div(&one.add(&sixteenth.mul(&t)));
        bites += 1;
    }
    let mut angle = atan_series(&t, bits).add(&Wide::whole(bites, bits).mul(&step));
    if flipped {
        angle = pi(bits).scaled(-1).sub(&angle);
    }
    if negative { angle.negated() } else { angle }
}

/// `atan t` for a small `t`, by its Taylor series.
fn atan_series(t: &Wide, bits: u64) -> Wide {
    let square = t.mul(t);
    let mut power = t.clone();
    let mut sum = t.clone();
    for n in 1..=(bits as i64) {
        power = power.mul(&square).negated();
        let term = power.div(&Wide::whole(2 * n + 1, bits));
        let next = sum.add(&term);
        if term.is_zero() || next == sum {
            break;
        }
        sum = next;
    }
    sum
}

/// π, by Machin: `π/4 = 4 atan(1/5) − atan(1/239)`.
///
/// Worked out rather than written down, for the same reason `ln 2` is: a constant copied
/// from somewhere is a constant nobody checked, and this one has to be right to however
/// many bits the argument turns out to need — which for a sine of a very large number is
/// a great many.
fn pi(bits: u64) -> Wide {
    let work = bits + 32;
    let one = Wide::whole(1, work);
    let fifth = one.div(&Wide::whole(5, work));
    let small = one.div(&Wide::whole(239, work));
    let quarter = atan_series(&fifth, work)
        .mul(&Wide::whole(4, work))
        .sub(&atan_series(&small, work));
    quarter.mul(&Wide::whole(4, work)).to_bits_of(bits)
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

pub fn pi_for_tests(bits: u64) -> Wide {
    pi(bits)
}

pub fn sin_for_tests(x: f64, bits: u64) -> Wide {
    circular(x, bits, false)
}

pub fn cos_for_tests(x: f64, bits: u64) -> Wide {
    circular(x, bits, true)
}

pub fn atan_for_tests(x: f64, bits: u64) -> Wide {
    arctan(&Wide::from_f64(x, bits), bits)
}
