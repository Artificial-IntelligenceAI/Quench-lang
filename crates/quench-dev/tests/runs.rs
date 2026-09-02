//! The Dev JIT, end to end: hand-written QIR, compiled by Cranelift, run, and asked what
//! it got.
//!
//! There is no parser yet, so these build the IR directly. That is not a workaround --
//! it is the only way to test a backend without the frontend's opinions in the way, and
//! these tests stay useful once there is a parser for exactly that reason.

use quench_dev::compile;
use quench_qir::{BinOp, Builder, CmpOp, Module, Ty};

/// A module with one function, which is the entry.
fn just(f: quench_qir::Function) -> Module {
    let mut m = Module::new();
    let id = m.add(f);
    m.set_entry(id);
    m
}

#[test]
fn an_entry_can_return_a_constant() {
    let mut b = Builder::new("main", &[], Ty::I64);
    let n = b.const_i64(42);
    b.ret(n);

    assert_eq!(compile(&just(b.finish())).unwrap().run(), 42);
}

#[test]
fn arithmetic_arrives_intact() {
    // (7 + 5) * 3 - 16 / 4 % 3  ==  36 - 1  ==  35
    let mut b = Builder::new("main", &[], Ty::I64);
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

    assert_eq!(compile(&just(b.finish())).unwrap().run(), 35);
}

#[test]
fn negative_division_truncates_towards_zero() {
    // The one arithmetic answer the three engines could plausibly differ on, so it is
    // pinned here from the start rather than discovered later by the oracle.
    let mut b = Builder::new("main", &[], Ty::I64);
    let minus_seven = b.const_i64(-7);
    let two = b.const_i64(2);
    let q = b.div(minus_seven, two);
    let r = b.rem(minus_seven, two);
    // -3 and -1, so -3 * 10 + -1 == -31, which distinguishes it from flooring.
    let ten = b.const_i64(10);
    let scaled = b.mul(q, ten);
    let total = b.add(scaled, r);
    b.ret(total);

    assert_eq!(compile(&just(b.finish())).unwrap().run(), -31);
}

#[test]
fn a_loop_carries_its_values_in_block_parameters() {
    // acc = 0; for i in 1..=10 { acc += i }  ->  55
    let mut b = Builder::new("main", &[], Ty::I64);
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

    assert_eq!(compile(&just(b.finish())).unwrap().run(), 55);
}

#[test]
fn a_function_can_call_itself() {
    let mut m = Module::new();
    let me = m.next_id();

    let mut b = Builder::new("factorial", &[Ty::I64], Ty::I64);
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

    let factorial = m.add(b.finish());
    assert_eq!(factorial, me, "next_id promised the id the function actually got");

    let mut b = Builder::new("main", &[], Ty::I64);
    let ten = b.const_i64(10);
    let answer = b.call(factorial, &[ten], Ty::I64);
    b.ret(answer);
    let main = m.add(b.finish());
    m.set_entry(main);

    assert_eq!(compile(&m).unwrap().run(), 3_628_800);
}

#[test]
fn booleans_survive_being_negated() {
    let mut b = Builder::new("main", &[], Ty::I64);
    let yes = b.block(&[]);
    let no = b.block(&[]);

    let one = b.const_i64(1);
    let two = b.const_i64(2);
    let less = b.cmp(CmpOp::Lt, one, two);
    let not_less = b.not(less);
    b.br_if(not_less, (yes, &[]), (no, &[]));

    b.switch_to(yes);
    let wrong = b.const_i64(0);
    b.ret(wrong);

    b.switch_to(no);
    let right = b.const_i64(1);
    b.ret(right);

    assert_eq!(compile(&just(b.finish())).unwrap().run(), 1);
}

#[test]
fn every_comparison_means_what_it_says() {
    let cases = [
        (CmpOp::Eq, 3, 3, 1),
        (CmpOp::Eq, 3, 4, 0),
        (CmpOp::Ne, 3, 4, 1),
        (CmpOp::Lt, -1, 0, 1),
        (CmpOp::Le, 5, 5, 1),
        (CmpOp::Gt, 5, 5, 0),
        (CmpOp::Ge, 5, 5, 1),
    ];
    for (op, lhs, rhs, want) in cases {
        let mut b = Builder::new("main", &[], Ty::I64);
        let t = b.block(&[]);
        let f = b.block(&[]);
        let l = b.const_i64(lhs);
        let r = b.const_i64(rhs);
        let c = b.cmp(op, l, r);
        b.br_if(c, (t, &[]), (f, &[]));
        b.switch_to(t);
        let one = b.const_i64(1);
        b.ret(one);
        b.switch_to(f);
        let zero = b.const_i64(0);
        b.ret(zero);

        assert_eq!(compile(&just(b.finish())).unwrap().run(), want, "{op:?} {lhs} {rhs}");
    }
}

#[test]
fn an_entry_that_takes_arguments_is_refused() {
    let mut b = Builder::new("main", &[Ty::I64], Ty::I64);
    let n = b.param(0);
    b.ret(n);

    let err = compile(&just(b.finish())).unwrap_err();
    assert!(err.to_string().contains("called with none"), "{err}");
}

#[test]
fn a_module_with_no_entry_says_so() {
    let mut b = Builder::new("somewhere", &[], Ty::I64);
    let n = b.const_i64(1);
    b.ret(n);
    let mut m = Module::new();
    m.add(b.finish());

    let err = compile(&m).unwrap_err();
    assert!(err.to_string().contains("names no entry"), "{err}");
}

#[test]
fn ill_typed_ir_is_stopped_before_cranelift_sees_it() {
    // Adding a bool to an i64. Nothing in the frontend should produce this; if something
    // does, it is caught here rather than becoming three different wrong answers.
    let mut b = Builder::new("main", &[], Ty::I64);
    let one = b.const_i64(1);
    let two = b.const_i64(2);
    let flag = b.cmp(CmpOp::Lt, one, two);
    let nonsense = b.bin(BinOp::Add, one, flag);
    b.ret(nonsense);

    let err = compile(&just(b.finish())).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("not well formed"), "{text}");
    assert!(text.contains("wants i64"), "{text}");
}
