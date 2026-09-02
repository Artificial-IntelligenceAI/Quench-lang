//! Whether `Big` is actually right, checked against something that already is.
//!
//! Where a value fits in a `u128`, the answer is compared with `u128` arithmetic. Where
//! it does not, the check is a law that must hold whatever the numbers are — `q·v + r`
//! is `u`, a gcd divides both of its arguments, dividing both by it leaves them coprime.
//! Laws are what can be tested at sizes nothing else can reach.

use quench_num::Big;

/// A deterministic scrambler, so a failure can be repeated exactly. Not for anything
/// that matters; entirely for producing a lot of different limbs.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A number of up to `limbs` limbs, sometimes with awkward all-ones limbs, because
    /// those are what make a quotient estimate need correcting.
    fn big(&mut self, limbs: usize) -> Big {
        let count = (self.next() as usize % limbs) + 1;
        let mut text = String::new();
        for _ in 0..count {
            let limb = match self.next() % 4 {
                0 => u64::MAX,
                1 => self.next() >> (self.next() % 63),
                _ => self.next(),
            };
            text.push_str(&limb.to_string());
        }
        Big::parse(&text).expect("digits are digits")
    }
}

fn n(text: &str) -> Big {
    Big::parse(text).expect("a number")
}

#[test]
fn zero_has_one_representation() {
    assert_eq!(Big::zero(), n("0"));
    assert_eq!(Big::zero(), n("-0"));
    assert_eq!(n("0").to_string(), "0");
    assert_eq!(n("-0").to_string(), "0", "there is no negative zero");
    assert!(!n("-0").is_negative());
    assert_eq!(Big::from_i64(0), Big::from_u64(0));
}

#[test]
fn text_survives_the_round_trip() {
    for text in [
        "1",
        "-1",
        "9",
        "10",
        "18446744073709551615",  // u64::MAX
        "18446744073709551616",  // one more, so two limbs
        "340282366920938463463374607431768211455", // u128::MAX
        "-340282366920938463463374607431768211456",
        "123456789012345678901234567890123456789012345678901234567890",
    ] {
        assert_eq!(n(text).to_string(), text, "{text}");
    }
}

#[test]
fn nonsense_is_not_a_number() {
    for text in ["", "-", "1.5", "0x10", " 1", "1 ", "12a", "--1", "+1"] {
        assert!(Big::parse(text).is_none(), "{text:?} parsed");
    }
}

#[test]
fn small_arithmetic_agrees_with_u128() {
    let mut rng = Rng(0x5EED);
    for _ in 0..2000 {
        let a = (rng.next() as u128) << 32 | rng.next() as u128 >> 32;
        let b = (rng.next() as u128) << 32 | rng.next() as u128 >> 32;
        let (x, y) = (n(&a.to_string()), n(&b.to_string()));

        assert_eq!(x.add(&y).to_string(), (a + b).to_string(), "{a} + {b}");
        // Only where u128 can still hold the answer. Beyond that the law-based tests
        // below are what checks multiplication, since there is nothing left to compare to.
        if let Some(product) = a.checked_mul(b) {
            assert_eq!(x.mul(&y).to_string(), product.to_string(), "{a} * {b}");
        }
        if a >= b {
            assert_eq!(x.sub(&y).to_string(), (a - b).to_string(), "{a} - {b}");
        }
        let (q, r) = x.div_rem(&y).expect("b is not zero");
        assert_eq!(q.to_string(), (a / b).to_string(), "{a} / {b}");
        assert_eq!(r.to_string(), (a % b).to_string(), "{a} % {b}");
    }
}

#[test]
fn signs_go_where_they_should() {
    // Truncating division, and a remainder that follows the dividend.
    let cases = [(7, 2, 3, 1), (-7, 2, -3, -1), (7, -2, -3, 1), (-7, -2, 3, -1)];
    for (a, b, wq, wr) in cases {
        let (q, r) = Big::from_i64(a).div_rem(&Big::from_i64(b)).unwrap();
        assert_eq!(q, Big::from_i64(wq), "{a} / {b}");
        assert_eq!(r, Big::from_i64(wr), "{a} % {b}");
    }
}

#[test]
fn dividing_by_nothing_is_nothing() {
    assert!(n("5").div_rem(&Big::zero()).is_none());
    assert!(Big::zero().div_rem(&Big::zero()).is_none());
}

#[test]
fn division_obeys_its_own_law_at_any_size() {
    // q·v + r = u, and |r| < |v|. This is the check that reaches sizes u128 cannot,
    // and the one that catches a quotient estimate corrected in the wrong direction.
    let mut rng = Rng(0xD1D1DE);
    for round in 0..3000 {
        let u = rng.big(6);
        let v = rng.big(3);
        if v.is_zero() {
            continue;
        }
        let (q, r) = u.div_rem(&v).expect("v is not zero");
        assert_eq!(q.mul(&v).add(&r), u, "round {round}: {u} / {v}");
        assert!(r.abs() < v.abs(), "round {round}: remainder {r} is not smaller than {v}");
    }
}

#[test]
fn the_estimate_needs_correcting_sometimes_and_still_comes_out_right() {
    // All-ones limbs are what make Knuth's two-limb estimate land one too high, so the
    // add-back path is exercised rather than merely written.
    let awkward = n("340282366920938463463374607431768211455"); // 2^128 - 1
    for scale in 1..40u32 {
        let u = awkward.mul(&n("10").pow_for_test(scale));
        let (q, r) = u.div_rem(&awkward).unwrap();
        assert_eq!(q.mul(&awkward).add(&r), u, "scale {scale}");
    }
}

#[test]
fn gcd_divides_both_and_leaves_them_coprime() {
    let mut rng = Rng(0x6CD);
    for round in 0..1500 {
        let a = rng.big(4);
        let b = rng.big(4);
        let g = Big::gcd(&a, &b);

        if a.is_zero() && b.is_zero() {
            assert!(g.is_zero());
            continue;
        }
        assert!(!g.is_negative(), "round {round}: a gcd is never negative");
        assert!(!g.is_zero(), "round {round}: gcd({a}, {b}) is zero");

        for x in [&a, &b] {
            let (q, r) = x.div_rem(&g).unwrap();
            assert!(r.is_zero(), "round {round}: {g} does not divide {x}");
            let _ = q;
        }
        // And it is the *greatest*: dividing both by it leaves nothing in common.
        let (a2, _) = a.div_rem(&g).unwrap();
        let (b2, _) = b.div_rem(&g).unwrap();
        assert_eq!(Big::gcd(&a2, &b2), Big::from_u64(1), "round {round}: gcd({a}, {b}) = {g} was not the greatest");
    }
}

#[test]
fn gcd_ignores_signs_and_handles_zero() {
    assert_eq!(Big::gcd(&n("-12"), &n("18")), n("6"));
    assert_eq!(Big::gcd(&n("12"), &n("-18")), n("6"));
    assert_eq!(Big::gcd(&n("0"), &n("7")), n("7"));
    assert_eq!(Big::gcd(&n("7"), &n("0")), n("7"));
    assert_eq!(Big::gcd(&Big::zero(), &Big::zero()), Big::zero());
    assert_eq!(Big::gcd(&n("17"), &n("13")), n("1"), "coprime");
}

#[test]
fn a_thousand_factorial_is_right_at_both_ends() {
    let mut f = Big::from_u64(1);
    for i in 2..=1000u64 {
        f = f.mul(&Big::from_u64(i));
    }
    let text = f.to_string();
    assert!(text.starts_with("402387260077093773543702433923"), "{}", &text[..40]);
    // 1000! ends in 249 zeros: floor(1000/5) + floor(1000/25) + ... = 249.
    assert_eq!(text.len() - text.trim_end_matches('0').len(), 249);
    assert_eq!(text.len(), 2568, "1000! has 2568 digits");
}

#[test]
fn ordering_puts_negatives_where_they_belong() {
    let mut all = vec![n("-100"), n("5"), n("0"), n("-1"), n("100"), n("-99999999999999999999")];
    all.sort();
    let order: Vec<String> = all.iter().map(|b| b.to_string()).collect();
    assert_eq!(order, ["-99999999999999999999", "-100", "-1", "0", "5", "100"]);
}

/// Only for the tests: repeated multiplication, which is all they need.
trait PowForTest {
    fn pow_for_test(&self, exponent: u32) -> Big;
}

impl PowForTest for Big {
    fn pow_for_test(&self, exponent: u32) -> Big {
        let mut out = Big::from_u64(1);
        for _ in 0..exponent {
            out = out.mul(self);
        }
        out
    }
}
