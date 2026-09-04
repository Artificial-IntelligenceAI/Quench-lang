//! Ours against the platform's, on a lot of arguments. A disagreement is not
//! automatically our bug: the platform is only *recommended* to round correctly, and
//! this file is written on the premise that it usually does and sometimes does not.
fn main() {
    let mut rng = 0x2026_0904u64;
    let mut next = move || { rng ^= rng << 13; rng ^= rng >> 7; rng ^= rng << 17; rng };
    let unit = |n: u64| (n >> 11) as f64 / 9_007_199_254_740_992.0; // [0, 1)
    let (mut de, mut dl, mut dp) = (0u32, 0u32, 0u32);
    let mut shown = 0;
    let rounds = 20_000;
    for _ in 0..rounds {
        let x = unit(next()) * 1400.0 - 700.0;
        let positive = unit(next()) * 1e6 + 1e-6;
        let y = unit(next()) * 8.0 - 4.0;

        let (o, t) = (quench_num::transcend::exp(x), x.exp());
        if o.to_bits() != t.to_bits() {
            de += 1;
            if shown < 8 { println!("exp({x:e})  ours {o:e}  platform {t:e}"); shown += 1; }
        }
        let (o, t) = (quench_num::transcend::ln(positive), positive.ln());
        if o.to_bits() != t.to_bits() {
            dl += 1;
            if shown < 8 { println!("ln({positive:e})  ours {o:e}  platform {t:e}"); shown += 1; }
        }
        let (o, t) = (quench_num::transcend::pow(positive, y), positive.powf(y));
        if o.is_finite() && t.is_finite() && o.to_bits() != t.to_bits() {
            dp += 1;
            if shown < 8 { println!("pow({positive:e}, {y:e})  ours {o:e}  platform {t:e}"); shown += 1; }
        }
    }
    println!("{rounds} arguments each: exp differs {de}, ln differs {dl}, pow differs {dp}");
}
