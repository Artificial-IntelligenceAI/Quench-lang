//! The interpreter on its own, and then the interpreter against the Dev JIT.
//!
//! The second half is the first thing in Quench that is actually an oracle: one program,
//! two engines, one answer insisted upon. Two of the three engines exist so far, and the
//! third will join this file rather than replace it.

use quench_interp::{run, Outcome, Trap};
use quench_qir::{BinOp, Builder, CmpOp, Module, Ty};

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

// --- the oracle ---------------------------------------------------------------------

/// A deterministic scrambler, so a disagreement can be replayed from its seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn upto(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// A program built from the types outward, so it always compiles, and containing no
/// division by zero, so it always finishes. Both engines must therefore answer.
///
/// This is a sketch of what `quench-gen` will be, kept here because an oracle with
/// nothing to run is not an oracle.
fn written(seed: u64) -> Module {
    let mut rng = Rng(seed | 1);
    let mut b = Builder::new(quench_qir::ENTRY, &[], Ty::I64);

    let mut numbers = vec![b.const_i64(rng.next() as i64), b.const_i64(rng.upto(1000) as i64)];
    let mut booleans = Vec::new();

    for _ in 0..rng.upto(24) + 4 {
        match rng.upto(8) {
            0 => numbers.push(b.const_i64(rng.next() as i64)),
            1..=4 => {
                let lhs = numbers[rng.upto(numbers.len() as u64) as usize];
                let rhs = numbers[rng.upto(numbers.len() as u64) as usize];
                let op = match rng.upto(5) {
                    0 => BinOp::Add,
                    1 => BinOp::Sub,
                    2 => BinOp::Mul,
                    3 => BinOp::Div,
                    _ => BinOp::Rem,
                };
                if matches!(op, BinOp::Div | BinOp::Rem) {
                    // A divisor that cannot be zero and cannot be -1, so neither engine
                    // traps and both have to produce a number.
                    let safe = b.const_i64(rng.upto(9999) as i64 + 2);
                    numbers.push(b.bin(op, lhs, safe));
                } else {
                    numbers.push(b.bin(op, lhs, rhs));
                }
            }
            5 | 6 => {
                let lhs = numbers[rng.upto(numbers.len() as u64) as usize];
                let rhs = numbers[rng.upto(numbers.len() as u64) as usize];
                let op = match rng.upto(6) {
                    0 => CmpOp::Eq,
                    1 => CmpOp::Ne,
                    2 => CmpOp::Lt,
                    3 => CmpOp::Le,
                    4 => CmpOp::Gt,
                    _ => CmpOp::Ge,
                };
                booleans.push(b.cmp(op, lhs, rhs));
            }
            _ => {
                if let Some(&flag) = booleans.last() {
                    booleans.push(b.not(flag));
                }
            }
        }
    }

    // End on a branch, so control flow is exercised rather than one straight line.
    if let Some(&flag) = booleans.last() {
        let yes = b.block(&[]);
        let no = b.block(&[]);
        b.br_if(flag, (yes, &[]), (no, &[]));
        let last = *numbers.last().expect("there is always one");
        let first = numbers[0];
        b.switch_to(yes);
        b.ret(last);
        b.switch_to(no);
        b.ret(first);
    } else {
        let last = *numbers.last().expect("there is always one");
        b.ret(last);
    }

    just(b.finish())
}

#[test]
fn the_interpreter_and_the_dev_jit_agree() {
    // One program, two engines, one answer. The third joins here when it exists.
    for seed in 1..500u64 {
        let module = written(seed);
        let walked = run(&module).expect("it runs");
        let compiled = quench_dev::compile(&module).expect("it compiles").run();

        match walked {
            Outcome::Returned(value) => assert_eq!(
                value, compiled,
                "seed {seed}: the interpreter said {value}, the Dev JIT said {compiled}"
            ),
            Outcome::Trapped(trap) => {
                panic!("seed {seed}: the interpreter stopped with {trap:?}, and generated programs are not supposed to stop")
            }
        }
    }
}
