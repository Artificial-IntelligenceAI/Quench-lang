//! Ours against the platform's, adjudicated at four hundred bits where the two differ.
//!
//!     cargo run --release -p quench-num --example transcend
fn main() {
    use quench_num::{transcend as t, Wide};
    let mut rng = 0x2026_0904u64;
    let mut next = move || { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; rng };
    let unit = |n: u64| (n >> 11) as f64 / 9_007_199_254_740_992.0;
    let nearer = |truth: &Wide, a: f64, b: f64| -> f64 {
        let (wa, wb) = (Wide::from_f64(a, truth.bits()), Wide::from_f64(b, truth.bits()));
        if truth.sub(&wa).abs().cmp_abs(&truth.sub(&wb).abs()) == std::cmp::Ordering::Greater { b } else { a }
    };
    let n = 5000;
    let cases: [(&str, fn(f64) -> f64, fn(f64) -> f64, f64, f64); 13] = [
        ("exp", t::exp, |x| x.exp(), -700.0, 700.0),
        ("ln", t::ln, |x| x.ln(), 1e-6, 1e6),
        ("sin", t::sin, |x| x.sin(), -100.0, 100.0),
        ("cos", t::cos, |x| x.cos(), -100.0, 100.0),
        ("tan", t::tan, |x| x.tan(), -100.0, 100.0),
        ("atan", t::atan, |x| x.atan(), -10.0, 10.0),
        ("asin", t::asin, |x| x.asin(), -1.0, 1.0),
        ("acos", t::acos, |x| x.acos(), -1.0, 1.0),
        ("sinh", t::sinh, |x| x.sinh(), -10.0, 10.0),
        ("cosh", t::cosh, |x| x.cosh(), -10.0, 10.0),
        ("tanh", t::tanh, |x| x.tanh(), -10.0, 10.0),
        ("atanh", t::atanh, |x| x.atanh(), -1.0, 1.0),
        ("cbrt", t::cbrt, |x| x.cbrt(), -100.0, 100.0),
    ];
    let (mut total, mut lost) = (0, 0);
    for (name, ours_fn, theirs_fn, low, high) in cases {
        let (mut differ, mut win) = (0, 0);
        for _ in 0..n {
            let x = low + unit(next()) * (high - low);
            let (o, p) = (ours_fn(x), theirs_fn(x));
            if !o.is_finite() || !p.is_finite() || o.to_bits() == p.to_bits() { continue; }
            differ += 1;
            let truth = t::at_width(name, x, 0.0, 400);
            if nearer(&truth, o, p).to_bits() == o.to_bits() { win += 1 }
        }
        total += differ;
        lost += differ - win;
        println!("{name:<6} differed on {differ:>5} of {n}   nearer ours {win:>5}, platform {:>5}", differ - win);
    }
    println!("\n{total} disagreements, {lost} of them the platform's way");
}
