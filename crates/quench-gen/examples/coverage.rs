//! What the generator actually writes, counted.
//!
//! `cargo run --release -p quench-gen --example coverage`
//!
//! The oracle can only check what somebody generated, and a step that never fires looks
//! exactly like a step that fires and passes: both are silence. This prints the shapes
//! rather than judging them, which is what `tests/reaches.rs` is for -- the point of
//! having it as well is that a count of one in two thousand is a shape nobody has
//! really tested, and a test can only say whether it was there at all.

use std::collections::BTreeMap;

use quench_qir::Inst;

fn main() {
    let module = quench_gen::batch(&(1..=4000u64).collect::<Vec<_>>());
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();

    for func in &module.functions {
        for ty in &func.params {
            *seen.entry(format!("takes {}", ty.name())).or_default() += 1;
        }
        *seen.entry(format!("gives {}", func.ret.name())).or_default() += 1;
        for block in &func.blocks {
            for (_, inst) in &block.insts {
                let name = match inst {
                    Inst::CallHost { host, .. } => host.name().to_string(),
                    Inst::Bin { op, .. } => format!("{op:?}"),
                    Inst::CmpU { .. } => "cmp-unsigned".to_string(),
                    Inst::Call { .. } => "call".to_string(),
                    Inst::Narrow { .. } => "narrow".to_string(),
                    Inst::FCmp { .. } => "fcmp".to_string(),
                    _ => continue,
                };
                *seen.entry(name).or_default() += 1;
            }
        }
    }

    println!("{} functions, from four thousand seeds\n", module.functions.len());
    for (shape, n) in &seen {
        println!("{n:>8}  {shape}");
    }
}
