//! Writing programs, so that the ways of running one can be made to disagree.
//!
//! A generated program is **built from the types outward**, so it always checks out.
//! That is the whole point. A program that failed to check would be rejected identically
//! by every engine and would prove nothing — the interesting programs are the ones that
//! *run*, because those are the ones two implementations can answer differently.
//!
//! Inherited from Luarust, which worked this out first.
//!
//! # What it writes, and what it does not
//!
//! - **Programs that stop.** Division and remainder are sometimes given a divisor that
//!   can be zero, or `-1` against the smallest `i64`, so a generated program can stop
//!   rather than answer. Stopping in the same place for the same reason is exactly as
//!   much an agreement as printing the same number, and until compiled code could
//!   report a stop rather than abort the process, this could not be checked at all.
//! - **Large loops.** A fuzzer that takes a minute per program is a fuzzer nobody runs,
//!   and the whole economics of the oracle rests on programs being small: compiling one
//!   costs a few hundred times what running it costs.
//! - **Recursion without a floor.** A runaway call is reported by the interpreter and
//!   overflows the stack in compiled code, so the two cannot be compared on it.

use quench_conf::{Division, Logic, NoNumber, Overflow, Settings};
use quench_qir::{BinOp, Builder, CmpOp, FuncId, Function, Host, Module, Ty, Value};

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

/// Which configuration a seed is checked under.
///
/// A seed picks a program **and** the settings it means something under, because a
/// semantic setting is not a variation on a language — it is a different language, and
/// a bug that only appears under one of them is found only if something generated it.
/// See `notes/every-knob-is-a-multiplier.md`.
pub fn settings_for(seed: u64) -> Settings {
    let mut rng = Seeded::from(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    Settings {
        division: if rng.upto(2) == 0 { Division::Truncated } else { Division::Floored },
        logic: if rng.upto(2) == 0 { Logic::StopsEarly } else { Logic::AsksBoth },
        no_number: if rng.upto(2) == 0 { NoNumber::CarriesOn } else { NoNumber::Stops },
        overflow: if rng.upto(2) == 0 { Overflow::Wrap } else { Overflow::Trap },
        ..Settings::default()
    }
}

/// One program: a function taking nothing and giving back an `i64`.
///
/// `helper`, when given, is something the program may call — which is how a generated
/// program exercises calls at all without being able to recurse.
pub fn program(seed: u64, helper: Option<FuncId>) -> Function {
    program_under(seed, helper, settings_for(seed))
}

/// The same, under settings chosen by the caller.
pub fn program_under(seed: u64, helper: Option<FuncId>, settings: Settings) -> Function {
    let mut rng = Seeded::from(seed);
    let mut b = Builder::new(name_of(seed), &[], Ty::I64);

    // Mostly small, because a program built out of enormous numbers overflows on its
    // first multiplication and then tests nothing else. The extremes are here because
    // they are where the edges are -- `i64::MIN` has no positive counterpart, and
    // `MIN / -1` is the one division that does not fit -- but they are the minority on
    // purpose, so a program reaches them rather than starting there.
    let mut numbers = vec![
        b.const_i64(rng.upto(1000) as i64),
        b.const_i64(rng.upto(1000) as i64 - 500),
        b.const_i64(0),
        b.const_i64(1),
        b.const_i64(-1),
    ];
    let mut flags: Vec<Value> = Vec::new();

    let steps = rng.upto(30) + 6;
    for _ in 0..steps {
        match rng.upto(10) {
            0 => {
                // The extremes are where the edges are -- `i64::MIN` has no positive
                // counterpart, and `MIN / -1` is the one division that does not fit --
                // so they have to appear. But they are kept rare, because a trapping
                // sum touching one stops immediately, and a program that stops on its
                // first operation exercises one instruction and then nothing.
                let n = match rng.upto(16) {
                    0 => b.const_i64(i64::MIN),
                    1 => b.const_i64(i64::MAX),
                    2 => b.const_i64(rng.next() as i64),
                    _ => b.const_i64(rng.upto(2000) as i64 - 1000),
                };
                numbers.push(n);
            }
            1..=4 => {
                let lhs = rng.pick(&numbers);
                // Which division a program gets is the project's decision, written down
                // as an instruction by the time it reaches here.
                let (divide, remainder) = match settings.division {
                    Division::Truncated => (BinOp::DivTruncated, BinOp::RemTruncated),
                    Division::Floored => (BinOp::DivFloored, BinOp::RemFloored),
                };
                // Whether a sum that does not fit rounds or stops is the project's
                // decision too, so a seed picks it and the oracle checks both.
                let (add, sub, mul) = match settings.overflow {
                    Overflow::Wrap => (BinOp::Add, BinOp::Sub, BinOp::Mul),
                    Overflow::Trap => {
                        (BinOp::AddTrapping, BinOp::SubTrapping, BinOp::MulTrapping)
                    }
                };
                // A power, sometimes. Its exponent is kept small and is sometimes
                // negative on purpose, because a whole number raised to a negative
                // power is a trap and a trap the generator never writes is a trap the
                // oracle never checks.
                if rng.upto(12) == 0 {
                    let host = match settings.overflow {
                        Overflow::Wrap => Host::PowI64,
                        Overflow::Trap => Host::PowI64Trapping,
                    };
                    let exponent = match rng.upto(8) {
                        0 => b.const_i64(rng.upto(4) as i64 - 3),
                        _ => b.const_i64(rng.upto(12) as i64),
                    };
                    let value = b.call_host(host, &[lhs, exponent]);
                    numbers.push(value);
                    continue;
                }
                let op = match rng.upto(5) {
                    0 => add,
                    1 => sub,
                    2 => mul,
                    3 => divide,
                    _ => remainder,
                };
                // The one division that does not fit is far too rare to turn up by
                // chance once the extremes are kept rare, so it is aimed at. A trap the
                // generator never writes is a trap the oracle never checks.
                if matches!(op, BinOp::DivTruncated | BinOp::DivFloored) && rng.upto(64) == 0 {
                    let least = b.const_i64(i64::MIN);
                    let minus_one = b.const_i64(-1);
                    let value = b.bin(op, least, minus_one);
                    numbers.push(value);
                    continue;
                }
                let value = if matches!(op, BinOp::DivTruncated | BinOp::RemTruncated | BinOp::DivFloored | BinOp::RemFloored) {
                    // Most divisors are safe, because a program that stops on its first
                    // division exercises one instruction and then nothing. One in eight
                    // is whatever turned up, which is how zero and -1 get in.
                    if rng.upto(8) == 0 {
                        let risky = rng.pick(&numbers);
                        b.bin(op, lhs, risky)
                    } else {
                        let safe = b.const_i64(rng.upto(9_999) as i64 + 2);
                        b.bin(op, lhs, safe)
                    }
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
                let Some(&flag) = flags.last() else { continue };
                if flags.len() < 2 || rng.upto(2) == 0 {
                    flags.push(b.not(flag));
                    continue;
                }
                // Two flags joined, in whichever of the two shapes the settings ask
                // for. They answer the same here -- a generated program has nothing in
                // it that a skipped side could have done -- so what this checks is that
                // both *shapes* compile and run alike, which is the part that differs.
                let other = flags[rng.upto(flags.len() as u64 - 1)];
                let both = rng.upto(2) == 0;
                if settings.logic == Logic::AsksBoth {
                    let op = if both { BinOp::And } else { BinOp::Or };
                    flags.push(b.bin(op, other, flag));
                    continue;
                }
                let join = b.block(&[Ty::Bool]);
                let rest = b.block(&[]);
                let settled = b.const_bool(!both);
                if both {
                    b.br_if(other, (rest, &[]), (join, &[settled]));
                } else {
                    b.br_if(other, (join, &[settled]), (rest, &[]));
                }
                b.switch_to(rest);
                b.jump(join, &[flag]);
                b.switch_to(join);
                let answer = b.block_param(join, 0);
                flags.push(answer);
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
                // Sometimes leave early. Without this the body has one way out and
                // `done` has one way in, which is the one loop shape a `break` never
                // makes -- and `break` is now a thing people write.
                if rng.upto(3) == 0 {
                    let enough = b.cmp(CmpOp::Ge, step, rng.pick(&numbers));
                    let carry_on = b.block(&[]);
                    b.br_if(enough, (done, &[step]), (carry_on, &[]));
                    b.switch_to(carry_on);
                }
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
