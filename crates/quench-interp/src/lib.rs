//! Running QIR directly.
//!
//! This is the engine that does the least. It generates no code, allocates no registers,
//! selects no instructions and lowers nothing — it reads QIR and does what it says. That
//! makes it the one to believe when the oracle finds a disagreement, on the same
//! principle that made the Dev JIT the reference before it existed: the fewest
//! transformations means the fewest places for a wrong answer to come from.
//!
//! It is also, for the programs an oracle actually runs, the **fastest** engine —
//! which is not obvious and was measured rather than assumed. Compiling a small program
//! with Cranelift costs about 103µs and running the result about 292ns, so compilation
//! is roughly 352x the execution. An interpreter skips the 103µs entirely. Even at a
//! hundred times native speed it finishes in a third of the time the Dev JIT spends
//! before it has run anything at all. See `crates/quench-dev/examples/cost.rs`.
//!
//! # Agreeing about stopping, not just about answers
//!
//! A program that divides by zero has to stop in the same place for the same reason in
//! every engine, which is as much an agreement as printing the same number. So the traps
//! here are not the interpreter's own opinion — they are chosen to match what Cranelift's
//! `sdiv` and `srem` do, because that is what the machine code will do.

use quench_heap::Heap;
use quench_qir as qir;
use std::io::Write;

pub use quench_heap::Heap as TheHeap;
pub use qir::{Outcome, Trap};

/// Why a module could not be run at all, as opposed to a program that ran and stopped.
#[derive(Debug)]
pub enum Error {
    Invalid(Vec<qir::Invalid>),
    NoEntry,
    Entry(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Invalid(wrong) => {
                writeln!(f, "the IR handed to the interpreter is not well formed:")?;
                for one in wrong {
                    writeln!(f, "  - {one}")?;
                }
                Ok(())
            }
            Error::NoEntry => write!(f, "the module names no entry, so there is nothing to run"),
            Error::Entry(why) => write!(f, "the entry cannot be called: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// How deep calls may nest before the interpreter gives up.
///
/// This is a number the interpreter chooses, not one the machine imposes: calls are
/// kept on a stack of its own rather than on Rust's, so a runaway program is *reported*
/// rather than crashing the process. That matters more here than the number does — an
/// oracle that dies on one seed loses the whole run, and cannot tell you which seed.
pub const DEPTH: usize = 10_000;

/// Where a running program's output goes.
///
/// Two of them, because a program says which it means. See [`qir::Stream`].
/// What one array says, following handles as far down as it goes.
fn shown(
    handle: i64,
    kind: qir::Elements,
    depth: i64,
    heap: &Heap,
) -> String {
    let parts: Vec<String> = heap.at(handle).values
        .iter()
        .map(|value| {
            if depth > 0 {
                return shown(*value, kind, depth - 1, heap);
            }
            match kind {
                qir::Elements::I64 => value.to_string(),
                qir::Elements::Bool => if *value != 0 { "true" } else { "false" }.to_string(),
                // Wearing its marks, because an array of text with a space in it is
                // unreadable without them.
                qir::Elements::Text => format!("*{}*", heap.said(*value)),
                qir::Elements::Exact => heap.exactly(*value).to_string(),
                qir::Elements::Decimal => heap.decimally(*value).to_string(),
                qir::Elements::Float => quench_num::show_f64(f64::from_bits(*value as u64)),
            }
        })
        .collect();
    qir::show_array(&parts)
}

/// Whether two arrays hold the same things, following handles as far down as they go.
fn alike(
    a: i64,
    b: i64,
    kind: qir::Elements,
    depth: i64,
    heap: &Heap,
) -> bool {
    let (left, right) = (&heap.at(a).values, &heap.at(b).values);
    if left.len() != right.len() {
        return false;
    }
    left.iter().zip(right.iter()).all(|(x, y)| {
        if depth > 0 {
            return alike(*x, *y, kind, depth - 1, heap);
        }
        match kind {
            // Two names for one exact number are not the only way to hold the same one,
            // so these are compared by value rather than by which they are -- and so is
            // text, for the same reason.
            qir::Elements::Exact => heap.exactly(*x) == heap.exactly(*y),
            // By what they are as well, and for a third reason on top of those two:
            // `2.50` and `2.5` are one number written two ways, and a not-a-number is
            // equal to nothing including itself.
            qir::Elements::Decimal => {
                heap.decimally(*x).compare(heap.decimally(*y))
                    == Some(std::cmp::Ordering::Equal)
            }
            // By what they are, not by their bits: two ways of writing nought are one
            // number, and a not-a-number is not even itself.
            qir::Elements::Float => {
                f64::from_bits(*x as u64) == f64::from_bits(*y as u64)
            }
            qir::Elements::Text => heap.said(*x) == heap.said(*y),
            _ => x == y,
        }
    })
}

/// Everything a running program could still reach.
///
/// One entry per slot of every frame whose type is a reference. The type comes from the
/// function the frame is running: QIR says what every value in it is, so a slot holding
/// `7` is told apart from a slot holding handle 7 without either of them saying so.
fn rooted(stack: &[Frame], module: &qir::Module) -> Vec<(qir::Ty, i64)> {
    let mut roots = Vec::new();
    for frame in stack {
        let func = module.func(frame.func);
        for (n, value) in frame.slots.iter().enumerate() {
            let ty = func.ty_of(qir::Value(n as u32));
            if matches!(
                ty,
                qir::Ty::Handle | qir::Ty::Text | qir::Ty::Exact | qir::Ty::Decimal
            ) {
                roots.push((ty, *value));
            }
        }
    }
    roots
}

/// A value put back inside a narrower integer type, normalised for its sign.
///
/// Signed types are sign-extended and unsigned ones zero-extended, so that whatever is
/// in a slot orders and prints the same however it got there. Written once here and
/// once in the Dev JIT, and the oracle is what says the two agree.
fn narrowed(value: i64, bits: u8, signed: bool) -> i64 {
    if bits >= 64 {
        return value;
    }
    let spare = 64 - u32::from(bits);
    if signed {
        (value << spare) >> spare
    } else {
        ((value as u64) << spare >> spare) as i64
    }
}

/// Why a power had no answer, as a reason to stop.
/// The maths functions, in the order the lowering writes their numbers.
const ALONE: [quench_num::Alone; 6] = [
    quench_num::Alone::Sqrt,
    quench_num::Alone::Abs,
    quench_num::Alone::Floor,
    quench_num::Alone::Ceiling,
    quench_num::Alone::Round,
    quench_num::Alone::Truncate,
];

const PAIRED: [quench_num::Paired; 3] = [
    quench_num::Paired::CopySign,
    quench_num::Paired::Minimum,
    quench_num::Paired::Maximum,
];

/// Which decimal format a digit count names. The lowering only ever writes the two.
fn decimal_format(digits: i64) -> quench_num::Format {
    if digits == 7 { quench_num::D32 } else { quench_num::D64 }
}

fn no_power(trouble: quench_num::NoPower) -> Trap {
    match trouble {
        quench_num::NoPower::Negative => Trap::NegativePower,
        quench_num::NoPower::Fractional => Trap::FractionalPower,
        quench_num::NoPower::TooLarge => Trap::Overflowed,
    }
}

pub struct Writing<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
}

impl Writing<'_> {
    fn to(&mut self, stream: i64) -> &mut dyn Write {
        if stream == qir::Stream::Err as i64 { self.err } else { self.out }
    }
}

/// Run a module from its entry, printing where the program said to.
pub fn run(module: &qir::Module) -> Result<Outcome, Error> {
    let (mut out, mut err) = (std::io::stdout(), std::io::stderr());
    run_writing(module, &mut Writing { out: &mut out, err: &mut err })
}

/// What the heap looked like when the program ended.
///
/// Nothing a program can see, and that is the point: a collector that changed what a
/// program answered would be a bug rather than a feature. It is here so that a test can
/// check the one thing the oracle cannot — that memory nothing can reach went away.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Kept {
    /// Arrays, pieces of text and exact numbers still alive at the end.
    pub live: (usize, usize, usize),
    pub collections: usize,
}

/// Run it, and say what the heap looked like afterwards as well as what it answered.
pub fn run_kept(module: &qir::Module) -> Result<(Outcome, Kept), Error> {
    let (mut out, mut err) = (std::io::sink(), std::io::sink());
    let id = module.entry.ok_or(Error::NoEntry)?;
    let mut writing = Writing { out: &mut out, err: &mut err };
    let mut heap = Heap::new(module);
    let outcome = match walk_with(module, id, &mut writing, &mut heap) {
        Ok(value) => Outcome::Returned(value),
        Err(trap) => Outcome::Trapped(trap),
    };
    Ok((outcome, Kept { live: heap.live(), collections: heap.collections }))
}

/// The same, sending whatever it prints somewhere of your choosing.
///
/// Which is how a test reads what a program said, and how the oracle compares what two
/// engines said rather than only what they returned.
pub fn run_writing(module: &qir::Module, writing: &mut Writing<'_>) -> Result<Outcome, Error> {
    let id = module.entry.ok_or(Error::NoEntry)?;
    run_id(module, id, writing)
}

/// Run one named function that takes nothing and returns an i64.
///
/// The oracle's other door: a module holds many generated programs, and this runs one
/// of them without the module having to name it as the entry.
pub fn run_named(module: &qir::Module, name: &str) -> Result<Outcome, Error> {
    let (mut out, mut err) = (std::io::sink(), std::io::sink());
    run_named_writing(module, name, &mut Writing { out: &mut out, err: &mut err })
}

/// The same, keeping what it printed.
pub fn run_named_writing(
    module: &qir::Module,
    name: &str,
    writing: &mut Writing<'_>,
) -> Result<Outcome, Error> {
    let id = module
        .find(name)
        .ok_or_else(|| Error::Entry(format!("there is no function called `{name}`")))?;
    run_id(module, id, writing)
}

fn run_id(module: &qir::Module, id: qir::FuncId, writing: &mut Writing<'_>) -> Result<Outcome, Error> {
    qir::verify(module).map_err(Error::Invalid)?;
    let entry = module.func(id);
    if !entry.params.is_empty() {
        return Err(Error::Entry(format!(
            "`{}` takes {} argument(s), and an entry is called with none",
            entry.name,
            entry.params.len()
        )));
    }
    if entry.ret != qir::Ty::I64 {
        return Err(Error::Entry(format!(
            "`{}` returns {}, and an entry has to return i64",
            entry.name,
            entry.ret.name()
        )));
    }

    Ok(match walk(module, id, writing) {
        Ok(value) => Outcome::Returned(value),
        Err(trap) => Outcome::Trapped(trap),
    })
}

/// One call in progress.
struct Frame {
    func: qir::FuncId,
    /// Every value in the function. A `bool` is 0 or 1 in one of these, which is exactly
    /// what Cranelift's `i8` holds after a comparison.
    slots: Vec<i64>,
    block: qir::BlockId,
    /// Which instruction to resume at, so a call can be left and come back to.
    at: usize,
    /// Where a callee's answer goes when it returns.
    pending: Option<qir::Value>,
}

impl Frame {
    fn new(module: &qir::Module, id: qir::FuncId, args: &[i64]) -> Frame {
        let func = module.func(id);
        let mut frame = Frame {
            func: id,
            slots: vec![0; func.value_tys.len()],
            block: func.entry(),
            at: 0,
            pending: None,
        };
        frame.enter(func, func.entry(), args);
        frame
    }

    /// Arrive at a block: bind its parameters and start at its first instruction.
    fn enter(&mut self, func: &qir::Function, block: qir::BlockId, args: &[i64]) {
        for (n, param) in func.block(block).params.iter().enumerate() {
            self.slots[param.0 as usize] = args[n];
        }
        self.block = block;
        self.at = 0;
    }
}

/// Run, with the calls on a stack here rather than on Rust's.
fn walk(module: &qir::Module, entry: qir::FuncId, writing: &mut Writing<'_>) -> Result<i64, Trap> {
    let mut heap = Heap::new(module);
    walk_with(module, entry, writing, &mut heap)
}

fn walk_with(
    module: &qir::Module,
    entry: qir::FuncId,
    writing: &mut Writing<'_>,
    heap: &mut Heap,
) -> Result<i64, Trap> {
    let mut stack = vec![Frame::new(module, entry, &[])];
    // Allocated and never freed, which is the first stage of the collector and is all
    // an array needs in order to exist. A handle is an index into this.
    // The module's constant tables, laid out before anything runs — so table `i` is
    // handle `i`, and nothing has to look one up. The same thing the text table is,
    // with numbers in it.



    loop {
        let top = stack.len() - 1;
        let func = module.func(stack[top].func);
        let block = func.block(stack[top].block);

        // Instructions, until the block runs out or a call interrupts it.
        let mut calling = None;
        let mut at = stack[top].at;
        while at < block.insts.len() {
            let (result, inst) = &block.insts[at];
            at += 1;
            if let qir::Inst::Call { func: callee, args } = inst {
                let given: Vec<i64> =
                    args.iter().map(|a| stack[top].slots[a.0 as usize]).collect();
                stack[top].pending = Some(*result);
                calling = Some((*callee, given));
                break;
            }
            let value = evaluate(inst, &stack[top].slots, module, module.func(stack[top].func), writing, heap)?;
            stack[top].slots[result.0 as usize] = value;
        }
        stack[top].at = at;

        // Between instructions, and only there: every handle a program can still reach
        // is in a slot of some frame, and nothing is half-built. What makes the roots
        // exact here is the thing that makes this engine the reference one — its call
        // stack is a list it owns, not the machine's.
        if heap.worth_collecting() {
            let roots = rooted(&stack, module);
            heap.collect(&roots);
        }

        if let Some((callee, given)) = calling {
            if stack.len() >= DEPTH {
                return Err(Trap::TooDeep);
            }
            stack.push(Frame::new(module, callee, &given));
            continue;
        }

        match &block.term {
            qir::Term::Ret(v) => {
                let value = stack[top].slots[v.0 as usize];
                stack.pop();
                match stack.last_mut() {
                    // The entry returned, so the program is over.
                    None => return Ok(value),
                    Some(caller) => {
                        let slot = caller.pending.take().expect("a call is what put us here");
                        caller.slots[slot.0 as usize] = value;
                    }
                }
            }
            qir::Term::Jump { to, args } => {
                let given: Vec<i64> =
                    args.iter().map(|a| stack[top].slots[a.0 as usize]).collect();
                stack[top].enter(func, *to, &given);
            }
            qir::Term::BrIf { cond, then, otherwise } => {
                let taken =
                    if stack[top].slots[cond.0 as usize] != 0 { then } else { otherwise };
                let given: Vec<i64> =
                    taken.args.iter().map(|a| stack[top].slots[a.0 as usize]).collect();
                let block = taken.block;
                stack[top].enter(func, block, &given);
            }
        }
    }
}

/// Everything that is not a call, which is everything that cannot change the stack.
fn evaluate(
    inst: &qir::Inst,
    slots: &[i64],
    _module: &qir::Module,
    func: &qir::Function,
    writing: &mut Writing<'_>,
    heap: &mut Heap,
) -> Result<i64, Trap> {
    Ok(match inst {
        qir::Inst::ConstI64(n) => *n,
        qir::Inst::ConstBool(t) => i64::from(*t),
        // A text value is the index of the text, not a pointer to it.
        qir::Inst::ConstText(at) => i64::from(*at),
        qir::Inst::ConstHandle(at) => i64::from(*at),
        // Carried as bits, which is what a slot holds anyway.
        qir::Inst::ConstFloat(bits) => *bits as i64,
        qir::Inst::CallHost { host, args } => {
            match host {
                qir::Host::ArrayNew => {
                    let len = slots[args[0].0 as usize];
                    let holds = qir::Elements::from_code(slots[args[1].0 as usize])
                        .expect("the lowering wrote this constant");
                    let depth = slots[args[2].0 as usize];
                    return Ok(heap.make(holds, depth, vec![0; len.max(0) as usize]));
                }
                qir::Host::ArraySet => {
                    let (h, at, value) = (
                        slots[args[0].0 as usize] as usize,
                        slots[args[1].0 as usize],
                        slots[args[2].0 as usize],
                    );
                    let array = &mut heap.at_mut(h as i64).values;
                    // Counted from one, so `0` is no element and so is one past the end.
                    let Some(slot) = at.checked_sub(1).and_then(|i| usize::try_from(i).ok())
                    else {
                        return Err(Trap::OutsideTheArray);
                    };
                    let Some(cell) = array.get_mut(slot) else {
                        return Err(Trap::OutsideTheArray);
                    };
                    *cell = value;
                }
                qir::Host::ArrayGet => {
                    let (h, at) =
                        (slots[args[0].0 as usize] as usize, slots[args[1].0 as usize]);
                    let Some(slot) = at.checked_sub(1).and_then(|i| usize::try_from(i).ok())
                    else {
                        return Err(Trap::OutsideTheArray);
                    };
                    let Some(value) = heap.at(h as i64).values.get(slot) else {
                        return Err(Trap::OutsideTheArray);
                    };
                    return Ok(*value);
                }
                qir::Host::ArrayLen => {
                    return Ok(heap.at(slots[args[0].0 as usize]).values.len() as i64);
                }
                qir::Host::ArrayPush => {
                    let (h, value) =
                        (slots[args[0].0 as usize] as usize, slots[args[1].0 as usize]);
                    heap.at_mut(h as i64).values.push(value);
                }
                qir::Host::ArrayCopy => {
                    let from = slots[args[0].0 as usize];
                    let (holds, depth, values) = {
                        let of = heap.at(from);
                        (of.holds, of.depth, of.values.clone())
                    };
                    return Ok(heap.make(holds, depth, values));
                }
                qir::Host::ArrayEqual => {
                    let kind = qir::Elements::from_code(slots[args[2].0 as usize])
                        .expect("the lowering wrote this constant");
                    let depth = slots[args[3].0 as usize];
                    let (a, b) =
                        (slots[args[0].0 as usize], slots[args[1].0 as usize]);
                    return Ok(i64::from(alike(a, b, kind, depth, heap)));
                }
                qir::Host::PrintArray => {
                    let kind = qir::Elements::from_code(slots[args[2].0 as usize])
                        .expect("the lowering wrote this constant");
                    let depth = slots[args[3].0 as usize];
                    let shown = shown(slots[args[1].0 as usize], kind, depth, heap);
                    let _ = write!(writing.to(slots[args[0].0 as usize]), "{shown}");
                }
                qir::Host::ExactRead => {
                    let at = slots[args[0].0 as usize] as usize;
                    let read = quench_num::Exact::parse(heap.said(at as i64))
                        .expect("refused by the checker: an `e` that is not a number");
                    return Ok(heap.exact(read));
                }
                qir::Host::ExactAdd
                | qir::Host::ExactSub
                | qir::Host::ExactMul
                | qir::Host::ExactDiv => {
                    let (a, b) = (
                        heap.exactly(slots[args[0].0 as usize]).clone(),
                        heap.exactly(slots[args[1].0 as usize]).clone(),
                    );
                    let answer = match host {
                        qir::Host::ExactAdd => a.add(&b),
                        qir::Host::ExactSub => a.sub(&b),
                        qir::Host::ExactMul => a.mul(&b),
                        // The one exact division that has no answer. Nothing else does:
                        // a rational divided by a rational is a rational, always.
                        _ => a.div(&b).map_err(|_| Trap::DividedByZero)?,
                    };
                    return Ok(heap.exact(answer));
                }
                qir::Host::ExactPow => {
                    let (a, b) = (
                        heap.exactly(slots[args[0].0 as usize]).clone(),
                        heap.exactly(slots[args[1].0 as usize]).clone(),
                    );
                    let answer = a.power(&b).map_err(no_power)?;
                    return Ok(heap.exact(answer));
                }
                qir::Host::DecimalRead => {
                    let at = slots[args[0].0 as usize];
                    let format = decimal_format(slots[args[1].0 as usize]);
                    let read = quench_num::Decimal::parse(heap.said(at), format)
                        .expect("refused by the checker: a decimal that is not a number");
                    return Ok(heap.decimal(read));
                }
                qir::Host::DecimalAdd
                | qir::Host::DecimalSub
                | qir::Host::DecimalMul
                | qir::Host::DecimalDiv => {
                    let (a, b) = (
                        heap.decimally(slots[args[0].0 as usize]).clone(),
                        heap.decimally(slots[args[1].0 as usize]).clone(),
                    );
                    let format = decimal_format(slots[args[2].0 as usize]);
                    let answer = match host {
                        qir::Host::DecimalAdd => a.add(&b, format),
                        qir::Host::DecimalSub => a.sub(&b, format),
                        qir::Host::DecimalMul => a.mul(&b, format),
                        // No trap here, unlike an `e`: dividing by nought is infinity,
                        // which is an answer a float has and a ratio does not.
                        _ => a.div(&b, format),
                    };
                    return Ok(heap.decimal(answer));
                }
                qir::Host::DecimalCompare => {
                    let (a, b) = (
                        heap.decimally(slots[args[0].0 as usize]),
                        heap.decimally(slots[args[1].0 as usize]),
                    );
                    return Ok(match a.compare(b) {
                        Some(std::cmp::Ordering::Less) => -1,
                        Some(std::cmp::Ordering::Equal) => 0,
                        Some(std::cmp::Ordering::Greater) => 1,
                        // Not-a-number, which is none of the three.
                        None => 2,
                    });
                }
                qir::Host::PrintDecimal => {
                    let value = heap.decimally(slots[args[1].0 as usize]);
                    let _ = write!(writing.to(slots[args[0].0 as usize]), "{value}");
                }
                // The `Say` family: exactly what the matching `Print` would have
                // written, handed back instead. Each is the same expression as the one
                // above it, so the two cannot drift apart without the drift being
                // visible on one screen.
                qir::Host::SayI64 => {
                    let value = slots[args[0].0 as usize];
                    return Ok(heap.text(value.to_string()));
                }
                qir::Host::SayU64 => {
                    let value = slots[args[0].0 as usize] as u64;
                    return Ok(heap.text(value.to_string()));
                }
                qir::Host::SayBool => {
                    let yes = slots[args[0].0 as usize] != 0;
                    return Ok(heap.text(if yes { "true" } else { "false" }.to_string()));
                }
                qir::Host::SayFloat => {
                    let bits = slots[args[0].0 as usize];
                    let shown = match slots[args[1].0 as usize] {
                        64 => quench_num::show_f64(f64::from_bits(bits as u64)),
                        _ => quench_num::show_f32(f32::from_bits(bits as u32)),
                    };
                    return Ok(heap.text(shown));
                }
                qir::Host::SayExact => {
                    let shown = heap.exactly(slots[args[0].0 as usize]).to_string();
                    return Ok(heap.text(shown));
                }
                qir::Host::SayDecimal => {
                    let shown = heap.decimally(slots[args[0].0 as usize]).to_string();
                    return Ok(heap.text(shown));
                }
                qir::Host::SayArray => {
                    let kind = qir::Elements::from_code(slots[args[1].0 as usize])
                        .expect("the lowering wrote this constant");
                    let depth = slots[args[2].0 as usize];
                    let shown = shown(slots[args[0].0 as usize], kind, depth, heap);
                    return Ok(heap.text(shown));
                }
                qir::Host::FloatAlone => {
                    let which = ALONE[slots[args[1].0 as usize] as usize];
                    let bits = slots[args[0].0 as usize];
                    return Ok(match slots[args[2].0 as usize] {
                        64 => quench_num::maths::alone64(which, f64::from_bits(bits as u64))
                            .to_bits() as i64,
                        _ => i64::from(
                            quench_num::maths::alone32(which, f32::from_bits(bits as u32))
                                .to_bits(),
                        ),
                    });
                }
                qir::Host::FloatPaired => {
                    let which = PAIRED[slots[args[2].0 as usize] as usize];
                    let (a, b) = (slots[args[0].0 as usize], slots[args[1].0 as usize]);
                    return Ok(match slots[args[3].0 as usize] {
                        64 => quench_num::maths::paired64(
                            which,
                            f64::from_bits(a as u64),
                            f64::from_bits(b as u64),
                        )
                        .to_bits() as i64,
                        _ => i64::from(
                            quench_num::maths::paired32(
                                which,
                                f32::from_bits(a as u32),
                                f32::from_bits(b as u32),
                            )
                            .to_bits(),
                        ),
                    });
                }
                qir::Host::FloatFused => {
                    let (a, b, c) = (
                        slots[args[0].0 as usize],
                        slots[args[1].0 as usize],
                        slots[args[2].0 as usize],
                    );
                    return Ok(match slots[args[3].0 as usize] {
                        64 => quench_num::maths::fused64(
                            f64::from_bits(a as u64),
                            f64::from_bits(b as u64),
                            f64::from_bits(c as u64),
                        )
                        .to_bits() as i64,
                        _ => i64::from(
                            quench_num::maths::fused32(
                                f32::from_bits(a as u32),
                                f32::from_bits(b as u32),
                                f32::from_bits(c as u32),
                            )
                            .to_bits(),
                        ),
                    });
                }
                qir::Host::TextClusters => {
                    let said = heap.said(slots[args[0].0 as usize]);
                    return Ok(quench_text::grapheme::count(said) as i64);
                }
                qir::Host::TextLetters => {
                    let said = heap.said(slots[args[0].0 as usize]);
                    return Ok(said.chars().count() as i64);
                }
                qir::Host::PowI64 | qir::Host::PowI64Trapping => {
                    let (base, exponent) =
                        (slots[args[0].0 as usize], slots[args[1].0 as usize]);
                    let wrapping = *host == qir::Host::PowI64;
                    return quench_num::power_i64(base, exponent, wrapping).map_err(no_power);
                }
                qir::Host::TextJoin => {
                    let joined = format!(
                        "{}{}",
                        heap.said(slots[args[0].0 as usize]),
                        heap.said(slots[args[1].0 as usize])
                    );
                    return Ok(heap.text(joined));
                }
                qir::Host::TextCompare => {
                    let (a, b) = (
                        heap.said(slots[args[0].0 as usize]),
                        heap.said(slots[args[1].0 as usize]),
                    );
                    return Ok(match a.cmp(b) {
                        std::cmp::Ordering::Less => -1,
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 1,
                    });
                }
                qir::Host::ExactCompare => {
                    let (a, b) = (
                        heap.exactly(slots[args[0].0 as usize]),
                        heap.exactly(slots[args[1].0 as usize]),
                    );
                    return Ok(match a.cmp(b) {
                        std::cmp::Ordering::Less => -1,
                        std::cmp::Ordering::Equal => 0,
                        std::cmp::Ordering::Greater => 1,
                    });
                }
                qir::Host::PrintFloat => {
                    let bits = slots[args[1].0 as usize];
                    let shown = match slots[args[2].0 as usize] {
                        64 => quench_num::show_f64(f64::from_bits(bits as u64)),
                        _ => quench_num::show_f32(f32::from_bits(bits as u32)),
                    };
                    let _ = write!(writing.to(slots[args[0].0 as usize]), "{shown}");
                }
                qir::Host::ToB16 => {
                    let x = f32::from_bits(slots[args[0].0 as usize] as u32);
                    return Ok(i64::from(quench_num::to_b16(x).to_bits()));
                }
                qir::Host::PrintExact => {
                    let value = heap.exactly(slots[args[1].0 as usize]);
                    let _ = write!(writing.to(slots[args[0].0 as usize]), "{value}");
                }
                qir::Host::PrintBool => {
                    let to = writing.to(slots[args[0].0 as usize]);
                    let yes = slots[args[1].0 as usize] != 0;
                    let _ = write!(to, "{}", if yes { "true" } else { "false" });
                }
                qir::Host::PrintU64 => {
                    let value = slots[args[1].0 as usize] as u64;
                    let _ = write!(writing.to(slots[args[0].0 as usize]), "{value}");
                }
                qir::Host::PrintI64 => {
                    let value = slots[args[1].0 as usize];
                    let _ = write!(writing.to(slots[args[0].0 as usize]), "{value}");
                }
                qir::Host::PrintText => {
                    let at = slots[args[1].0 as usize] as usize;
                    let text = heap.said(at as i64).to_string();
                    let out = writing.to(slots[args[0].0 as usize]);
                    // Nowhere to report a write that fails, and nothing sensible to do
                    // about one: a program that cannot print has not computed the wrong
                    // answer. The oracle compares what was written, so a lost write
                    // shows up there as a disagreement rather than being silent.
                    let _ = out.write_all(text.as_bytes());
                }
            }
            0
        }
        qir::Inst::Bin { op, lhs, rhs } => {
            let (l, r) = (slots[lhs.0 as usize], slots[rhs.0 as usize]);
            match op {
                // Wrapping, because Cranelift's `iadd` and friends wrap. What a Quench
                // program sees is a setting applied further up.
                // Both sides were worked out before this ran, which is the whole of
                // what `asks-both` means.
                // IEEE, plainly: nothing fused, nothing relaxed, and the bits back.
                qir::BinOp::FAdd
                | qir::BinOp::FSub
                | qir::BinOp::FMul
                | qir::BinOp::FDiv
                | qir::BinOp::FAddChecked
                | qir::BinOp::FSubChecked
                | qir::BinOp::FMulChecked
                | qir::BinOp::FDivChecked => {
                    // Which width it is comes from QIR, which says what every value in
                    // a function is. A `b32` and a `b64` are otherwise the same bits in
                    // the same slot, and reading one as the other is nonsense rather
                    // than an approximation.
                    if func.ty_of(*lhs) == qir::Ty::F64 {
                        let (a, b) = (f64::from_bits(l as u64), f64::from_bits(r as u64));
                        let answer = match op {
                            qir::BinOp::FAdd | qir::BinOp::FAddChecked => a + b,
                            qir::BinOp::FSub | qir::BinOp::FSubChecked => a - b,
                            qir::BinOp::FMul | qir::BinOp::FMulChecked => a * b,
                            _ => a / b,
                        };
                        if op.checks_the_answer() && !answer.is_finite() {
                            return Err(Trap::NoNumber);
                        }
                        answer.to_bits() as i64
                    } else {
                        let (a, b) = (f32::from_bits(l as u32), f32::from_bits(r as u32));
                        let answer = match op {
                            qir::BinOp::FAdd | qir::BinOp::FAddChecked => a + b,
                            qir::BinOp::FSub | qir::BinOp::FSubChecked => a - b,
                            qir::BinOp::FMul | qir::BinOp::FMulChecked => a * b,
                            _ => a / b,
                        };
                        if op.checks_the_answer() && !answer.is_finite() {
                            return Err(Trap::NoNumber);
                        }
                        i64::from(answer.to_bits())
                    }
                }
                qir::BinOp::DivU => {
                    let (a, b) = (l as u64, r as u64);
                    if b == 0 {
                        return Err(Trap::DividedByZero);
                    }
                    (a / b) as i64
                }
                qir::BinOp::RemU => {
                    let (a, b) = (l as u64, r as u64);
                    if b == 0 {
                        return Err(Trap::DividedByZero);
                    }
                    (a % b) as i64
                }
                qir::BinOp::AddTrappingU => {
                    (l as u64).checked_add(r as u64).ok_or(Trap::Overflowed)? as i64
                }
                qir::BinOp::SubTrappingU => {
                    (l as u64).checked_sub(r as u64).ok_or(Trap::Overflowed)? as i64
                }
                qir::BinOp::MulTrappingU => {
                    (l as u64).checked_mul(r as u64).ok_or(Trap::Overflowed)? as i64
                }
                qir::BinOp::And => i64::from(l != 0 && r != 0),
                qir::BinOp::Or => i64::from(l != 0 || r != 0),
                qir::BinOp::Add => l.wrapping_add(r),
                qir::BinOp::Sub => l.wrapping_sub(r),
                qir::BinOp::Mul => l.wrapping_mul(r),
                qir::BinOp::AddTrapping => l.checked_add(r).ok_or(Trap::Overflowed)?,
                qir::BinOp::SubTrapping => l.checked_sub(r).ok_or(Trap::Overflowed)?,
                qir::BinOp::MulTrapping => l.checked_mul(r).ok_or(Trap::Overflowed)?,
                qir::BinOp::DivTruncated => l.checked_div(r).ok_or(trap_for(r))?,
                qir::BinOp::RemTruncated => l.checked_rem(r).ok_or(trap_for(r))?,
                qir::BinOp::DivFloored => {
                    let quotient = l.checked_div(r).ok_or(trap_for(r))?;
                    let rest = l.wrapping_rem(r);
                    // Truncation rounded toward zero. When the remainder does not agree
                    // in sign with the divisor, that was one step the wrong way.
                    if rest != 0 && (rest < 0) != (r < 0) {
                        quotient.wrapping_sub(1)
                    } else {
                        quotient
                    }
                }
                qir::BinOp::RemFloored => {
                    let rest = l.checked_rem(r).ok_or(trap_for(r))?;
                    // `|rest| < |r|` and their signs differ, so this cannot overflow.
                    if rest != 0 && (rest < 0) != (r < 0) { rest + r } else { rest }
                }
            }
        }
        qir::Inst::Cmp { op, lhs, rhs } => {
            let (l, r) = (slots[lhs.0 as usize], slots[rhs.0 as usize]);
            i64::from(match op {
                qir::CmpOp::Eq => l == r,
                qir::CmpOp::Ne => l != r,
                qir::CmpOp::Lt => l < r,
                qir::CmpOp::Le => l <= r,
                qir::CmpOp::Gt => l > r,
                qir::CmpOp::Ge => l >= r,
            })
        }
        // Rust's own `f64` comparisons are IEEE's, including a not-a-number being
        // false against everything and itself.
        qir::Inst::FCmp { op, lhs, rhs } => {
            let wide = func.ty_of(*lhs) == qir::Ty::F64;
            let (l, r) = if wide {
                (
                    f64::from_bits(slots[lhs.0 as usize] as u64),
                    f64::from_bits(slots[rhs.0 as usize] as u64),
                )
            } else {
                (
                    f64::from(f32::from_bits(slots[lhs.0 as usize] as u32)),
                    f64::from(f32::from_bits(slots[rhs.0 as usize] as u32)),
                )
            };
            i64::from(match op {
                qir::CmpOp::Eq => l == r,
                qir::CmpOp::Ne => l != r,
                qir::CmpOp::Lt => l < r,
                qir::CmpOp::Le => l <= r,
                qir::CmpOp::Gt => l > r,
                qir::CmpOp::Ge => l >= r,
            })
        }
        qir::Inst::Not(v) => slots[v.0 as usize] ^ 1,
        qir::Inst::Narrow { of, bits, signed, checked } => {
            let value = slots[of.0 as usize];
            let put = narrowed(value, *bits, *signed);
            if *checked && put != value {
                return Err(Trap::Overflowed);
            }
            put
        }
        qir::Inst::CmpU { op, lhs, rhs } => {
            let (l, r) = (slots[lhs.0 as usize] as u64, slots[rhs.0 as usize] as u64);
            i64::from(match op {
                qir::CmpOp::Eq => l == r,
                qir::CmpOp::Ne => l != r,
                qir::CmpOp::Lt => l < r,
                qir::CmpOp::Le => l <= r,
                qir::CmpOp::Gt => l > r,
                qir::CmpOp::Ge => l >= r,
            })
        }
        qir::Inst::Call { .. } => unreachable!("calls are handled on the stack, not here"),
    })
}

/// Dividing by zero and overflowing are different stops, and an engine has to agree
/// about which one happened, not merely that something did.
fn trap_for(divisor: i64) -> Trap {
    if divisor == 0 { Trap::DividedByZero } else { Trap::DivisionOverflowed }
}
