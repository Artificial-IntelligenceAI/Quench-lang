//! Does an arbitrary `f32` round to the binary16 it should?
//!
//! Checked against a search over every binary16 there is, which is slow and obviously
//! right — the two things a reference wants to be.
//!
//!     cargo run --release -p quench-num --example rounding

fn main() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut wrong = 0u32;
    let mut checked = 0u32;
    for _ in 0..300_000 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        // Aimed at the range binary16 covers, and a little past both ends of it.
        // Aimed across binary16's whole range: subnormals, normals, and a little past
        // the top where the answer stops being a number.
        let exponent = (seed >> 40) as u32 % 46 + 96;
        let x = f32::from_bits(
            (seed as u32 & 0x8000_0000) | (exponent << 23) | ((seed >> 8) as u32 & 0x007f_ffff),
        );
        if !x.is_finite() {
            continue;
        }
        checked += 1;
        let ours = quench_num::to_b16_bits(x);
        let want = nearest_by_search(x);
        if ours != want {
            if wrong < 6 {
                println!("{x:e}: we say {ours:#06x} ({}), search says {want:#06x} ({})",
                    quench_num::from_b16_bits(ours), quench_num::from_b16_bits(want));
            }
            wrong += 1;
        }
    }
    println!("{checked} checked, {wrong} rounded the wrong way");
}

/// The nearest binary16, found by looking at all of them. Ties to even.
///
/// Overflow is not a search: IEEE rounds anything from halfway between the largest
/// finite value and the next power of two — 65520 — to an infinity, and no finite
/// value is nearest to that.
fn nearest_by_search(x: f32) -> u16 {
    if x.abs() >= 65520.0 {
        return if x < 0.0 { 0xfc00 } else { 0x7c00 };
    }
    let target = f64::from(x);
    let (mut best, mut best_gap) = (0u16, f64::INFINITY);
    for bits in 0u32..=0xffff {
        let h = bits as u16;
        let value = f64::from(quench_num::from_b16_bits(h));
        if value.is_nan() {
            continue;
        }
        let gap = (value - target).abs();
        if gap < best_gap || (gap == best_gap && h & 1 == 0 && best & 1 == 1) {
            best = h;
            best_gap = gap;
        }
    }
    // A zero keeps the sign of what underflowed to it, and a search by value cannot
    // see that, because `-0.0 == 0.0`.
    if best & 0x7fff == 0 {
        return if x.is_sign_negative() { 0x8000 } else { 0x0000 };
    }
    best
}
