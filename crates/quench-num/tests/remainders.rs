//! `remainder` against arithmetic that does not round.
//!
//! Its answer is exact — `x − y × n` is representable in the same format as `x`, always
//! — which means it can be *checked* rather than trusted. `Exact` holds any float
//! exactly, because a float is a whole number over a power of two, and rationals never
//! round. So the reference here is not another float implementation with the same blind
//! spots: it is the definition, worked out in a place where being wrong is impossible.
//!
//! This matters more than the usual test because a maths function that is subtly wrong
//! is wrong in *both* engines at once — they call the same code — and the oracle
//! compares engines against each other. It has nothing to compare a shared wrong answer
//! against. This does.

use quench_num::{maths, Big, Exact, Paired};

/// A float as the rational it exactly is.
fn exactly(x: f64) -> Exact {
    let (mantissa, exponent, sign) = decompose(x);
    let top = Big::from_u64(mantissa);
    let top = if sign < 0 { top.negated() } else { top };
    let two = Big::from_u64(2);
    let mut scale = Big::from_u64(1);
    for _ in 0..exponent.unsigned_abs() {
        scale = scale.mul(&two);
    }
    if exponent >= 0 {
        Exact::whole(top.mul(&scale))
    } else {
        Exact::ratio(top, scale).expect("a power of two is not nought")
    }
}

/// `x = mantissa × 2^exponent × sign`, exactly, which is what a binary float is.
fn decompose(x: f64) -> (u64, i32, i8) {
    let bits = x.to_bits();
    let sign = if bits >> 63 == 1 { -1i8 } else { 1 };
    let raw = ((bits >> 52) & 0x7FF) as i32;
    let fraction = bits & 0x000F_FFFF_FFFF_FFFF;
    if raw == 0 {
        (fraction, -1074, sign)
    } else {
        (fraction | 0x0010_0000_0000_0000, raw - 1075, sign)
    }
}

/// The definition: `x − y × n`, with `n` the nearest integer to `x / y`, ties to even.
/// Worked out in rationals, so nothing rounds and nothing overflows.
fn by_definition(x: f64, y: f64) -> f64 {
    let (a, b) = (exactly(x), exactly(y));
    let quotient = a.div(&b).expect("y is not nought");
    let n = nearest_even(&quotient);
    let answer = a.sub(&b.mul(&n));
    // Exact by construction, so this conversion cannot round: it is only being read
    // back into the format it was always representable in.
    read_back(&answer)
}

/// The integer nearest a rational, with a tie going to the even one.
fn nearest_even(q: &Exact) -> Exact {
    let (down, rest) = q.numerator().div_rem(q.denominator()).expect("not nought");
    let negative = q.is_negative();
    // `div_rem` truncates toward zero, so the floor is one lower when it went the other
    // way and left something behind.
    let floor = if negative && !rest.is_zero() {
        down.sub(&Big::from_u64(1))
    } else {
        down
    };
    let twice_rest = Exact::whole(floor.clone())
        .sub(q)
        .abs()
        .mul(&Exact::whole(Big::from_u64(2)));
    let one = Exact::one();
    let up = floor.add(&Big::from_u64(1));
    match twice_rest.cmp(&one) {
        std::cmp::Ordering::Less => Exact::whole(floor),
        std::cmp::Ordering::Greater => Exact::whole(up),
        // A tie: whichever of the two is even.
        std::cmp::Ordering::Equal => {
            let two = Big::from_u64(2);
            let (_, left) = floor.div_rem(&two).expect("two is not nought");
            if left.is_zero() { Exact::whole(floor) } else { Exact::whole(up) }
        }
    }
}

/// A rational that is exactly a float, read back as one.
fn read_back(value: &Exact) -> f64 {
    // Through its own decimal text, because a value this size and shape round-trips: the
    // rational is a small whole number over a power of two, and Rust's parser is
    // correctly rounded.
    let text = format!("{value}");
    if let Some((top, bottom)) = text.split_once('/') {
        let top: f64 = top.parse().expect("a whole number");
        let bottom: f64 = bottom.parse().expect("a whole number");
        return top / bottom;
    }
    text.parse().expect("a whole number")
}

fn remainder(x: f64, y: f64) -> f64 {
    maths::paired64(Paired::Remainder, x, y)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A float with a modest exponent, so that the rational reference stays small enough
    /// to work with and the interesting cases still turn up.
    fn modest(&mut self) -> f64 {
        let n = self.next();
        let sign = if n & 1 == 0 { 1.0 } else { -1.0 };
        let whole = ((n >> 8) % 10_000) as f64;
        let part = ((n >> 32) % 1_000) as f64 / 1_000.0;
        sign * (whole + part)
    }
}

#[test]
fn the_named_cases_are_what_the_standard_says() {
    // `%` would give 1 for the first of these, because it takes the quotient toward
    // zero. `remainder` takes it to the nearest, so the answer is negative.
    assert_eq!(remainder(5.0, 2.0), 1.0);
    assert_eq!(remainder(7.0, 2.0), -1.0, "the quotient is 4, not 3");
    assert_eq!(remainder(5.0, 3.0), -1.0);
    assert_eq!(remainder(-5.0, 3.0), 1.0);
    assert_eq!(remainder(1.0, 1.0), 0.0);

    // A tie goes to the even quotient, which is the whole reason this is not `%`.
    assert_eq!(remainder(3.0, 2.0), -1.0, "quotient 2 rather than 1");
    assert_eq!(remainder(1.0, 2.0), 1.0, "quotient 0 rather than 1");
    assert_eq!(remainder(5.0, 2.0), 1.0, "quotient 2 rather than 3");
}

#[test]
fn the_edges_answer_the_way_the_standard_says() {
    assert!(remainder(1.0, 0.0).is_nan(), "a divisor of nought has no answer");
    assert!(remainder(f64::INFINITY, 2.0).is_nan());
    assert!(remainder(f64::NAN, 2.0).is_nan());
    assert!(remainder(2.0, f64::NAN).is_nan());
    assert_eq!(remainder(2.5, f64::INFINITY), 2.5, "everything is left over");
    assert_eq!(remainder(0.0, 3.0), 0.0);
}

#[test]
fn every_answer_matches_arithmetic_that_does_not_round() {
    // The claim `remainder` makes is that its answer is exact. Rationals are where that
    // can be checked, because nothing in them rounds.
    let mut rng = Rng(0x2026_0904);
    let mut checked = 0;
    for _ in 0..20_000 {
        let (x, y) = (rng.modest(), rng.modest());
        if y == 0.0 {
            continue;
        }
        let got = remainder(x, y);
        let want = by_definition(x, y);
        assert_eq!(
            got.to_bits(),
            want.to_bits(),
            "remainder({x}, {y}) was {got} and the definition says {want}"
        );
        // And the property the standard promises, which is what it is for.
        assert!(
            got.abs() * 2.0 <= y.abs() * (1.0 + f64::EPSILON),
            "remainder({x}, {y}) was {got}, which is more than half of {y}"
        );
        checked += 1;
    }
    assert!(checked > 19_000, "only {checked} pairs were usable");
}
