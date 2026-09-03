//! `e` — an exact rational.
//!
//! A numerator over a denominator, both unbounded, always in lowest terms, with the sign
//! on the numerator and the denominator strictly positive. There is one representation
//! of every value, so two of these are equal exactly when they are the same number.
//!
//! Nothing here rounds and nothing here overflows. That is the point of the type:
//! `0.1 + 0.2` is `3/10` rather than `0.30000000000000004`, and a third is a third
//! rather than the nearest `b64` to it.
//!
//! # One value, not three
//!
//! An `e` is one thing holding two magnitudes, not a pair of pointers at two separately
//! allocated integers. Addition renews both parts and then reduces, so there is nothing
//! left to share; assignment shares the whole value anyway; and large arithmetic is
//! bandwidth-bound, so keeping both magnitudes together is what the cache wants. It also
//! leaves an `e` a leaf as far as the collector is concerned — no references inside,
//! nothing anywhere in Quench to trace.
//!
//! # Lowest terms, every time
//!
//! Reducing after every operation costs a gcd and buys a canonical form: equality is a
//! comparison rather than cross-multiplication, and a value printed twice looks the same
//! both times. Whether that trade still holds when the numbers are the *point* is the
//! open question in `notes/e-is-big-and-exact.md` — it is why the gcd underneath is
//! binary rather than Euclid, so that the cost of being canonical is as small as it can
//! be made before the policy itself is reconsidered.

use crate::big::Big;
use std::cmp::Ordering;

/// An exact rational.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Exact {
    /// Carries the sign.
    numerator: Big,
    /// Never zero, never negative.
    denominator: Big,
}

/// What went wrong, when something did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trouble {
    /// A denominator of zero. There is no value to give back and no rounding to hide
    /// behind, so this stops rather than producing an infinity `e` cannot represent.
    DivideByZero,
}

impl Exact {
    /// A whole number.
    pub fn whole(n: Big) -> Exact {
        Exact { numerator: n, denominator: Big::from_u64(1) }
    }

    pub fn zero() -> Exact {
        Exact::whole(Big::zero())
    }

    pub fn one() -> Exact {
        Exact::whole(Big::from_u64(1))
    }

    /// A numerator over a denominator, reduced and with its signs sorted out.
    pub fn ratio(numerator: Big, denominator: Big) -> Result<Exact, Trouble> {
        if denominator.is_zero() {
            return Err(Trouble::DivideByZero);
        }
        let negative = numerator.is_negative() != denominator.is_negative();
        let (mut n, mut d) = (numerator.abs(), denominator.abs());

        let g = Big::gcd(&n, &d);
        if g != Big::from_u64(1) {
            n = n.div_rem(&g).expect("a gcd is never zero here").0;
            d = d.div_rem(&g).expect("a gcd is never zero here").0;
        }

        Ok(Exact { numerator: if negative { n.negated() } else { n }, denominator: d })
    }

    pub fn numerator(&self) -> &Big {
        &self.numerator
    }

    pub fn denominator(&self) -> &Big {
        &self.denominator
    }

    pub fn is_zero(&self) -> bool {
        self.numerator.is_zero()
    }

    pub fn is_negative(&self) -> bool {
        self.numerator.is_negative()
    }

    /// Whether this is a whole number, which is to say its denominator is one.
    pub fn is_whole(&self) -> bool {
        self.denominator == Big::from_u64(1)
    }

    pub fn negated(&self) -> Exact {
        Exact { numerator: self.numerator.negated(), denominator: self.denominator.clone() }
    }

    pub fn abs(&self) -> Exact {
        Exact { numerator: self.numerator.abs(), denominator: self.denominator.clone() }
    }

    /// `1/x`. Fails on zero, which has no reciprocal.
    pub fn reciprocal(&self) -> Result<Exact, Trouble> {
        Exact::ratio(self.denominator.clone(), self.numerator.clone())
    }

    pub fn add(&self, other: &Exact) -> Exact {
        // a/b + c/d = (a·d + c·b) div (b·d). Reducing afterwards is what keeps the
        // denominator from growing without bound over a long run of additions.
        let numerator = self
            .numerator
            .mul(&other.denominator)
            .add(&other.numerator.mul(&self.denominator));
        let denominator = self.denominator.mul(&other.denominator);
        Exact::ratio(numerator, denominator).expect("neither denominator was zero")
    }

    pub fn sub(&self, other: &Exact) -> Exact {
        self.add(&other.negated())
    }

    pub fn mul(&self, other: &Exact) -> Exact {
        Exact::ratio(
            self.numerator.mul(&other.numerator),
            self.denominator.mul(&other.denominator),
        )
        .expect("neither denominator was zero")
    }

    /// Exact division. This is what separates rationals from big integers: a third is a
    /// third, not the nearest thing to it.
    pub fn div(&self, other: &Exact) -> Result<Exact, Trouble> {
        Exact::ratio(
            self.numerator.mul(&other.denominator),
            self.denominator.mul(&other.numerator),
        )
    }

    /// Read `12`, `-3/4`, or `0.1`.
    ///
    /// A decimal point is exact here, which is the whole reason to write one: `0.1` is
    /// one tenth, and not the `b64` nearest to it.
    pub fn parse(text: &str) -> Option<Exact> {
        if let Some((top, bottom)) = text.split_once('/') {
            return Exact::ratio(Big::parse(top)?, Big::parse(bottom)?).ok();
        }
        if let Some((whole, fraction)) = text.split_once('.') {
            // `-0.5` has its sign on a whole part that is zero, so the sign is taken
            // from the text rather than from the number it parsed to.
            let negative = whole.starts_with('-');
            let whole_part = if whole == "-" { Big::zero() } else { Big::parse(whole)? };
            if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let scale = Big::parse(&format!("1{}", "0".repeat(fraction.len())))?;
            let magnitude =
                whole_part.abs().mul(&scale).add(&Big::parse(fraction)?);
            let numerator = if negative { magnitude.negated() } else { magnitude };
            return Exact::ratio(numerator, scale).ok();
        }
        Some(Exact::whole(Big::parse(text)?))
    }
}

impl std::fmt::Display for Exact {
    /// `12`, or `-3/4`. A whole number does not wear a denominator of one.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_whole() {
            write!(f, "{}", self.numerator)
        } else {
            write!(f, "{}/{}", self.numerator, self.denominator)
        }
    }
}

impl Ord for Exact {
    fn cmp(&self, other: &Exact) -> Ordering {
        // Cross-multiply. Both denominators are positive, so nothing flips.
        self.numerator
            .mul(&other.denominator)
            .cmp(&other.numerator.mul(&self.denominator))
    }
}

impl PartialOrd for Exact {
    fn partial_cmp(&self, other: &Exact) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
