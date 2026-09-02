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
//! is roughly 352× the execution. An interpreter skips the 103µs entirely. Even at a
//! hundred times native speed it finishes in a third of the time the Dev JIT spends
//! before it has run anything at all. See `crates/quench-dev/examples/cost.rs`.
//!
//! # Agreeing about stopping, not just about answers
//!
//! A program that divides by zero has to stop in the same place for the same reason in
//! every engine, which is as much an agreement as printing the same number. So the traps
//! here are not the interpreter's own opinion — they are chosen to match what Cranelift's
//! `sdiv` and `srem` do, because that is what the machine code will do.

use quench_qir as qir;

/// How a run ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// It finished, and this is what `START` gave back.
    Returned(i64),
    /// It stopped. Every engine must stop here too, and for this reason.
    Trapped(Trap),
}

/// A reason a program stops.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trap {
    /// Division or remainder by zero.
    DividedByZero,
    /// `i64::MIN / -1`, whose answer is one larger than an `i64` holds. Cranelift traps
    /// on this rather than wrapping, so this does too.
    DivisionOverflowed,
    /// Calls nested deeper than the interpreter will follow.
    ///
    /// An admitted mismatch: compiled code has a real machine stack and overflows it
    /// rather than reporting anything, so two engines cannot be compared on a program
    /// that gets here. The interpreter reports it instead of crashing, which is the
    /// difference between an oracle that skips a seed and one that loses a run.
    TooDeep,
}

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

/// Run a module from its entry.
pub fn run(module: &qir::Module) -> Result<Outcome, Error> {
    let id = module.entry.ok_or(Error::NoEntry)?;
    run_id(module, id)
}

/// Run one named function that takes nothing and returns an i64.
///
/// The oracle's other door: a module holds many generated programs, and this runs one
/// of them without the module having to name it as the entry.
pub fn run_named(module: &qir::Module, name: &str) -> Result<Outcome, Error> {
    let id = module
        .find(name)
        .ok_or_else(|| Error::Entry(format!("there is no function called `{name}`")))?;
    run_id(module, id)
}

fn run_id(module: &qir::Module, id: qir::FuncId) -> Result<Outcome, Error> {
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

    Ok(match walk(module, id) {
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
fn walk(module: &qir::Module, entry: qir::FuncId) -> Result<i64, Trap> {
    let mut stack = vec![Frame::new(module, entry, &[])];

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
            let value = evaluate(inst, &stack[top].slots)?;
            stack[top].slots[result.0 as usize] = value;
        }
        stack[top].at = at;

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
fn evaluate(inst: &qir::Inst, slots: &[i64]) -> Result<i64, Trap> {
    Ok(match inst {
        qir::Inst::ConstI64(n) => *n,
        qir::Inst::ConstBool(t) => i64::from(*t),
        qir::Inst::Bin { op, lhs, rhs } => {
            let (l, r) = (slots[lhs.0 as usize], slots[rhs.0 as usize]);
            match op {
                // Wrapping, because Cranelift's `iadd` and friends wrap. What a Quench
                // program sees is a setting applied further up.
                qir::BinOp::Add => l.wrapping_add(r),
                qir::BinOp::Sub => l.wrapping_sub(r),
                qir::BinOp::Mul => l.wrapping_mul(r),
                qir::BinOp::Div => l.checked_div(r).ok_or(trap_for(r))?,
                qir::BinOp::Rem => l.checked_rem(r).ok_or(trap_for(r))?,
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
        qir::Inst::Not(v) => slots[v.0 as usize] ^ 1,
        qir::Inst::Call { .. } => unreachable!("calls are handled on the stack, not here"),
    })
}

/// Dividing by zero and overflowing are different stops, and an engine has to agree
/// about which one happened, not merely that something did.
fn trap_for(divisor: i64) -> Trap {
    if divisor == 0 { Trap::DividedByZero } else { Trap::DivisionOverflowed }
}
