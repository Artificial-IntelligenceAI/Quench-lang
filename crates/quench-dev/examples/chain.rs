//! The dependent-chain benchmark, the same one Luarust uses:
//! `sum = (sum + i) mod 1000000007`, ten million times.
//!
//! Each value needs the one before it, so it cannot be folded into a formula,
//! vectorised, or run out of order. What is measured is one add and one remainder.
//!
//!     cargo run --release -p quench-dev --example chain -- 100000000 [add]
//!
//! `add` drops the modulus, which matters more than it sounds: without it the loop has a
//! closed form, and an optimising compiler replaces the whole thing with the answer.
//! Luarust's LLVM JIT does exactly that — its emitted IR for a hundred million
//! iterations is a single `call print(5000000050000000)`. This engine runs all hundred
//! million, at half a nanosecond each, because `opt_level = none` is what it is for.

use quench_qir::{Builder, CmpOp, Module, Ty};
use std::time::Instant;

fn chain(iterations: i64, with_modulus: bool) -> Module {
    let mut b = Builder::new(quench_qir::ENTRY, &[], Ty::I64);
    let head = b.block(&[Ty::I64, Ty::I64]);
    let body = b.block(&[Ty::I64, Ty::I64]);
    let done = b.block(&[Ty::I64]);

    let zero = b.const_i64(0);
    let one = b.const_i64(1);
    b.jump(head, &[zero, one]);

    b.switch_to(head);
    let (sum, i) = (b.block_param(head, 0), b.block_param(head, 1));
    let limit = b.const_i64(iterations);
    let more = b.cmp(CmpOp::Le, i, limit);
    b.br_if(more, (body, &[sum, i]), (done, &[sum]));

    b.switch_to(body);
    let (sum, i) = (b.block_param(body, 0), b.block_param(body, 1));
    let added = b.add(sum, i);
    let next_sum = if with_modulus {
        let modulus = b.const_i64(1_000_000_007);
        b.rem(added, modulus)
    } else {
        added
    };
    let step = b.const_i64(1);
    let next_i = b.add(i, step);
    b.jump(head, &[next_sum, next_i]);

    b.switch_to(done);
    let out = b.block_param(done, 0);
    b.ret(out);

    let mut m = Module::new();
    let id = m.add(b.finish());
    m.set_entry(id);
    m
}

fn main() {
    let iterations: i64 =
        std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(10_000_000);
    // Without the modulus the loop has a closed form, which is what an optimiser finds
    // and what `opt_level = none` never will. With it, the chain is irreducible.
    let with_modulus = std::env::args().nth(2).as_deref() != Some("add");
    let module = chain(iterations, with_modulus);

    // Compiling, measured on its own, because that is the half the Dev JIT is for.
    let began = Instant::now();
    let compiled = quench_dev::compile(&module).expect("it compiles");
    let compiling = began.elapsed();

    let began = Instant::now();
    let answer = compiled.run();
    let running = began.elapsed();

    println!("iterations   {iterations}");
    println!("answer       {answer}");
    println!("compile      {compiling:?}");
    println!("run          {running:?}");
    println!("per loop     {:.2}ns", running.as_nanos() as f64 / iterations as f64);
    println!("total        {:?}", compiling + running);
}
