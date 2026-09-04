//! Whether `exp`, `ln` and `pow` are right, checked against something that is not them.
//!
//! A series can be self-consistently wrong: run it at more bits and it converges, neatly,
//! on the wrong number. So the first thing here is not a comparison against another
//! implementation of the same thing but against **published digits** — `ln 2` and `e` are
//! known to hundreds of places, and forty of them is far past anything a `b64` could
//! hide an error in.
//!
//! Once the series are known to be right, the rest follows: an answer worked out at three
//! hundred bits is nearer the truth than a `b64` can measure, so whichever `b64` it
//! rounds to is the correctly rounded one, and that is the standard the platform's own
//! library is held to here rather than the other way round.

use quench_num::{transcend, Big, Wide};

/// `floor(|value| × 10^places)`, for reading digits off a wide value.
fn digits(value: &Wide, places: u32) -> Big {
    let ten = Wide::whole(10, value.bits());
    let mut scaled = value.abs();
    for _ in 0..places {
        scaled = scaled.mul(&ten);
    }
    scaled.floor_abs()
}

fn published(text: &str) -> Big {
    Big::parse(text).expect("a whole number")
}

#[test]
fn ln_two_matches_the_digits_everybody_else_publishes() {
    // 0.693147180559945309417232121458176568075500134360255254120680...
    let want = published("6931471805599453094172321214581765680755");
    let got = digits(&transcend::ln_two_for_tests(300), 40);
    assert_eq!(got, want, "forty digits of ln 2");
}

#[test]
fn e_matches_the_digits_everybody_else_publishes() {
    // 2.718281828459045235360287471352662497757247093699959574966967...
    let want = published("27182818284590452353602874713526624977572");
    let got = digits(&transcend::exp_for_tests(1.0, 300), 40);
    assert_eq!(got, want, "forty-one digits of e");
}

#[test]
fn ln_ten_matches_too() {
    // 2.302585092994045684017991454684364207601101488628772976033327...
    let want = published("23025850929940456840179914546843642076011");
    let got = digits(&transcend::ln_for_tests(10.0, 300), 40);
    assert_eq!(got, want, "forty-one digits of ln 10");
}

#[test]
fn pi_matches_the_digits_everybody_else_publishes() {
    // 3.14159265358979323846264338327950288419716939937510582097494...
    let want = published("31415926535897932384626433832795028841971");
    let got = digits(&transcend::pi_for_tests(300), 40);
    assert_eq!(got, want, "forty-one digits of pi");
}

#[test]
fn the_trig_answers_what_the_identities_say_it_must() {
    // No published table can cover every argument, so the check is the relations that
    // hold everywhere: a sine and a cosine square to one, and a tangent is their ratio.
    let mut rng = Rng(0x5EED);
    for _ in 0..400 {
        let x = rng.unit() * 200.0 - 100.0;
        let (s, c) = (transcend::sin(x), transcend::cos(x));
        let sum = s * s + c * c;
        assert!((sum - 1.0).abs() < 1e-15, "sin({x})^2 + cos({x})^2 was {sum}");
        let t = transcend::tan(x);
        if c.abs() > 1e-8 {
            let ratio = s / c;
            assert!(
                (t - ratio).abs() <= ratio.abs() * 1e-14,
                "tan({x}) was {t} and sin/cos is {ratio}"
            );
        }
        // And the angle comes back from its own tangent, inside a quarter turn.
        let small = rng.unit() * 2.0 - 1.0;
        let back = transcend::tan(transcend::atan(small));
        assert!((back - small).abs() < 1e-14, "atan then tan of {small} gave {back}");
    }
}

#[test]
fn a_sine_of_something_enormous_is_still_a_sine() {
    // The case a C library cannot do: reducing 10^300 into a quarter turn needs π to a
    // thousand bits, which nobody keeps in a table. Here it is a `Big`.
    for x in [1e17f64, 1e100, 1e300, -1e300] {
        let (s, c) = (transcend::sin(x), transcend::cos(x));
        let sum = s * s + c * c;
        assert!((sum - 1.0).abs() < 1e-15, "sin({x:e})^2 + cos({x:e})^2 was {sum}");
        assert!(s.abs() <= 1.0 && c.abs() <= 1.0, "out of range at {x:e}");
    }
}

#[test]
fn atan2_takes_its_quarter_from_both_signs() {
    let pi = std::f64::consts::PI;
    assert_eq!(transcend::atan2(0.0, 1.0), 0.0);
    assert_eq!(transcend::atan2(-0.0, 1.0), -0.0, "the sign survives");
    assert_eq!(transcend::atan2(0.0, -1.0), pi);
    assert_eq!(transcend::atan2(-0.0, -1.0), -pi);
    assert!((transcend::atan2(1.0, 1.0) - pi / 4.0).abs() < 1e-15);
    assert!((transcend::atan2(1.0, -1.0) - 3.0 * pi / 4.0).abs() < 1e-15);
    assert!((transcend::atan2(-1.0, -1.0) + 3.0 * pi / 4.0).abs() < 1e-15);
    assert!((transcend::atan2(1.0, 0.0) - pi / 2.0).abs() < 1e-15);
}

/// The series being right, an answer at three hundred bits settles which `b64` is
/// correct — and where the platform's library differs, this says which of the two the
/// true value is actually nearer.
#[test]
fn where_we_differ_from_the_platform_the_wider_answer_agrees_with_us() {
    let mut rng = Rng(0x2026_0904);
    let mut differed = 0;
    let mut ours_right = 0;
    for _ in 0..3_000 {
        let x = rng.unit() * 1400.0 - 700.0;
        let (ours, theirs) = (transcend::exp(x), x.exp());
        if ours.to_bits() == theirs.to_bits() {
            continue;
        }
        differed += 1;
        // Three hundred bits is far finer than the gap between two `b64`s, so what this
        // rounds to is the correctly rounded answer whichever it turns out to be.
        let settled = transcend::exp_for_tests(x, 300).to_f64();
        assert_eq!(
            settled.to_bits(),
            ours.to_bits(),
            "exp({x}): ours {ours:e}, platform {theirs:e}, three hundred bits says {settled:e}"
        );
        ours_right += 1;
    }
    assert!(differed > 0, "the platform agreed everywhere, so this checked nothing");
    assert_eq!(differed, ours_right);
    println!("{differed} of 3000 differed from the platform, and the wider answer backed us every time");
}

struct Rng(u64);

impl Rng {
    fn unit(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / 9_007_199_254_740_992.0
    }
}
