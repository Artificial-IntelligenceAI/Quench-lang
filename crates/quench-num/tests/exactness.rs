//! Whether `e` actually never rounds.
//!
//! The claims worth testing are not "addition works" but the ones the type exists to
//! make: that `0.1 + 0.2` is a tenth plus a fifth exactly, that a third is a third, and
//! that a value has one representation so equality means what it looks like.

use quench_num::{Big, Exact, Trouble};

fn e(text: &str) -> Exact {
    Exact::parse(text).unwrap_or_else(|| panic!("{text:?} should parse"))
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A rational with small parts, so a run of them can be checked against hand
    /// arithmetic without the numbers themselves being the thing under test.
    fn small(&mut self) -> Exact {
        let n = (self.next() % 200) as i64 - 100;
        let d = (self.next() % 99) as i64 + 1;
        Exact::ratio(Big::from_i64(n), Big::from_i64(d)).expect("d is never zero")
    }
}

#[test]
fn the_thing_the_type_exists_for() {
    // The number that made floating point famous.
    assert_eq!(e("0.1").add(&e("0.2")).to_string(), "3/10");
    assert_eq!(e("0.1").add(&e("0.2")), e("0.3"));

    // And a third, which no binary float has ever held.
    let third = e("1").div(&e("3")).unwrap();
    assert_eq!(third.to_string(), "1/3");
    assert_eq!(third.add(&third).add(&third), e("1"), "three thirds are one, exactly");
}

#[test]
fn every_value_has_one_representation() {
    assert_eq!(e("2/4"), e("1/2"));
    assert_eq!(e("-2/4"), e("1/-2"), "the sign ends up on the numerator either way");
    assert_eq!(e("100/10"), e("10"));
    assert_eq!(e("0/5"), e("0"));
    assert_eq!(e("0/5").to_string(), "0");
    assert_eq!(e("-0/5").to_string(), "0", "there is no negative zero");
    // Which is what makes equality a comparison rather than arithmetic.
    assert_eq!(e("6/8").to_string(), "3/4");
}

#[test]
fn a_whole_number_does_not_wear_a_denominator() {
    assert_eq!(e("12").to_string(), "12");
    assert_eq!(e("-12").to_string(), "-12");
    assert_eq!(e("24/2").to_string(), "12");
    assert!(e("24/2").is_whole());
    assert!(!e("1/2").is_whole());
}

#[test]
fn a_decimal_point_is_exact() {
    assert_eq!(e("0.5").to_string(), "1/2");
    assert_eq!(e("0.25").to_string(), "1/4");
    assert_eq!(e("-0.5").to_string(), "-1/2", "a sign on a zero whole part still counts");
    assert_eq!(e("1.5").to_string(), "3/2");
    assert_eq!(e("-1.5").to_string(), "-3/2");
    assert_eq!(e("2.000").to_string(), "2");
    // The b64 answer to 0.1 + 0.2, written out. It reduces by four -- which is lowest
    // terms working, and is why the expected string here is not the digits as typed.
    assert_eq!(
        e("0.30000000000000004").to_string(),
        "7500000000000001/25000000000000000"
    );
    assert_ne!(e("0.30000000000000004"), e("0.3"), "and it is still not three tenths");
}

#[test]
fn nonsense_is_not_a_number() {
    for text in ["", "1/", "/2", "1.", ".5", "1/2/3", "1.2.3", "a/b", "1 / 2"] {
        assert!(Exact::parse(text).is_none(), "{text:?} parsed");
    }
}

#[test]
fn nothing_is_divided_by_zero() {
    assert_eq!(e("1").div(&e("0")), Err(Trouble::DivideByZero));
    assert_eq!(e("0").reciprocal(), Err(Trouble::DivideByZero));
    assert_eq!(Exact::ratio(Big::from_u64(1), Big::zero()), Err(Trouble::DivideByZero));
}

#[test]
fn denominators_do_not_run_away() {
    // A thousand additions of fractions with nothing in common. Without reducing, the
    // denominator would be the product of every one of them; with it, this stays small
    // enough to finish, which is the whole reason lowest terms are kept.
    let mut total = Exact::zero();
    for i in 1..=1000u64 {
        total = total.add(&Exact::ratio(Big::from_u64(1), Big::from_u64(i)).unwrap());
    }
    // The 1000th harmonic number is a little over 7.485.
    assert!(total > e("7.485"), "{total}");
    assert!(total < e("7.486"), "{total}");
    assert!(!total.is_whole());
}

#[test]
fn the_laws_hold_whatever_the_numbers_are() {
    let mut rng = Rng(0xE7AC7);
    for round in 0..2000 {
        let (a, b, c) = (rng.small(), rng.small(), rng.small());

        assert_eq!(a.add(&b), b.add(&a), "round {round}: addition commutes");
        assert_eq!(a.add(&b).add(&c), a.add(&b.add(&c)), "round {round}: and associates");
        assert_eq!(a.mul(&b), b.mul(&a), "round {round}: multiplication commutes");
        assert_eq!(
            a.mul(&b.add(&c)),
            a.mul(&b).add(&a.mul(&c)),
            "round {round}: and distributes"
        );
        assert_eq!(a.add(&b).sub(&b), a, "round {round}: subtraction undoes addition");
        assert_eq!(a.add(&a.negated()), Exact::zero(), "round {round}");

        if !b.is_zero() {
            // The one that is only true because nothing rounds.
            assert_eq!(a.div(&b).unwrap().mul(&b), a, "round {round}: {a} / {b} * {b}");
            assert_eq!(b.reciprocal().unwrap().reciprocal().unwrap(), b, "round {round}");
        }
    }
}

#[test]
fn ordering_is_by_value_and_not_by_how_it_is_written() {
    let mut all = vec![e("1/2"), e("-3"), e("0"), e("2/4"), e("10/3"), e("-1/100")];
    all.sort();
    let order: Vec<String> = all.iter().map(|x| x.to_string()).collect();
    assert_eq!(order, ["-3", "-1/100", "0", "1/2", "1/2", "10/3"]);
}

#[test]
fn it_holds_numbers_nothing_else_can() {
    // 2^400 over 3, exactly, which no float of any width in the language can express.
    let mut power = Big::from_u64(1);
    for _ in 0..400 {
        power = power.mul(&Big::from_u64(2));
    }
    let value = Exact::ratio(power.clone(), Big::from_u64(3)).unwrap();
    assert!(!value.is_whole());
    assert_eq!(value.mul(&e("3")).numerator(), &power, "multiplying back is exact");
    assert_eq!(value.numerator().to_string().len(), 121, "2^400 has 121 digits");
}
