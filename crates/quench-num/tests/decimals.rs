//! Whether `d32` and `d64` round where IEEE 754 says to round.
//!
//! The claims worth testing are the ones the type exists to make: that a tenth is a
//! tenth, that a written value keeps the precision it was written with, and that the
//! four exceptional answers -- infinity, not-a-number, overflow and underflow -- happen
//! at the exponents the standard names and not one step either side.

use quench_num::{Decimal, Format, D32, D64};

fn d(text: &str, format: Format) -> Decimal {
    Decimal::parse(text, format).unwrap_or_else(|| panic!("{text:?} should parse"))
}

/// One operation, written the way the decimal arithmetic specification writes its own
/// test cases: two operands and the text of the answer.
fn wide(a: &str, op: char, b: &str) -> String {
    let (x, y) = (d(a, D64), d(b, D64));
    match op {
        '+' => x.add(&y, D64),
        '-' => x.sub(&y, D64),
        'x' => x.mul(&y, D64),
        _ => x.div(&y, D64),
    }
    .to_string()
}

fn narrow(a: &str, op: char, b: &str) -> String {
    let (x, y) = (d(a, D32), d(b, D32));
    match op {
        '+' => x.add(&y, D32),
        '-' => x.sub(&y, D32),
        'x' => x.mul(&y, D32),
        _ => x.div(&y, D32),
    }
    .to_string()
}

#[test]
fn a_tenth_is_a_tenth() {
    // The sum every binary float is famous for. Nothing clever here -- a tenth is
    // exactly representable in base ten, so this is what "rounds in the base it was
    // written in" buys and the whole reason the type exists.
    assert_eq!(wide("0.1", '+', "0.2"), "0.3");
    assert_eq!(wide("0.1", '+', "0.7"), "0.8");
    assert_eq!(wide("1.1", '-', "1.0"), "0.1");
}

#[test]
fn the_answer_keeps_the_exponent_the_operation_asks_for() {
    // Addition and subtraction take the smaller exponent of the two, multiplication
    // takes the sum, and an exact division walks back to the difference. That is what
    // makes `2.50 + 1.00` end in a zero and `2.5 / 1` not grow one.
    assert_eq!(wide("2.50", '+', "1.00"), "3.50");
    assert_eq!(wide("2.5", '+', "1"), "3.5");
    assert_eq!(wide("1.005", 'x', "100"), "100.500");
    assert_eq!(wide("2.5", '/', "1"), "2.5");
    assert_eq!(wide("1", '/', "2"), "0.5");
    assert_eq!(wide("1", '/', "8"), "0.125");
    // Nought carries an exponent like anything else, and the operation decides it.
    assert_eq!(wide("0.00", '+', "0.0"), "0.00");
}

#[test]
fn a_division_that_does_not_come_out_fills_the_digits_it_has() {
    assert_eq!(wide("1", '/', "3"), "0.3333333333333333");
    assert_eq!(narrow("1", '/', "3"), "0.3333333");
    // And a third times three is not one, which is the difference from `e` and the
    // reason both types exist.
    let third = d("1", D64).div(&d("3", D64), D64);
    assert_eq!(third.mul(&d("3", D64), D64).to_string(), "0.9999999999999999");
}

#[test]
fn rounding_is_half_even_and_the_sticky_digit_counts() {
    // Ties go to the even digit; anything at all beyond the tie goes up. `12345675`
    // and `12345685` both sit exactly halfway and land on the even one -- in opposite
    // directions, which is the whole point of the rule.
    assert_eq!(narrow("12345678", '+', "0"), "1.234568E+7");
    assert_eq!(narrow("12345675", '+', "0"), "1.234568E+7");
    assert_eq!(narrow("12345685", '+', "0"), "1.234568E+7");
    assert_eq!(narrow("12345665", '+', "0"), "1.234566E+7");
    // Not a tie: a single digit past halfway rounds up on its own. Both sides fit in
    // seven digits, so what is rounded is the *sum* -- reading `12345665` under a `d32`
    // would round it before the addition ever saw it.
    assert_eq!(narrow("1234566", '+', "0.5"), "1234566");
    assert_eq!(narrow("1234566", '+', "0.5001"), "1234567");
    assert_eq!(narrow("1234567", '+', "0.5"), "1234568");
}

#[test]
fn a_number_too_small_to_write_rounds_to_the_smallest_exponent() {
    // `Etiny` for a `d64` is -398, and everything below it lands there rather than
    // pretending to a precision the format cannot hold.
    assert_eq!(d("1E-400", D64).add(&d("0", D64), D64).to_string(), "0E-398");
    assert_eq!(d("1E-101", D32).add(&d("0", D32), D32).to_string(), "1E-101");
    assert_eq!(d("1E-102", D32).add(&d("0", D32), D32).to_string(), "0E-101");
}

#[test]
fn a_number_too_large_to_write_is_infinity() {
    // `Emax` for a `d64` is 384, said as an adjusted exponent -- so the boundary is
    // between a number with 385 digits before its point and one with 384.
    assert_eq!(wide("9E384", 'x', "10"), "infinity");
    assert_eq!(wide("-9E384", 'x', "10"), "-infinity");
    assert_eq!(narrow("9E96", 'x', "10"), "infinity");
    // And one step under it is an ordinary number. Multiplication's exponent is the
    // sum of the two, so `9E383` times `10` keeps the zero that `10` brought.
    assert_eq!(wide("9E383", 'x', "10"), "9.0E+384");
}

#[test]
fn dividing_by_nought_is_an_answer_rather_than_a_stop() {
    assert_eq!(wide("1", '/', "0"), "infinity");
    assert_eq!(wide("-1", '/', "0"), "-infinity");
    assert_eq!(wide("0", '/', "0"), "not-a-number");
    // Infinity carries through arithmetic the way IEEE says, including the one
    // subtraction that has no answer.
    let big = d("1", D64).div(&d("0", D64), D64);
    assert_eq!(big.add(&d("1", D64), D64).to_string(), "infinity");
    assert_eq!(big.sub(&big, D64).to_string(), "not-a-number");
    assert_eq!(big.mul(&d("0", D64), D64).to_string(), "not-a-number");
}

#[test]
fn a_not_a_number_compares_as_none_of_the_three() {
    let none = d("0", D64).div(&d("0", D64), D64);
    assert!(none.compare(&d("1", D64)).is_none());
    assert!(none.compare(&none).is_none());
    // Which is the fourth answer the lowering has to carry, and the reason `<==` is
    // not one comparison against one number.
    assert_eq!(d("1", D64).compare(&d("2", D64)), Some(std::cmp::Ordering::Less));
}

#[test]
fn two_ways_of_writing_one_number_compare_equal() {
    // A cohort is kept by arithmetic and ignored by comparison, which are two different
    // questions: what precision was this worked out to, and what number is it.
    assert_eq!(d("2.50", D64).compare(&d("2.5", D64)), Some(std::cmp::Ordering::Equal));
    assert_eq!(d("0.0", D64).compare(&d("0", D64)), Some(std::cmp::Ordering::Equal));
    // Including the two noughts, which are one number and not one written value.
    assert_eq!(d("-0", D64).compare(&d("0", D64)), Some(std::cmp::Ordering::Equal));
    assert_eq!(d("-0", D64).to_string(), "-0");
    assert_eq!(d("1E+3", D64).compare(&d("1000", D64)), Some(std::cmp::Ordering::Equal));
}

#[test]
fn a_number_is_written_back_the_way_the_standard_writes_it() {
    // `to-scientific-string`: plain notation while the adjusted exponent is at least
    // -6, and exponential past that, so a reader never counts more than six zeros.
    for (written, shown) in [
        ("0.000001", "0.000001"),
        ("0.0000001", "1E-7"),
        ("1234567890123456789", "1.234567890123457E+18"),
        ("0", "0"),
        ("0.00", "0.00"),
        ("-7.5", "-7.5"),
        ("1E+3", "1E+3"),
    ] {
        assert_eq!(d(written, D64).to_string(), shown, "{written}");
    }
    // The two that are shown and cannot be written: they are answers a program reaches,
    // the same rule a `b64` keeps.
    assert_eq!(d("1", D64).div(&d("0", D64), D64).to_string(), "infinity");
    assert_eq!(d("0", D64).div(&d("0", D64), D64).to_string(), "not-a-number");
}

#[test]
fn what_is_not_a_number_at_all_is_refused() {
    // A ratio is how an `e` is written, and the two are not the same type. Everything
    // else here is the checker's business, and it asks this same question.
    for text in ["1/3", "", "1.2.3", "hello", "1e", "--1", "0x10", "Infinity", "NaN"] {
        assert!(Decimal::parse(text, D64).is_none(), "{text:?} should not parse");
    }
}
