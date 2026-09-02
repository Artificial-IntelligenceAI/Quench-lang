//! The interpreter on its own, and then the interpreter against the Dev JIT.
//!
//! The second half is the first thing in Quench that is actually an oracle: one program,
//! two engines, one answer insisted upon. Two of the three engines exist so far, and the
//! third will join this file rather than replace it.

use quench_interp::{run, Outcome, Trap};
use quench_qir::{Builder, CmpOp, Module, Ty};

fn just(f: quench_qir::Function) -> Module {
    let mut m = Module::new();
    let id = m.add(f);
    m.set_entry(id);
    m
}

fn returned(m: &Module) -> i64 {
    match run(m).expect("it runs") {
        Outcome::Returned(v) => v,
        other => panic!("expected a value, got {other:?}"),
    }
}

#[test]
fn it_gets_the_same_answers_the_dev_jit_does() {
    // The programs the Dev JIT is already asserted on, so a difference here is visible
    // immediately rather than only through the oracle.
    let mut b = Builder::new(quench_qir::ENTRY, &[], Ty::I64);
    let seven = b.const_i64(7);
    let five = b.const_i64(5);
    let three = b.const_i64(3);
    let sixteen = b.const_i64(16);
    let four = b.const_i64(4);
    let sum = b.add(seven, five);
    let scaled = b.mul(sum, three);
    let quotient = b.div(sixteen, four);
    let leftover = b.rem(quotient, three);
    let total = b.sub(scaled, leftover);
    b.ret(total);
    assert_eq!(returned(&just(b.finish())), 35);
}

#[test]
fn a_loop_carries_its_values() {
    let mut b = Builder::new(quench_qir::ENTRY, &[], Ty::I64);
    let head = b.block(&[Ty::I64, Ty::I64]);
    let body = b.block(&[Ty::I64, Ty::I64]);
    let done = b.block(&[Ty::I64]);
    let zero = b.const_i64(0);
    let one = b.const_i64(1);
    b.jump(head, &[zero, one]);
    b.switch_to(head);
    let (acc, i) = (b.block_param(head, 0), b.block_param(head, 1));
    let ten = b.const_i64(10);
    let more = b.cmp(CmpOp::Le, i, ten);
    b.br_if(more, (body, &[acc, i]), (done, &[acc]));
    b.switch_to(body);
    let (acc, i) = (b.block_param(body, 0), b.block_param(body, 1));
    let next_acc = b.add(acc, i);
    let step = b.const_i64(1);
    let next_i = b.add(i, step);
    b.jump(head, &[next_acc, next_i]);
    b.switch_to(done);
    let result = b.block_param(done, 0);
    b.ret(result);
    assert_eq!(returned(&just(b.finish())), 55);
}

#[test]
fn stopping_is_an_answer_too() {
    // Which stop it was matters, not just that it stopped.
    let mut b = Builder::new(quench_qir::ENTRY, &[], Ty::I64);
    let one = b.const_i64(1);
    let zero = b.const_i64(0);
    let bad = b.div(one, zero);
    b.ret(bad);
    assert_eq!(run(&just(b.finish())).unwrap(), Outcome::Trapped(Trap::DividedByZero));

    let mut b = Builder::new(quench_qir::ENTRY, &[], Ty::I64);
    let least = b.const_i64(i64::MIN);
    let minus_one = b.const_i64(-1);
    let bad = b.div(least, minus_one);
    b.ret(bad);
    assert_eq!(
        run(&just(b.finish())).unwrap(),
        Outcome::Trapped(Trap::DivisionOverflowed),
        "i64::MIN / -1 does not fit, and Cranelift traps rather than wrapping"
    );
}

#[test]
fn arithmetic_wraps_the_way_the_machine_does() {
    let mut b = Builder::new(quench_qir::ENTRY, &[], Ty::I64);
    let most = b.const_i64(i64::MAX);
    let one = b.const_i64(1);
    let over = b.add(most, one);
    b.ret(over);
    assert_eq!(returned(&just(b.finish())), i64::MIN, "iadd wraps, so this does");
}

#[test]
fn recursion_that_never_stops_is_stopped() {
    let mut m = Module::new();
    let me = m.next_id();
    let mut b = Builder::new("forever", &[Ty::I64], Ty::I64);
    let n = b.param(0);
    let again = b.call(me, &[n], Ty::I64);
    b.ret(again);
    let forever = m.add(b.finish());

    let mut b = Builder::new(quench_qir::ENTRY, &[], Ty::I64);
    let one = b.const_i64(1);
    let never = b.call(forever, &[one], Ty::I64);
    b.ret(never);
    let start = m.add(b.finish());
    m.set_entry(start);

    assert_eq!(run(&m).unwrap(), Outcome::Trapped(Trap::TooDeep));
}

// The oracle itself lives in `quench-gen`, which generates properly and runs batches
// across every core. What stays here is the pair of engines agreeing on cases chosen by
// hand -- the ones where a difference would be a difference of *meaning* rather than of
// luck, and so are worth naming rather than stumbling upon.

#[test]
fn the_two_divisions_are_different_languages() {
    // -7 / 2 is -3 remainder -1 one way, and -4 remainder 1 the other. A program that
    // divides means a different thing under each, which is why one setting doubles what
    // the oracle has to prove.
    let cases: [(fn(&mut Builder, quench_qir::Value, quench_qir::Value) -> quench_qir::Value, i64); 4] = [
        (|b, l, r| b.div(l, r), -3),
        (|b, l, r| b.rem(l, r), -1),
        (|b, l, r| b.div_floored(l, r), -4),
        (|b, l, r| b.rem_floored(l, r), 1),
    ];
    for (build, want) in cases {
        let mut b = Builder::new(quench_qir::ENTRY, &[], Ty::I64);
        let seven = b.const_i64(-7);
        let two = b.const_i64(2);
        let value = build(&mut b, seven, two);
        b.ret(value);
        let module = just(b.finish());

        assert_eq!(returned(&module), want, "the interpreter");
        assert_eq!(
            quench_dev::compile(&module).expect("it compiles").run(),
            want,
            "and the Dev JIT, which has to build flooring out of what the processor gives it"
        );
    }
}

#[test]
fn flooring_is_flooring_in_both_engines() {
    // Checked against what flooring *is* rather than against a second copy of how it is
    // done -- a formula here would only re-derive the implementation and agree with its
    // mistakes. The defining properties: q·d + r is a, the remainder leans the way the
    // divisor does, and it is smaller than the divisor.
    //
    // All four sign quadrants, and the cases where the remainder is zero and nothing
    // should be corrected at all.
    fn value(build: impl FnOnce(&mut Builder) -> quench_qir::Value) -> (i64, i64) {
        let mut b = Builder::new(quench_qir::ENTRY, &[], Ty::I64);
        let out = build(&mut b);
        b.ret(out);
        let module = just(b.finish());
        (returned(&module), quench_dev::compile(&module).expect("it compiles").run())
    }

    for a in [-13i64, -12, -1, 0, 1, 12, 13] {
        for d in [-6i64, -4, -1, 1, 4, 6] {
            let (walked_q, compiled_q) =
                value(|b| { let l = b.const_i64(a); let r = b.const_i64(d); b.div_floored(l, r) });
            let (walked_r, compiled_r) =
                value(|b| { let l = b.const_i64(a); let r = b.const_i64(d); b.rem_floored(l, r) });

            assert_eq!(walked_q, compiled_q, "{a} / {d}: the engines disagree");
            assert_eq!(walked_r, compiled_r, "{a} % {d}: the engines disagree");

            let (q, r) = (walked_q, walked_r);
            assert_eq!(q * d + r, a, "{a} / {d}: q·d + r is not a");
            assert!(r.abs() < d.abs(), "{a} % {d}: remainder {r} is not smaller than {d}");
            if r != 0 {
                assert_eq!(r < 0, d < 0, "{a} % {d}: remainder {r} leans the wrong way");
            }
            // And that it really floored: truncating would have rounded toward zero.
            assert!(q <= a / d, "{a} / {d}: flooring never rounds further from -inf than truncating");
        }
    }
}
