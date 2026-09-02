//! Writing programs, so that the ways of running one can be made to disagree.
//!
//! A generated program is **built from the types outward**, so it always checks out.
//! That is the whole point. A program that failed to check would be rejected identically
//! by every engine and would prove nothing — the interesting programs are the ones that
//! *run*, because those are the ones two implementations can answer differently.
//!
//! Inherited from Luarust, which worked this out first.
//!
//! # What it deliberately never writes
//!
//! - **Anything that stops.** Division and remainder are given a divisor that cannot be
//!   zero and cannot be `-1`, so no program here traps. Not because stopping does not
//!   matter — stopping in the same place for the same reason is exactly as much an
//!   agreement as printing the same number — but because a trap in compiled code is a
//!   signal, and the Dev JIT has no way yet to catch one and report it. The day it does,
//!   this restriction should be the first thing lifted.
//! - **Large loops.** A fuzzer that takes a minute per program is a fuzzer nobody runs,
//!   and the whole economics of the oracle rests on programs being small: compiling one
//!   costs a few hundred times what running it costs.
//! - **Recursion without a floor.** A runaway call is reported by the interpreter and
//!   overflows the stack in compiled code, so the two cannot be compared on it.

use quench_qir::{BinOp, Builder, CmpOp, FuncId, Function, Module, Ty, Value};

/// A deterministic scrambler. Every program is a pure function of its seed, so a
/// disagreement is replayed by writing the seed down.
pub struct Seeded(u64);

impl Seeded {
    pub fn from(seed: u64) -> Seeded {
        // Zero is a fixed point of this generator, so it is never allowed to be the state.
        Seeded(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn upto(&mut self, n: u64) -> usize {
        (self.next() % n) as usize
    }

    fn pick(&mut self, from: &[Value]) -> Value {
        from[self.upto(from.len() as u64)]
    }
}

/// The name a program gets, given its seed. The oracle calls it by this.
pub fn name_of(seed: u64) -> String {
    format!("p{seed}")
}

/// One program: a function taking nothing and giving back an `i64`.
///
/// `helper`, when given, is something the program may call — which is how a generated
/// program exercises calls at all without being able to recurse.
pub fn program(seed: u64, helper: Option<FuncId>) -> Function {
    let mut rng = Seeded::from(seed);
    let mut b = Builder::new(name_of(seed), &[], Ty::I64);

    let mut numbers = vec![
        b.const_i64(rng.next() as i64),
        b.const_i64(rng.upto(1000) as i64),
        b.const_i64(0),
        b.const_i64(-1),
        b.const_i64(i64::MIN),
        b.const_i64(i64::MAX),
    ];
    let mut flags: Vec<Value> = Vec::new();

    let steps = rng.upto(30) + 6;
    for _ in 0..steps {
        match rng.upto(10) {
            0 => {
                let n = b.const_i64(rng.next() as i64);
                numbers.push(n);
            }
            1..=4 => {
                let lhs = rng.pick(&numbers);
                let op = match rng.upto(5) {
                    0 => BinOp::Add,
                    1 => BinOp::Sub,
                    2 => BinOp::Mul,
                    3 => BinOp::Div,
                    _ => BinOp::Rem,
                };
                let value = if matches!(op, BinOp::Div | BinOp::Rem) {
                    // Neither zero nor -1, so neither engine stops. See the module docs.
                    let safe = b.const_i64(rng.upto(9_999) as i64 + 2);
                    b.bin(op, lhs, safe)
                } else {
                    let rhs = rng.pick(&numbers);
                    b.bin(op, lhs, rhs)
                };
                numbers.push(value);
            }
            5 | 6 => {
                let (lhs, rhs) = (rng.pick(&numbers), rng.pick(&numbers));
                let op = match rng.upto(6) {
                    0 => CmpOp::Eq,
                    1 => CmpOp::Ne,
                    2 => CmpOp::Lt,
                    3 => CmpOp::Le,
                    4 => CmpOp::Gt,
                    _ => CmpOp::Ge,
                };
                flags.push(b.cmp(op, lhs, rhs));
            }
            7 => {
                if let Some(&flag) = flags.last() {
                    flags.push(b.not(flag));
                }
            }
            8 => {
                if let Some(id) = helper {
                    let called = b.call(id, &[], Ty::I64);
                    numbers.push(called);
                }
            }
            _ => {
                // A loop, with a bound small enough that running it stays free.
                let head = b.block(&[Ty::I64, Ty::I64]);
                let body = b.block(&[Ty::I64, Ty::I64]);
                let done = b.block(&[Ty::I64]);

                let from = rng.pick(&numbers);
                let one = b.const_i64(1);
                b.jump(head, &[from, one]);

                b.switch_to(head);
                let (acc, i) = (b.block_param(head, 0), b.block_param(head, 1));
                let limit = b.const_i64(rng.upto(20) as i64 + 1);
                let more = b.cmp(CmpOp::Le, i, limit);
                b.br_if(more, (body, &[acc, i]), (done, &[acc]));

                b.switch_to(body);
                let (acc, i) = (b.block_param(body, 0), b.block_param(body, 1));
                let step = match rng.upto(3) {
                    0 => b.add(acc, i),
                    1 => b.mul(acc, i),
                    _ => b.sub(acc, i),
                };
                let one = b.const_i64(1);
                let next_i = b.add(i, one);
                b.jump(head, &[step, next_i]);

                b.switch_to(done);
                let out = b.block_param(done, 0);
                numbers.push(out);
            }
        }
    }

    // Finish on a branch, so control flow is part of the answer rather than decoration.
    let last = *numbers.last().expect("there is always at least one");
    match flags.last() {
        Some(&flag) => {
            let yes = b.block(&[]);
            let no = b.block(&[]);
            b.br_if(flag, (yes, &[]), (no, &[]));
            b.switch_to(yes);
            b.ret(last);
            b.switch_to(no);
            let other = numbers[0];
            b.ret(other);
        }
        None => b.ret(last),
    }

    b.finish()
}

/// Many programs in one module.
///
/// This is the shape the oracle wants, and the reason is measured rather than assumed:
/// compiling a small program costs about 352 times what running it costs, so putting a
/// hundred programs in one module turns a hundred compilations into one. Each is still
/// called and compared on its own; only the expensive part is shared.
pub fn batch(seeds: &[u64]) -> Module {
    let mut module = Module::new();

    // One helper, so generated programs can contain calls. It is not itself compared;
    // it exists to be called.
    let helper_id = module.next_id();
    let mut h = Builder::new("helper", &[], Ty::I64);
    let a = h.const_i64(6_364_136_223_846_793_005);
    let b_ = h.const_i64(1_442_695_040_888_963_407);
    let mixed = h.add(a, b_);
    h.ret(mixed);
    module.add(h.finish());

    for &seed in seeds {
        module.add(program(seed, Some(helper_id)));
    }

    // A module wants an entry even when the oracle calls each program by name.
    let mut s = Builder::new(quench_qir::ENTRY, &[], Ty::I64);
    let zero = s.const_i64(0);
    s.ret(zero);
    let start = module.add(s.finish());
    module.set_entry(start);

    module
}
