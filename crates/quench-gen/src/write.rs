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

use quench_conf::{Characters, Division, Logic, MinMax, NoNumber, Overflow, Settings};
use quench_qir::{
    BinOp, Builder, CmpOp, FuncId, Function, Host, Module, Reading, Ty, Value,
};

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
        characters: if rng.upto(2) == 0 { Characters::Clusters } else { Characters::Letters },
        min_max: if rng.upto(2) == 0 { MinMax::Skips } else { MinMax::Spreads },
        overflow: if rng.upto(2) == 0 { Overflow::Wrap } else { Overflow::Trap },
        ..Settings::default()
    }
}

/// One program: a function taking nothing and giving back an `i64`.
///
/// `helper`, when given, is something the program may call — which is how a generated
/// program exercises calls at all without being able to recurse.
pub fn program(
    module: &mut Module,
    seed: u64,
    helper: Option<FuncId>,
    floating: &[(FuncId, Ty)],
) -> Function {
    program_under(module, seed, helper, floating, settings_for(seed))
}

/// The same, under settings chosen by the caller.
///
/// The module comes in because a decimal is read from the text it was written with, and
/// that text lives in the module's table.
pub fn program_under(
    module: &mut Module,
    seed: u64,
    helper: Option<FuncId>,
    floating: &[(FuncId, Ty)],
    settings: Settings,
) -> Function {
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

    // One binary width per program, so a seed sweeps all three rather than mixing them
    // in a way no source could. Floats compare by their *bits* in the oracle, which is
    // a sharper check than the rest of it gets: a fused multiply-add or a flushed
    // denormal is a different bit pattern, not a different-looking number.
    let (float_ty, half) = match rng.upto(3) {
        0 => (Ty::F64, false),
        1 => (Ty::F32, false),
        _ => (Ty::F16, true),
    };
    let carrier = |x: f64| -> u64 {
        match float_ty {
            Ty::F64 => x.to_bits(),
            _ => u64::from(quench_num::to_b16_or_f32(x as f32, half).to_bits()),
        }
    };
    let mut floats: Vec<Value> = vec![
        b.const_float(carrier(0.5), float_ty),
        b.const_float(carrier(rng.upto(1000) as f64 - 500.0), float_ty),
        b.const_float(carrier(0.0), float_ty),
        b.const_float(carrier(-1.0), float_ty),
    ];

    // One decimal format per program, for the same reason there is one binary width:
    // a seed sweeps both rather than mixing them in a way no source could.
    let keep = if rng.upto(2) == 0 { 7i64 } else { 16 };
    let digits = b.const_i64(keep);
    let written = |b: &mut Builder, module: &mut Module, text: &str| {
        let at = module.intern(text);
        let piece = b.const_text(at);
        let held = b.const_i64(keep);
        b.call_host(Host::DecimalRead, &[piece, held])
    };
    // The same for an `e`, which is read from text for the same reason: what it reads
    // to is a ratio of unbounded integers and does not fit in anything the IR carries.
    let exactly = |b: &mut Builder, module: &mut Module, text: &str| {
        let at = module.intern(text);
        let piece = b.const_text(at);
        b.call_host(Host::ExactRead, &[piece])
    };
    // Written values rather than made ones, because reading is what a source does and
    // the cohort a literal arrives with is part of what the arithmetic has to keep.
    let mut decimals: Vec<Value> = vec![
        written(&mut b, module, "0.1"),
        written(&mut b, module, "2.50"),
        written(&mut b, module, "0"),
        written(&mut b, module, "-1"),
        written(&mut b, module, &format!("{}", rng.upto(1000) as i64 - 500)),
    ];

    // Exact numbers, read from text the way a program's would be. A ratio, a decimal
    // point that is exact here, a whole number and a negative one.
    let mut exacts: Vec<Value> = vec![
        exactly(&mut b, module, "1/3"),
        exactly(&mut b, module, "0.1"),
        exactly(&mut b, module, &format!("{}", rng.upto(1000) as i64 - 500)),
        exactly(&mut b, module, "-1"),
    ];

    // Pieces of text, including two that differ only past the end of the shorter, since
    // that is the comparison a length check would get wrong.
    let mut texts: Vec<Value> = [
        "",
        "a",
        "ab",
        "b",
        // Two whose length depends on the answer: `é` written as `e` and a combining
        // acute is one cluster and two scalars, and the family is one and seven.
        "e\u{0301}",
        "\u{1F9D1}\u{200D}\u{1F9D1}\u{200D}\u{1F9D2}\u{200D}\u{1F9D2}",
    ]
        .iter()
        .map(|t| {
            let at = module.intern(t);
            b.const_text(at)
        })
        .collect();

    // Text shaped like a number, kept in its own list as well as in the one above.
    // `as` picks from here most of the time, so that it usually answers rather than
    // usually stopping — a step that stops every program before its third instruction
    // is a step that hides everything after it. `200` fits a `u8` and not an `i8`,
    // `2.5` is no whole number, `3/4` is an `e` and nothing else, and `true` is only
    // ever a `bool`, so which type was asked for still decides the answer.
    let numberish: Vec<Value> = ["42", "-7", "200", "2.5", "3/4", "true"]
        .iter()
        .map(|t| {
            let at = module.intern(t);
            b.const_text(at)
        })
        .collect();
    texts.extend_from_slice(&numberish);

    // One array to start with, for the same reason there are numbers and floats to
    // start with: a program whose first array step has to allocate before it can index
    // spends most of its array steps allocating and never reaches the rest.
    let mut handles: Vec<Value> = {
        let long = b.const_i64(rng.upto(6) as i64 + 1);
        let kind = b.const_i64(0); // `Elements::I64`
        let deep = b.const_i64(0);
        vec![b.call_host(Host::ArrayNew, &[long, kind, deep])]
    };

    let steps = rng.upto(30) + 6;
    for _ in 0..steps {
        match rng.upto(27) {
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
            17 => {
                // `exp`, `ln` and `pow`. Rare on purpose: each is worked out at a
                // hundred bits and more, which costs a thousand times what an addition
                // does, and a generated program that spent its life in one would test
                // that one thing very thoroughly and nothing else at all.
                if float_ty == Ty::F64 && rng.upto(4) == 0 {
                    let x = rng.pick(&floats);
                    let made = if rng.upto(3) == 0 {
                        let y = rng.pick(&floats);
                        let which = b.const_i64(rng.upto(3) as i64);
                        b.call_host_giving(Host::FloatPower, &[x, y, which], Ty::F64)
                    } else {
                        let which = b.const_i64(rng.upto(15) as i64);
                        b.call_host_giving(Host::FloatSlow, &[x, which], Ty::F64)
                    };
                    floats.push(made);
                }
            }
            16 => {
                // The maths IEEE requires. Every engine must give identical bits, so
                // this is the cheapest coverage there is -- and the one place where a
                // shared implementation is the *point* rather than a risk, because the
                // standard says what the answer is.
                let x = rng.pick(&floats);
                let width = b.const_i64(match float_ty {
                    Ty::F64 => 64,
                    Ty::F32 => 32,
                    _ => 16,
                });
                let mut made = match rng.upto(3) {
                    0 => {
                        let which = b.const_i64(rng.upto(6) as i64);
                        b.call_host_giving(Host::FloatAlone, &[x, which, width], float_ty)
                    }
                    1 => {
                        let y = rng.pick(&floats);
                        let pick = rng.upto(4) as i64;
                        let spreading = settings.min_max == MinMax::Spreads;
                        let which = b.const_i64(match (pick, spreading) {
                            (1, true) => 4,
                            (2, true) => 5,
                            (other, _) => other,
                        });
                        b.call_host_giving(Host::FloatPaired, &[x, y, which, width], float_ty)
                    }
                    _ => {
                        let (y, z) = (rng.pick(&floats), rng.pick(&floats));
                        b.call_host_giving(Host::FloatFused, &[x, y, z, width], float_ty)
                    }
                };
                if half {
                    made = b.call_host_giving(Host::ToB16, &[made], float_ty);
                }
                floats.push(made);
            }
            15 => {
                // `stitch`: the same formatting a `print` does, handed back instead of
                // written. Two engines could agree on every printed character and still
                // differ here, because this is a different code path to the same
                // answer -- and the answer becomes a piece of text the program can then
                // compare, which is how it reaches the number a run is checked on.
                let stream = b.const_i64(0);
                let said = match rng.upto(7) {
                    0 => {
                        let n = rng.pick(&numbers);
                        b.call_host(Host::SayI64, &[n])
                    }
                    1 => {
                        let n = rng.pick(&numbers);
                        b.call_host(Host::SayU64, &[n])
                    }
                    2 => {
                        let x = rng.pick(&floats);
                        let width = b.const_i64(match float_ty {
                            Ty::F64 => 64,
                            Ty::F32 => 32,
                            _ => 16,
                        });
                        b.call_host(Host::SayFloat, &[x, width])
                    }
                    3 => {
                        let d = rng.pick(&decimals);
                        b.call_host(Host::SayDecimal, &[d])
                    }
                    4 => {
                        let e = rng.pick(&exacts);
                        b.call_host(Host::SayExact, &[e])
                    }
                    5 => {
                        let Some(&flag) = flags.last() else { continue };
                        b.call_host(Host::SayBool, &[flag])
                    }
                    _ => {
                        let handle = rng.pick(&handles);
                        let kind = b.const_i64(0); // `Elements::I64`
                        let deep = b.const_i64(0);
                        b.call_host(Host::SayArray, &[handle, kind, deep])
                    }
                };
                texts.push(said);

                // Written as well as kept, sometimes, so that the two paths to the same
                // characters are both on the page and a difference between them shows.
                if rng.upto(2) == 0 {
                    b.call_host(Host::PrintText, &[stream, said]);
                }
            }
            14 => {
                // Printing, which is the only thing a program says that an answer
                // cannot carry -- and the one path the oracle could not see until it
                // started capturing. Every number type has its own way of being written
                // down, and each of those is code that could differ between engines
                // while every arithmetic answer agreed.
                let stream = b.const_i64(0); // standard output
                match rng.upto(7) {
                    0 => {
                        let n = rng.pick(&numbers);
                        b.call_host(Host::PrintI64, &[stream, n]);
                    }
                    // The same bits read as unsigned, which is a different set of
                    // characters for every number with its top bit set.
                    1 => {
                        let n = rng.pick(&numbers);
                        b.call_host(Host::PrintU64, &[stream, n]);
                    }
                    // The shortest text that reads back as the same bits, which is the
                    // hardest thing on this list to get identical twice.
                    2 => {
                        let x = rng.pick(&floats);
                        let width = b.const_i64(match float_ty {
                            Ty::F64 => 64,
                            Ty::F32 => 32,
                            _ => 16,
                        });
                        b.call_host(Host::PrintFloat, &[stream, x, width]);
                    }
                    3 => {
                        let d = rng.pick(&decimals);
                        b.call_host(Host::PrintDecimal, &[stream, d]);
                    }
                    4 => {
                        let e = rng.pick(&exacts);
                        b.call_host(Host::PrintExact, &[stream, e]);
                    }
                    5 => {
                        let t = rng.pick(&texts);
                        b.call_host(Host::PrintText, &[stream, t]);
                    }
                    _ => {
                        let handle = rng.pick(&handles);
                        let kind = b.const_i64(0); // `Elements::I64`
                        let deep = b.const_i64(0);
                        b.call_host(Host::PrintArray, &[stream, handle, kind, deep]);
                    }
                }
                // A flag, sometimes, so that what a program prints and what it answers
                // are not two unrelated halves of it.
                if rng.upto(4) == 0 {
                    let (l, r) = (rng.pick(&numbers), rng.pick(&numbers));
                    flags.push(b.cmp(CmpOp::Lt, l, r));
                }
            }
            13 => {
                // `e` and text, the last two things that allocate and the last two the
                // oracle had never seen. Both engines call the same code to work an `e`
                // out, as they do for a decimal -- so what this checks is the plumbing
                // around it: a handle surviving a call, and a comparison becoming a flag.
                if rng.upto(2) == 0 {
                    let (l, r) = (rng.pick(&exacts), rng.pick(&exacts));
                    let made = match rng.upto(6) {
                        0 => b.call_host(Host::ExactAdd, &[l, r]),
                        1 => b.call_host(Host::ExactSub, &[l, r]),
                        2 => b.call_host(Host::ExactMul, &[l, r]),
                        // Dividing by nought stops here, unlike a decimal: a ratio has
                        // no infinity to hand back. Kept rare for the usual reason.
                        3 => {
                            let divisor =
                                if rng.upto(8) == 0 { r } else { exactly(&mut b, module, "3") };
                            b.call_host(Host::ExactDiv, &[l, divisor])
                        }
                        // A whole exponent, and sometimes a negative one, which is
                        // where an `e` parts company with an `i64` -- and sometimes a
                        // fraction, whose answer is generally not a ratio and which is
                        // therefore a stop. Nothing wrote one of those until now, so
                        // one of the nine reasons to stop was implemented twice and
                        // compared never.
                        4 => {
                            let written = if rng.upto(4) == 0 {
                                "1/2".to_string()
                            } else {
                                format!("{}", rng.upto(6) as i64 - 2)
                            };
                            let exponent = exactly(&mut b, module, &written);
                            b.call_host(Host::ExactPow, &[l, exponent])
                        }
                        _ => {
                            let sign = b.call_host(Host::ExactCompare, &[l, r]);
                            let zero = b.const_i64(0);
                            let how = match rng.upto(3) {
                                0 => CmpOp::Lt,
                                1 => CmpOp::Eq,
                                _ => CmpOp::Gt,
                            };
                            flags.push(b.cmp(how, sign, zero));
                            continue;
                        }
                    };
                    exacts.push(made);
                    continue;
                }

                // Taking text apart. Three of these can stop -- a position outside the
                // text, a needle that is not there, a separator with nothing in it --
                // and the first two are reached by ordinary numbers and ordinary
                // needles rather than by anything aimed, because the pools hold both.
                //
                // One in four rather than one in three: the shapes already here have a
                // floor to clear in `reaches.rs`, and taking a third of the text step
                // put `text-compare` under it.
                if rng.upto(4) == 0 {
                    let said = rng.pick(&texts);
                    match rng.upto(5) {
                        0 => {
                            let (from, to) = (rng.pick(&numbers), rng.pick(&numbers));
                            let host = match settings.characters {
                                Characters::Clusters => Host::TextSliceClusters,
                                Characters::Letters => Host::TextSliceLetters,
                            };
                            texts.push(b.call_host(host, &[said, from, to]));
                        }
                        1 => {
                            let sub = rng.pick(&texts);
                            let host = match settings.characters {
                                Characters::Clusters => Host::TextFindClusters,
                                Characters::Letters => Host::TextFindLetters,
                            };
                            numbers.push(b.call_host(host, &[said, sub]));
                        }
                        2 => {
                            let sub = rng.pick(&texts);
                            flags.push(b.call_host(Host::TextHas, &[said, sub]));
                        }
                        3 => {
                            // Counted rather than kept: the handle pool holds arrays of
                            // `i64` and this one holds text, so putting it there would
                            // hand a later `array-get` the wrong element kind.
                            //
                            // A separator with nothing in it is aimed at rather than
                            // waited for. One text in twelve is empty, and one branch in
                            // twenty reaches here, which is a stop the oracle would meet
                            // about once a run -- which is to say, not reliably at all.
                            let sep = if rng.upto(4) == 0 {
                                let at = module.intern("");
                                b.const_text(at)
                            } else {
                                rng.pick(&texts)
                            };
                            let cut = b.call_host(Host::TextSplit, &[said, sep]);
                            numbers.push(b.call_host(Host::ArrayLen, &[cut]));
                        }
                        _ => texts.push(b.call_host(Host::TextTrim, &[said])),
                    }
                    continue;
                }

                // Reading a number back out of text. `is` never stops and `as` does,
                // and both are written: stopping in the same place for the same reason
                // is as much an agreement as answering with the same number.
                if rng.upto(2) == 0 {
                    let width = match float_ty {
                        Ty::F64 => 64,
                        Ty::F32 => 32,
                        _ => 16,
                    };
                    // Two in three ask, and the asking sweeps every type — including
                    // the narrow whole numbers, where whether `200` fits is the whole
                    // question and no answer has to be carried anywhere.
                    if rng.upto(2) == 0 {
                        let said = rng.pick(&texts);
                        let (kind, first, second) = match rng.upto(5) {
                            0 => (
                                Reading::Whole,
                                [8i64, 16, 32, 64][rng.upto(4)],
                                rng.upto(2) as i64,
                            ),
                            1 => (Reading::Float, width, 0),
                            2 => (Reading::Exact, 0, 0),
                            3 => (Reading::Decimal, keep, 0),
                            _ => (Reading::Bool, 0, 0),
                        };
                        let kind = b.const_i64(kind as i64);
                        let first = b.const_i64(first);
                        let second = b.const_i64(second);
                        let yes =
                            b.call_host(Host::TextReads, &[said, kind, first, second]);
                        flags.push(yes);
                        continue;
                    }
                    // And the other half answers, at the width and format this
                    // program uses, so that what comes back belongs in the pool it goes
                    // to. Mostly on text that reads, and sometimes on text that does
                    // not, which is the trap.
                    let said =
                        if rng.upto(4) == 0 { rng.pick(&texts) } else { rng.pick(&numberish) };
                    match rng.upto(5) {
                        0 => {
                            let bits = b.const_i64(64);
                            let signed = b.const_i64(1);
                            let read =
                                b.call_host(Host::TextAsWhole, &[said, bits, signed]);
                            numbers.push(read);
                        }
                        1 => {
                            let said_width = b.const_i64(width);
                            let read = b.call_host_giving(
                                Host::TextAsFloat,
                                &[said, said_width],
                                float_ty,
                            );
                            floats.push(read);
                        }
                        2 => exacts.push(b.call_host(Host::TextAsExact, &[said])),
                        3 => {
                            let held = b.const_i64(keep);
                            decimals
                                .push(b.call_host(Host::TextAsDecimal, &[said, held]));
                        }
                        _ => flags.push(b.call_host(Host::TextAsBool, &[said])),
                    }
                    continue;
                }

                // Text, whose one arithmetic is joining and whose one question is which
                // of two comes first. Joined pieces go past the end of the module's
                // table into one the runtime keeps, which is the part worth checking.
                let (l, r) = (rng.pick(&texts), rng.pick(&texts));
                if rng.upto(2) == 0 {
                    let joined = b.call_host(Host::TextJoin, &[l, r]);
                    texts.push(joined);
                    continue;
                }
                if rng.upto(3) == 0 {
                    let host = match settings.characters {
                        Characters::Clusters => Host::TextClusters,
                        Characters::Letters => Host::TextLetters,
                    };
                    let how_many = b.call_host(host, &[l]);
                    numbers.push(how_many);
                    continue;
                }
                let order = b.call_host(Host::TextCompare, &[l, r]);
                let zero = b.const_i64(0);
                let how = match rng.upto(3) {
                    0 => CmpOp::Lt,
                    1 => CmpOp::Eq,
                    _ => CmpOp::Gt,
                };
                flags.push(b.cmp(how, order, zero));
            }
            12 => {
                // Arrays, which nothing generated had ever asked for -- and with them
                // the collector, and the Dev JIT's shadow root slots, which are its own
                // machinery and have no counterpart in the interpreter. Allocation is
                // the one thing here that happens *between* the two engines rather than
                // inside each, so it is the last place they could quietly differ.
                let kind = b.const_i64(0); // `Elements::I64`
                let deep = b.const_i64(0);

                if rng.upto(5) == 0 {
                    let long = b.const_i64(rng.upto(6) as i64 + 1);
                    let made = b.call_host(Host::ArrayNew, &[long, kind, deep]);
                    handles.push(made);
                    continue;
                }

                let handle = rng.pick(&handles);
                match rng.upto(6) {
                    // Mostly in range, because a program that stops on its first index
                    // exercises one call and then nothing. One in eight is whatever
                    // turned up, which is how the bounds trap gets written at all.
                    0 => {
                        let at = if rng.upto(8) == 0 {
                            rng.pick(&numbers)
                        } else {
                            b.const_i64(1)
                        };
                        let got = b.call_host(Host::ArrayGet, &[handle, at]);
                        numbers.push(got);
                    }
                    1 => {
                        let at = if rng.upto(8) == 0 {
                            rng.pick(&numbers)
                        } else {
                            b.const_i64(1)
                        };
                        let value = rng.pick(&numbers);
                        b.call_host(Host::ArraySet, &[handle, at, value]);
                    }
                    2 => {
                        let long = b.call_host(Host::ArrayLen, &[handle]);
                        numbers.push(long);
                    }
                    // Growing, which is where an array is reallocated underneath every
                    // name that reaches it.
                    3 => {
                        let value = rng.pick(&numbers);
                        b.call_host(Host::ArrayPush, &[handle, value]);
                    }
                    // A second array holding the same things, which is the one way to
                    // find out whether a collection freed a row that was still live.
                    4 => {
                        let copied = b.call_host(Host::ArrayCopy, &[handle]);
                        let same =
                            b.call_host(Host::ArrayEqual, &[handle, copied, kind, deep]);
                        handles.push(copied);
                        flags.push(same);
                    }
                    _ => {
                        let other = rng.pick(&handles);
                        let same =
                            b.call_host(Host::ArrayEqual, &[handle, other, kind, deep]);
                        flags.push(same);
                    }
                }
            }
            11 => {
                // The unsigned side of whole numbers, which nothing generated reached.
                // A `u64` is the one width whose comparison, division and overflow have
                // to read the same bits as unsigned rather than being put back inside a
                // narrower type afterwards -- so it is the one width where the
                // *operation* differs, and the only one an engine could get wrong on
                // its own.
                let lhs = rng.pick(&numbers);
                let (add, sub, mul) = match settings.overflow {
                    Overflow::Wrap => (BinOp::Add, BinOp::Sub, BinOp::Mul),
                    Overflow::Trap => {
                        (BinOp::AddTrappingU, BinOp::SubTrappingU, BinOp::MulTrappingU)
                    }
                };
                let value = match rng.upto(5) {
                    0 => b.bin(add, lhs, rng.pick(&numbers)),
                    1 => b.bin(sub, lhs, rng.pick(&numbers)),
                    2 => b.bin(mul, lhs, rng.pick(&numbers)),
                    // A divisor of nought stops here as it does anywhere, and one in
                    // eight is whatever turned up so that it can.
                    op => {
                        let divisor = if rng.upto(8) == 0 {
                            rng.pick(&numbers)
                        } else {
                            b.const_i64(rng.upto(9_999) as i64 + 2)
                        };
                        b.bin(if op == 3 { BinOp::DivU } else { BinOp::RemU }, lhs, divisor)
                    }
                };
                numbers.push(value);
                // And the unsigned comparison, which reads the same bits differently:
                // `-1` is the largest number there is when it is read as a `u64`.
                if rng.upto(3) == 0 {
                    let (l, r) = (rng.pick(&numbers), rng.pick(&numbers));
                    let how = match rng.upto(4) {
                        0 => CmpOp::Lt,
                        1 => CmpOp::Le,
                        2 => CmpOp::Gt,
                        _ => CmpOp::Ge,
                    };
                    flags.push(b.cmp_unsigned(how, l, r));
                }
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
            9 => {
                // Plain IEEE, and the narrow one put back in its own set afterwards.
                let (lhs, rhs) = (rng.pick(&floats), rng.pick(&floats));
                let stops = settings.no_number == quench_conf::NoNumber::Stops;
                let op = match (rng.upto(4), stops) {
                    (0, false) => BinOp::FAdd,
                    (1, false) => BinOp::FSub,
                    (2, false) => BinOp::FMul,
                    (_, false) => BinOp::FDiv,
                    (0, true) => BinOp::FAddChecked,
                    (1, true) => BinOp::FSubChecked,
                    (2, true) => BinOp::FMulChecked,
                    (_, true) => BinOp::FDivChecked,
                };
                let mut value = b.bin(op, lhs, rhs);
                if half {
                    value = b.call_host_giving(Host::ToB16, &[value], float_ty);
                }
                floats.push(value);
                // And a comparison, so a float can reach the answer: a program hands
                // back an `i64`, and a flag is how the two meet.
                if rng.upto(3) == 0 {
                    let how = match rng.upto(6) {
                        0 => CmpOp::Eq,
                        1 => CmpOp::Ne,
                        2 => CmpOp::Lt,
                        3 => CmpOp::Le,
                        4 => CmpOp::Gt,
                        _ => CmpOp::Ge,
                    };
                    flags.push(b.fcmp(how, lhs, rhs));
                }
            }
            10 => {
                // Every decimal answer is rounded to the format's digits, so this
                // checks the plumbing rather than the arithmetic: both engines call the
                // same code to work one out, and could still differ in how a handle is
                // kept alive across the call or how a comparison is turned into a flag.
                let (lhs, rhs) = (rng.pick(&decimals), rng.pick(&decimals));
                let host = match rng.upto(4) {
                    0 => Host::DecimalAdd,
                    1 => Host::DecimalSub,
                    2 => Host::DecimalMul,
                    // Nought is left in on purpose: a decimal division by nought is
                    // infinity rather than a trap, and that is the difference from `e`.
                    _ => Host::DecimalDiv,
                };
                decimals.push(b.call_host(host, &[lhs, rhs, digits]));

                // Sometimes a literal, so a program keeps meeting values it has not
                // worked out -- and so the read itself is exercised more than five
                // times per program.
                if rng.upto(4) == 0 {
                    let text = match rng.upto(5) {
                        0 => "1E+90".to_string(),
                        1 => "1E-90".to_string(),
                        2 => "0.000".to_string(),
                        3 => "9999999999999999".to_string(),
                        _ => format!("{}.{}", rng.upto(1000), rng.upto(1000)),
                    };
                    let made = written(&mut b, module, &text);
                    decimals.push(made);
                }

                // And a comparison, so a decimal can reach the answer: a program hands
                // back an `i64`, and a flag is how the two meet. These are the shapes
                // the lowering writes, including the two that a not-a-number makes
                // awkward -- `<==` and `>==` cannot be one comparison against one
                // number, because unordered is a fourth answer.
                if rng.upto(3) == 0 {
                    let how = b.call_host(Host::DecimalCompare, &[lhs, rhs]);
                    let unordered = b.const_i64(2);
                    let flag = match rng.upto(6) {
                        0 => {
                            let zero = b.const_i64(0);
                            b.cmp(CmpOp::Eq, how, zero)
                        }
                        1 => {
                            let zero = b.const_i64(0);
                            b.cmp(CmpOp::Ne, how, zero)
                        }
                        2 => {
                            let less = b.const_i64(-1);
                            b.cmp(CmpOp::Eq, how, less)
                        }
                        3 => {
                            let more = b.const_i64(1);
                            b.cmp(CmpOp::Eq, how, more)
                        }
                        4 => {
                            let more = b.const_i64(1);
                            let above = b.cmp(CmpOp::Eq, how, more);
                            let strange = b.cmp(CmpOp::Eq, how, unordered);
                            let out = b.bin(BinOp::Or, above, strange);
                            b.not(out)
                        }
                        _ => {
                            let less = b.const_i64(-1);
                            let below = b.cmp(CmpOp::Eq, how, less);
                            let strange = b.cmp(CmpOp::Eq, how, unordered);
                            let out = b.bin(BinOp::Or, below, strange);
                            b.not(out)
                        }
                    };
                    flags.push(flag);
                }
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
            18 => {
                // What arrived from outside, which for a generated program is nothing:
                // both engines are handed empty input on purpose, so `more` is false,
                // `all` is empty text and `line` stops. Which is the point -- that stop
                // is a thing the two must agree about, and this is the only way the
                // oracle ever reaches it.
                //
                // An arm of its own rather than a slice of the text step, because it is
                // not a text operation and taking it from there put `text-compare` under
                // the floor `reaches.rs` holds it to.
                match rng.upto(4) {
                    0 => texts.push(b.call_host(Host::InputAll, &[])),
                    1 => flags.push(b.call_host(Host::InputMore, &[])),
                    2 => texts.push(b.call_host(Host::InputLine, &[])),
                    _ => {
                        let all = b.call_host(Host::InputArguments, &[]);
                        numbers.push(b.call_host(Host::ArrayLen, &[all]));
                    }
                }
            }
            8 => {
                // A call whose signature carries a float, when there is one of this
                // program's width to call: a parameter and a return type are places a
                // type appears that the body never reaches on its own.
                if rng.upto(2) == 0
                    && let Some((id, _)) = floating.iter().find(|(_, ty)| *ty == float_ty)
                {
                    let given = rng.pick(&floats);
                    let answer = b.call(*id, &[given], float_ty);
                    floats.push(answer);
                    continue;
                }
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

    // And one whose signature carries a float, in and out. A parameter and a return
    // type are the two places a type appears that a body never reaches, so a helper
    // that only ever took nothing and gave back an `i64` left them untested -- which is
    // how a function returning a `b64` crashed the Dev JIT's code generator while
    // 200,000 programs a run said everything agreed. One of them takes it in each
    // width, since a `b16` is carried in an `f32` and could have been the odd one.
    let mut floating = Vec::new();
    for (name, ty) in [("halved64", Ty::F64), ("halved32", Ty::F32), ("halved16", Ty::F16)] {
        let id = module.next_id();
        let mut h = Builder::new(name, &[ty], ty);
        let x = h.param(0);
        let half = h.const_float(
            match ty {
                Ty::F64 => 0.5f64.to_bits(),
                _ => u64::from(0.5f32.to_bits()),
            },
            ty,
        );
        let scaled = h.bin(BinOp::FMul, x, half);
        h.ret(scaled);
        module.add(h.finish());
        floating.push((id, ty));
    }

    for &seed in seeds {
        let written = program(&mut module, seed, Some(helper_id), &floating);
        module.add(written);
    }

    // A module wants an entry even when the oracle calls each program by name.
    let mut s = Builder::new(quench_qir::ENTRY, &[], Ty::I64);
    let zero = s.const_i64(0);
    s.ret(zero);
    let start = module.add(s.finish());
    module.set_entry(start);

    module
}
