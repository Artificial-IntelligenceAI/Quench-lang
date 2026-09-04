//! Whether [`Wide`] is a float at all, before anything is built on it.
//!
//! Everything the transcendentals will claim rests on this: that a `b64` goes in
//! exactly, that arithmetic at a chosen width is correctly rounded to that width, and
//! that coming back out rounds once and to the nearest. If any of that is wrong then
//! every function above it is wrong in a way no amount of testing the function will
//! find, so it is checked here on its own.

use quench_num::Wide;

/// A `b64` in and straight back out is the same bits, whatever the working width — a
/// float is a whole number times a power of two, and that is exactly what `Wide` holds.
#[test]
fn a_b64_goes_in_and_comes_back_unchanged() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for bits in [53u64, 64, 128, 300] {
        for _ in 0..2_000 {
            let x = rng.finite();
            let there_and_back = Wide::from_f64(x, bits).to_f64();
            assert_eq!(
                there_and_back.to_bits(),
                x.to_bits(),
                "{x} at {bits} bits came back as {there_and_back}"
            );
        }
        for named in [
            0.0, -0.0, 1.0, -1.0, 0.5, 2.0, f64::MIN_POSITIVE, f64::MAX, -f64::MAX,
            5e-324, 1e308, -1e-308, 0.1, 1.0 / 3.0,
        ] {
            assert_eq!(Wide::from_f64(named, bits).to_f64().to_bits(), named.to_bits(), "{named}");
        }
    }
}

/// Arithmetic wide enough to be exact must give exactly what the hardware gives, because
/// then both are the same sum with no rounding in it anywhere.
#[test]
fn exact_arithmetic_agrees_with_the_hardware() {
    let mut rng = Rng(7);
    for _ in 0..5_000 {
        let (a, b) = (rng.modest(), rng.modest());
        let (wa, wb) = (Wide::from_f64(a, 200), Wide::from_f64(b, 200));
        // A product of two 53-bit numbers needs 106 bits, and 200 is more than that, so
        // nothing rounds and the answer is the one true product.
        let product = wa.mul(&wb).to_f64();
        assert_eq!(product, a * b, "{a} × {b}");
        // A sum of two modest numbers is exact at this width too.
        assert_eq!(wa.add(&wb).to_f64(), a + b, "{a} + {b}");
        assert_eq!(wa.sub(&wb).to_f64(), a - b, "{a} - {b}");
    }
}

/// Division rounds, so it is checked the other way: multiply the answer back and see
/// that it lands within half a step of what went in.
#[test]
fn division_is_correctly_rounded() {
    let mut rng = Rng(11);
    for _ in 0..5_000 {
        let (a, b) = (rng.modest(), rng.modest());
        if b == 0.0 {
            continue;
        }
        let got = Wide::from_f64(a, 200).div(&Wide::from_f64(b, 200)).to_f64();
        // At two hundred bits the quotient is far more accurate than a `b64`, so
        // narrowing it must land on the same `b64` the hardware's own division does.
        assert_eq!(got, a / b, "{a} / {b}");
    }
}

/// Rounding to a width is to the nearest, ties to even — the rule the hardware uses,
/// applied where the hardware cannot reach.
#[test]
fn narrowing_rounds_to_nearest_with_ties_to_even() {
    // Three bits: 1.00, 1.01, 1.10, 1.11 and up. A four-bit value exactly between two
    // of them must land on the one whose last bit is nought.
    let four = |n: i64| Wide::whole(n, 60).to_bits_of(3);
    assert_eq!(four(9).to_f64(), 8.0, "1001 is a tie and 1000 is the even one");
    assert_eq!(four(11).to_f64(), 12.0, "1011 is a tie and 1100 is the even one");
    assert_eq!(four(10).to_f64(), 10.0, "1010 fits in three bits already");
    assert_eq!(four(13).to_f64(), 12.0, "1101 is nearer 1100 than 1110");

    // And a carry out of the top, which is the case a shift written by hand gets wrong.
    assert_eq!(Wide::whole(15, 60).to_bits_of(3).to_f64(), 16.0, "1111 rounds up to 10000");
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Any finite `b64` at all, including subnormals, because those are where a
    /// conversion written by hand comes apart.
    fn finite(&mut self) -> f64 {
        loop {
            let x = f64::from_bits(self.next());
            if x.is_finite() {
                return x;
            }
        }
    }

    /// One with a modest exponent, so that a sum or product of two stays finite and the
    /// comparison against the hardware means something.
    fn modest(&mut self) -> f64 {
        let n = self.next();
        let sign = if n & 1 == 0 { 1.0 } else { -1.0 };
        sign * ((n >> 11) as f64) / 4_194_304.0
    }
}

/// A square root wide enough to be exact must land on the same `b64` the hardware does,
/// because the hardware's is *required* to be correctly rounded and so is this.
#[test]
fn square_root_agrees_with_the_hardware() {
    let mut rng = Rng(0x_5417);
    for _ in 0..5_000 {
        let x = rng.modest().abs();
        let got = Wide::from_f64(x, 200).sqrt().to_f64();
        assert_eq!(got, x.sqrt(), "sqrt({x})");
    }
    for named in [0.0, 1.0, 2.0, 4.0, 0.25, 1e300, 1e-300, f64::MIN_POSITIVE] {
        assert_eq!(Wide::from_f64(named, 200).sqrt().to_f64(), named.sqrt(), "sqrt({named})");
    }
    // And squaring it back lands on the value again where the root was exact.
    for n in 1..500u32 {
        let square = f64::from(n * n);
        assert_eq!(Wide::from_f64(square, 200).sqrt().to_f64(), f64::from(n), "sqrt({square})");
    }
}
