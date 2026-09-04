//! Ours against the platform's, and then against arithmetic wide enough to settle it.
//!
//! A disagreement is not automatically our bug: these are the functions IEEE 754 only
//! *recommends* rounding correctly. Where the two differ, the question is which the true
//! value is nearer, and four hundred bits answers that far past any doubt.
//!
//!     cargo run --release -p quench-num --example transcend

fn main() {
    let mut rng = 0x2026_0904u64;
    let mut next = move || { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; rng };
    let unit = |n: u64| (n >> 11) as f64 / 9_007_199_254_740_992.0;
    let rounds = 5_000;

    let tally = |name: &str, differ: u32, ours: u32, theirs: u32| {
        println!(
            "{name:<6} differed on {differ:>5} of {rounds}   ours right {ours:>5}   platform right {theirs:>5}"
        );
    };

    let (mut ds, mut so, mut st) = (0, 0, 0);
    let (mut dc, mut co, mut ct) = (0, 0, 0);
    let (mut dt, mut to, mut tt) = (0, 0, 0);
    let (mut da, mut ao, mut at) = (0, 0, 0);
    let (mut de, mut eo, mut et) = (0, 0, 0);
    for _ in 0..rounds {
        let x = unit(next()) * 200.0 - 100.0;
        let (o, t) = (quench_num::transcend::sin(x), x.sin());
        if o.to_bits() != t.to_bits() {
            ds += 1;
            let settled = quench_num::transcend::sin_for_tests(x, 400).to_f64();
            if settled.to_bits() == o.to_bits() { so += 1 } else if settled.to_bits() == t.to_bits() { st += 1 }
        }
        let (o, t) = (quench_num::transcend::cos(x), x.cos());
        if o.to_bits() != t.to_bits() {
            dc += 1;
            let settled = quench_num::transcend::cos_for_tests(x, 400).to_f64();
            if settled.to_bits() == o.to_bits() { co += 1 } else if settled.to_bits() == t.to_bits() { ct += 1 }
        }
        let (o, t) = (quench_num::transcend::tan(x), x.tan());
        if o.to_bits() != t.to_bits() {
            dt += 1;
            let s = quench_num::transcend::sin_for_tests(x, 400);
            let c = quench_num::transcend::cos_for_tests(x, 400);
            let settled = s.div(&c).to_f64();
            if settled.to_bits() == o.to_bits() { to += 1 } else if settled.to_bits() == t.to_bits() { tt += 1 }
        }
        let a = unit(next()) * 20.0 - 10.0;
        let (o, t) = (quench_num::transcend::atan(a), a.atan());
        if o.to_bits() != t.to_bits() {
            da += 1;
            let settled = quench_num::transcend::atan_for_tests(a, 400).to_f64();
            if settled.to_bits() == o.to_bits() { ao += 1 } else if settled.to_bits() == t.to_bits() { at += 1 }
        }
        let e = unit(next()) * 1400.0 - 700.0;
        let (o, t) = (quench_num::transcend::exp(e), e.exp());
        if o.to_bits() != t.to_bits() {
            de += 1;
            let settled = quench_num::transcend::exp_for_tests(e, 400).to_f64();
            if settled.to_bits() == o.to_bits() { eo += 1 } else if settled.to_bits() == t.to_bits() { et += 1 }
        }
    }
    tally("sin", ds, so, st);
    tally("cos", dc, co, ct);
    tally("tan", dt, to, tt);
    tally("atan", da, ao, at);
    tally("exp", de, eo, et);
}
