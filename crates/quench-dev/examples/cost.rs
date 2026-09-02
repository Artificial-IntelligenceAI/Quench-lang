//! How long the Dev JIT spends compiling, versus how long the program then runs.
//! The generated programs an oracle uses are small, so this ratio is what decides
//! whether a fourth engine would slow the oracle down or speed it up.

use quench_qir::{Builder, CmpOp, Module, Ty};
use std::time::Instant;

fn small() -> Module {
    // Roughly what a generated program looks like: a loop, some arithmetic, a call.
    let mut m = Module::new();
    let me = m.next_id();
    let mut b = Builder::new("helper", &[Ty::I64], Ty::I64);
    let base = b.block(&[]);
    let step = b.block(&[]);
    let n = b.param(0);
    let one = b.const_i64(1);
    let small = b.cmp(CmpOp::Le, n, one);
    b.br_if(small, (base, &[]), (step, &[]));
    b.switch_to(base);
    let one = b.const_i64(1);
    b.ret(one);
    b.switch_to(step);
    let one = b.const_i64(1);
    let less = b.sub(n, one);
    let rest = b.call(me, &[less], Ty::I64);
    let total = b.mul(n, rest);
    b.ret(total);
    let helper = m.add(b.finish());

    let mut b = Builder::new("START", &[], Ty::I64);
    let head = b.block(&[Ty::I64, Ty::I64]);
    let body = b.block(&[Ty::I64, Ty::I64]);
    let done = b.block(&[Ty::I64]);
    let zero = b.const_i64(0);
    let one = b.const_i64(1);
    b.jump(head, &[zero, one]);
    b.switch_to(head);
    let (acc, i) = (b.block_param(head, 0), b.block_param(head, 1));
    let limit = b.const_i64(20);
    let more = b.cmp(CmpOp::Le, i, limit);
    b.br_if(more, (body, &[acc, i]), (done, &[acc]));
    b.switch_to(body);
    let (acc, i) = (b.block_param(body, 0), b.block_param(body, 1));
    let f = b.call(helper, &[i], Ty::I64);
    let next_acc = b.add(acc, f);
    let step_by = b.const_i64(1);
    let next_i = b.add(i, step_by);
    b.jump(head, &[next_acc, next_i]);
    b.switch_to(done);
    let result = b.block_param(done, 0);
    b.ret(result);
    let start = m.add(b.finish());
    m.set_entry(start);
    m
}

fn main() {
    let module = small();
    const ROUNDS: u32 = 200;

    let mut compiled = None;
    let compiling = Instant::now();
    for _ in 0..ROUNDS {
        compiled = Some(quench_dev::compile(&module).expect("it compiles"));
    }
    let per_compile = compiling.elapsed() / ROUNDS;

    let compiled = compiled.expect("one of them stuck");
    let answer = compiled.run();
    let running = Instant::now();
    for _ in 0..ROUNDS {
        std::hint::black_box(compiled.run());
    }
    let per_run = running.elapsed() / ROUNDS;

    println!("answer:        {answer}");
    println!("compile:       {per_compile:?}");
    println!("run:           {per_run:?}");
    println!("compile is     {:.0}x the run", per_compile.as_secs_f64() / per_run.as_secs_f64());
}
