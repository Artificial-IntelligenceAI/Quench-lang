//! Every binary16 there is, put into an `f32` and rounded back.
//!
//!     cargo run --release -p quench-num --example halves

fn main() {
    let mut wrong = 0u32;
    for bits in 0u32..=0xffff {
        let h = bits as u16;
        let carried = quench_num::from_b16_bits(h);
        if carried.is_nan() {
            continue;
        }
        let back = quench_num::to_b16_bits(carried);
        if back != h {
            if wrong < 5 {
                println!("{h:#06x} -> {carried} -> {back:#06x}");
            }
            wrong += 1;
        }
    }
    println!("binary16 values that do not survive the carrier: {wrong} of 65536");
}
