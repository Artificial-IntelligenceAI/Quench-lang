//! The Dev JIT: QIR to machine code, by Cranelift, in this process.
//!
//! This is the method meant for editing. It compiles roughly an order of magnitude faster
//! than an LLVM pipeline and optimises far less, and both halves of that are the point:
//!
//! - **Fast** is what makes it usable between keystrokes.
//! - **Barely optimised** is what makes it the *reference*. The other two methods are
//!   allowed to be cleverer, and being cleverer is where miscompiles come from. When the
//!   oracle finds a disagreement, something has to be believed, and it should be the
//!   engine that did the least to the code.
//!
//! So the optimisation level here is deliberately `none`. It is not a placeholder waiting
//! to be turned up.

use cranelift_codegen::ir::condcodes::IntCC;
use cranelift_codegen::ir::{types, AbiParam, BlockArg, FuncRef, InstBuilder, MemFlagsData};
use cranelift_codegen::ir::{Function as ClifFunction, Value as ClifValue};
use cranelift_codegen::isa::TargetFrontendConfig;
use cranelift_codegen::settings::{self, Configurable as _};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{default_libcall_names, Linkage, Module as _};
use quench_conf::Optimise;
use quench_qir as qir;

/// Why a module could not be compiled.
///
/// None of these are things a Quench program can cause. A program that does not make
/// sense is stopped by the frontend, with a diagnostic; anything here is a bug in the
/// compiler.
///
/// [`Error::Invalid`] carries its findings rather than a rendered message, so a caller
/// can say the true thing about them: [`qir::diagnose`] with [`qir::Audience::Ourselves`]
/// when this compiler built the module, and [`qir::Audience::AFileWeWereGiven`] when it
/// was read from somewhere. See [`qir::verify`].
#[derive(Debug)]
pub enum Error {
    /// The IR did not check out. See [`qir::verify`].
    Invalid(Vec<qir::Invalid>),
    /// Cranelift refused something, or the host is not a target we can compile for.
    Backend(String),
    /// The module named no entry, so there is nothing to run.
    NoEntry,
    /// The entry exists but cannot be called as one.
    Entry(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Invalid(wrong) => {
                writeln!(f, "the IR handed to the Dev JIT is not well formed:")?;
                for one in wrong {
                    writeln!(f, "  - {one}")?;
                }
                Ok(())
            }
            Error::Backend(why) => write!(f, "Cranelift could not compile this: {why}"),
            Error::NoEntry => write!(f, "the module names no entry, so there is nothing to run"),
            Error::Entry(why) => write!(f, "the entry cannot be called: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// What a program wrote, kept apart by where it said to write it.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Printed {
    pub out: String,
    pub err: String,
}

/// A module that has been compiled and is ready to run.
///
/// The machine code lives as long as this does, which is why [`Compiled::run`] can hand
/// back a value from it safely.
pub struct Compiled {
    /// Held so the code it owns outlives every pointer taken from it.
    _jit: JITModule,
    /// The module's text, owned here so the pointers below stay good. A `String`'s
    /// buffer does not move when the `Vec` holding the `String` grows, which is what
    /// makes this safe.
    _text: Vec<String>,
    /// What the generated code indexes into. Boxed so its address is fixed, because that
    /// address was compiled into the code.
    _pieces: Box<[Piece]>,
    /// The module's constant tables. Laid into the heap at the start of every run, so
    /// that table `i` is handle `i` and the compiled code needs no lookup for one.
    _tables: Vec<Vec<i64>>,
    /// The block whose address the code carries. Boxed for the same reason, and read
    /// afterwards to find out whether the program stopped.
    runtime: Box<Runtime>,
    entry: *const u8,
    /// Every function that takes nothing and returns an i64, by name.
    ///
    /// The oracle needs this: compiling costs a few hundred times what running costs, so
    /// it puts many generated programs in one module and calls them one at a time rather
    /// than compiling each on its own.
    callable: Vec<(String, *const u8)>,
}

impl Compiled {
    /// Run one named function that takes nothing and returns an i64.
    ///
    /// `None` if there is no such function, or if it takes arguments.
    pub fn call(&self, name: &str) -> Option<i64> {
        let (_, code) = self.callable.iter().find(|(known, _)| known == name)?;
        HEAP.with(|heap| *heap.borrow_mut() = self._tables.clone());
        let rt = &*self.runtime as *const Runtime as *mut Runtime;
        unsafe { (*rt).stopped = 0 };
        // Safe for the same reasons `run` is: the signature was checked at compile time
        // and the code is kept alive by `_jit` for as long as `self` exists.
        let f: extern "C" fn() -> i64 = unsafe { std::mem::transmute(*code) };
        Some(f())
    }

    /// Run the entry, keeping whatever it printed rather than letting it out.
    pub fn run_capturing(&self) -> (qir::Outcome, Printed) {
        SINK.with(|sink| *sink.borrow_mut() = Some((Vec::new(), Vec::new())));
        let outcome = self.outcome();
        let (out, err) = SINK.with(|sink| sink.borrow_mut().take()).unwrap_or_default();
        (
            outcome,
            Printed {
                out: String::from_utf8_lossy(&out).into_owned(),
                err: String::from_utf8_lossy(&err).into_owned(),
            },
        )
    }

    /// Run the entry, and say whether it finished or stopped.
    pub fn outcome(&self) -> qir::Outcome {
        let answer = self.run();
        match qir::Trap::from_code(self.runtime.stopped) {
            Some(trap) => qir::Outcome::Trapped(trap),
            None => qir::Outcome::Returned(answer),
        }
    }

    /// Run the entry and hand back what it returned.
    ///
    /// A program that stopped returns whatever was on hand at the time, which means
    /// nothing — [`Compiled::outcome`] is the one that says so.
    pub fn run(&self) -> i64 {
        HEAP.with(|heap| *heap.borrow_mut() = self._tables.clone());
        // The flag is per-run, so a program that stopped does not make the next one look
        // as though it did.
        let rt = &*self.runtime as *const Runtime as *mut Runtime;
        unsafe { (*rt).stopped = 0 };
        // Safe because `compile` checked the entry takes nothing and returns i64, and the
        // code is kept alive by `_jit` for as long as `self` exists.
        let entry: extern "C" fn() -> i64 = unsafe { std::mem::transmute(self.entry) };
        entry()
    }

    /// Run one named function that takes nothing and returns an i64, saying whether it
    /// finished or stopped.
    pub fn call_outcome(&self, name: &str) -> Option<qir::Outcome> {
        let answer = self.call(name)?;
        Some(match qir::Trap::from_code(self.runtime.stopped) {
            Some(trap) => qir::Outcome::Trapped(trap),
            None => qir::Outcome::Returned(answer),
        })
    }
}

impl std::fmt::Debug for Compiled {
    /// Enough to tell two apart in a failing assertion. The machine code behind the
    /// pointer is not something a test wants printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Compiled {{ entry: {:p} }}", self.entry)
    }
}

/// Write down a reason and leave.
///
/// The generated code stores the code itself rather than calling anything, so a division
/// by zero costs a compare and a branch rather than a call into Rust.
fn stop_now(
    b: &mut FunctionBuilder<'_>,
    runtime: i64,
    stopping: cranelift_codegen::ir::Block,
    trap: qir::Trap,
) {
    let at = b.ins().iconst(types::I64, runtime);
    let code = b.ins().iconst(types::I64, trap as i64);
    b.ins().store(MemFlagsData::trusted(), code, at, STOPPED_AT);
    b.ins().jump(stopping, &[]);
}

/// Arithmetic that stops rather than rounding.
///
/// Cranelift gives the overflow bit alongside the answer, so this costs one branch and
/// no arithmetic of its own — cheaper than the division guards, which have to work out
/// beforehand whether the processor would object.
fn trapping(
    b: &mut FunctionBuilder<'_>,
    runtime: i64,
    stopping: cranelift_codegen::ir::Block,
    op: qir::BinOp,
    lhs: ClifValue,
    rhs: ClifValue,
) -> ClifValue {
    let (answer, overflowed) = match op {
        qir::BinOp::AddTrapping => {
            let r = b.ins().sadd_overflow(lhs, rhs);
            (r.0, r.1)
        }
        qir::BinOp::SubTrapping => {
            let r = b.ins().ssub_overflow(lhs, rhs);
            (r.0, r.1)
        }
        qir::BinOp::MulTrapping => {
            let r = b.ins().smul_overflow(lhs, rhs);
            (r.0, r.1)
        }
        _ => unreachable!("only the trapping three reach here"),
    };
    let fits = b.create_block();
    let does_not = b.create_block();
    b.ins().brif(overflowed, does_not, &[], fits, &[]);
    b.switch_to_block(does_not);
    stop_now(b, runtime, stopping, qir::Trap::Overflowed);
    b.switch_to_block(fits);
    answer
}

/// Emit a division that cannot fault, by refusing the two cases that would.
///
/// Cranelift's `sdiv` traps on a zero divisor and on `i64::MIN / -1`, and a trap in
/// compiled code is a hardware fault — which aborts the process and loses the whole run
/// rather than one program. So the two cases are tested for first, and the division that
/// follows is one the processor cannot object to.
///
/// It costs two compares and two branches per division. That is the price of a stop
/// being *reportable*, and it is worth it: an engine that cannot say why it stopped
/// cannot be compared with one that can.
fn guarded_division(
    b: &mut FunctionBuilder<'_>,
    runtime: i64,
    stopping: cranelift_codegen::ir::Block,
    lhs: ClifValue,
    rhs: ClifValue,
) {
    // A zero divisor.
    let by_zero = b.create_block();
    let not_zero = b.create_block();
    let is_zero = b.ins().icmp_imm_s(IntCC::Equal, rhs, 0);
    b.ins().brif(is_zero, by_zero, &[], not_zero, &[]);
    b.switch_to_block(by_zero);
    stop_now(b, runtime, stopping, qir::Trap::DividedByZero);
    b.switch_to_block(not_zero);

    // The one division whose answer does not fit: the smallest number over minus one.
    let overflows = b.create_block();
    let fits = b.create_block();
    let smallest = b.ins().iconst(types::I64, i64::MIN);
    let lhs_smallest = b.ins().icmp(IntCC::Equal, lhs, smallest);
    let rhs_minus_one = b.ins().icmp_imm_s(IntCC::Equal, rhs, -1);
    let both = b.ins().band(lhs_smallest, rhs_minus_one);
    b.ins().brif(both, overflows, &[], fits, &[]);
    b.switch_to_block(overflows);
    stop_now(b, runtime, stopping, qir::Trap::DivisionOverflowed);
    b.switch_to_block(fits);
}

/// Look at the runtime's stop flag, and leave if it is set.
///
/// Splits the block: everything after this carries on in a fresh one, which is what lets
/// the check sit in the middle of a run of instructions.
fn stop_if_stopped(b: &mut FunctionBuilder<'_>, runtime: i64, stopping: cranelift_codegen::ir::Block) {
    let at = b.ins().iconst(types::I64, runtime);
    let flag = b.ins().load(types::I64, MemFlagsData::trusted(), at, STOPPED_AT);
    let carry_on = b.create_block();
    b.ins().brif(flag, stopping, &[], carry_on, &[]);
    b.switch_to_block(carry_on);
}

/// Whether a remainder is non-zero and leans the other way from its divisor — which is
/// precisely when truncating rounded one step further from negative infinity than
/// flooring would have.
fn signs_disagree(
    b: &mut FunctionBuilder<'_>,
    rest: ClifValue,
    divisor: ClifValue,
) -> ClifValue {
    let not_zero = b.ins().icmp_imm_s(IntCC::NotEqual, rest, 0);
    let mixed = b.ins().bxor(rest, divisor);
    let differ = b.ins().icmp_imm_s(IntCC::SignedLessThan, mixed, 0);
    b.ins().band(not_zero, differ)
}

fn clif_ty(ty: qir::Ty) -> types::Type {
    match ty {
        qir::Ty::I64 => types::I64,
        // Cranelift has no one-bit type: a comparison yields an i8 that is 0 or 1.
        qir::Ty::Bool => types::I8,
        // An index into the module's text, which is an integer like any other. Turning
        // it into an address is this backend's business and happens in the runtime
        // function rather than in the generated code.
        qir::Ty::Text => types::I64,
        // A handle into the heap, which is an index and so an integer as well. The
        // generated code never dereferences one; it hands it back to the runtime.
        qir::Ty::Handle | qir::Ty::Exact => types::I64,
    }
}

/// One piece of text, as the runtime sees it: where it starts and how long it is.
///
/// The generated code never holds one of these. It passes an *index*, and the runtime
/// looks it up in a table whose address was baked in when the module was compiled — so
/// nothing target-specific reaches QIR, and the pointer exists only on this side.
#[repr(C)]
struct Piece {
    at: *const u8,
    len: usize,
}

/// What compiled code is given a pointer to, and the only address it ever holds.
///
/// Its address is baked into the code when the module is compiled, so reading the stop
/// flag is a plain load rather than a call — which is what makes checking after every
/// operation that can fail affordable.
#[repr(C)]
struct Runtime {
    /// The module's text, for `print-text`.
    pieces: *const Piece,
    /// Zero while the program is running. A [`qir::Trap`] code once it has stopped.
    ///
    /// This is the answer to the thing compiled code could not do: it has nowhere to
    /// put a failure and no way to unwind, so the failure is written down here and the
    /// generated code checks afterwards. Aborting the process was the previous answer,
    /// and it lost the whole run rather than one program.
    stopped: i64,
}

/// Where `stopped` sits inside [`Runtime`], for the load the generated code emits.
const STOPPED_AT: i32 = std::mem::size_of::<*const Piece>() as i32;

thread_local! {
    /// The heap compiled code allocates from.
    ///
    /// Allocated and never freed — the first stage of the collector, which needs no
    /// stack maps and no cooperation from any backend. A handle is an index into this.
    /// Thread-local for the same reason the sink is, and tolerable for the same reason:
    /// a worker in the oracle owns its thread.
    static HEAP: std::cell::RefCell<Vec<Vec<i64>>> = const { std::cell::RefCell::new(Vec::new()) };

    /// Where a running program's output goes, when something is collecting it.
    ///
    /// Two of them, because a program says which it means. A thread-local rather than an
    /// argument because the generated code calls a plain C function and there is nowhere
    /// to put a writer; tolerable because a worker in the oracle owns its thread.
    static SINK: std::cell::RefCell<Option<(Vec<u8>, Vec<u8>)>> =
        const { std::cell::RefCell::new(None) };

    /// Every exact number the program has made. An `e` value is an index into this.
    ///
    /// Allocated and never freed, like the heap above. The arithmetic itself is
    /// `quench_num`'s, which is the same code the interpreter calls — so the two cannot
    /// answer differently, however large the numbers get.
    static EXACTS: std::cell::RefCell<Vec<quench_num::Exact>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Put an exact number away, and give back the handle to it.
fn keep(value: quench_num::Exact) -> i64 {
    EXACTS.with(|exacts| {
        let mut exacts = exacts.borrow_mut();
        exacts.push(value);
        exacts.len() as i64 - 1
    })
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_read(rt: *mut Runtime, index: i64) -> i64 {
    let piece = unsafe { &*(*rt).pieces.add(index as usize) };
    let bytes = unsafe { std::slice::from_raw_parts(piece.at, piece.len) };
    let text = std::str::from_utf8(bytes).expect("the source was text");
    let read = quench_num::Exact::parse(text)
        .expect("refused by the checker: an `e` that is not a number");
    keep(read)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_add(_rt: *mut Runtime, a: i64, b: i64) -> i64 {
    let answer = EXACTS.with(|e| {
        let e = e.borrow();
        e[a as usize].add(&e[b as usize])
    });
    keep(answer)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_sub(_rt: *mut Runtime, a: i64, b: i64) -> i64 {
    let answer = EXACTS.with(|e| {
        let e = e.borrow();
        e[a as usize].sub(&e[b as usize])
    });
    keep(answer)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_mul(_rt: *mut Runtime, a: i64, b: i64) -> i64 {
    let answer = EXACTS.with(|e| {
        let e = e.borrow();
        e[a as usize].mul(&e[b as usize])
    });
    keep(answer)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_div(rt: *mut Runtime, a: i64, b: i64) -> i64 {
    let answer = EXACTS.with(|e| {
        let e = e.borrow();
        e[a as usize].div(&e[b as usize])
    });
    match answer {
        Ok(value) => keep(value),
        // Nothing sensible to hand back, so zero -- and the generated code is about to
        // notice the flag and stop before it can use this.
        Err(_) => {
            stop(rt, qir::Trap::DividedByZero);
            0
        }
    }
}

/// Why a power had no answer, as a reason to stop.
fn no_power(trouble: quench_num::NoPower) -> qir::Trap {
    match trouble {
        quench_num::NoPower::Negative => qir::Trap::NegativePower,
        quench_num::NoPower::Fractional => qir::Trap::FractionalPower,
        quench_num::NoPower::TooLarge => qir::Trap::Overflowed,
    }
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_pow(rt: *mut Runtime, a: i64, b: i64) -> i64 {
    let answer = EXACTS.with(|e| {
        let e = e.borrow();
        e[a as usize].power(&e[b as usize])
    });
    match answer {
        Ok(value) => keep(value),
        Err(trouble) => {
            stop(rt, no_power(trouble));
            0
        }
    }
}

/// Called by compiled code. Not called by anything else.
extern "C" fn pow_i64(rt: *mut Runtime, base: i64, exponent: i64) -> i64 {
    match quench_num::power_i64(base, exponent, true) {
        Ok(n) => n,
        Err(trouble) => {
            stop(rt, no_power(trouble));
            0
        }
    }
}

/// Called by compiled code. Not called by anything else.
extern "C" fn pow_i64_trapping(rt: *mut Runtime, base: i64, exponent: i64) -> i64 {
    match quench_num::power_i64(base, exponent, false) {
        Ok(n) => n,
        Err(trouble) => {
            stop(rt, no_power(trouble));
            0
        }
    }
}

/// Called by compiled code. Not called by anything else.
extern "C" fn text_compare(rt: *mut Runtime, a: i64, b: i64) -> i64 {
    match piece_of(rt, a).cmp(piece_of(rt, b)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_compare(_rt: *mut Runtime, a: i64, b: i64) -> i64 {
    EXACTS.with(|e| {
        let e = e.borrow();
        match e[a as usize].cmp(&e[b as usize]) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    })
}

/// Called by compiled code. Not called by anything else.
extern "C" fn print_exact(_rt: *mut Runtime, stream: i64, value: i64) -> i64 {
    let shown = EXACTS.with(|e| e.borrow()[value as usize].to_string());
    write_out(stream, shown.as_bytes());
    0
}

/// Write down why the program is stopping, for the generated code to notice.
///
/// Safe because the pointer was baked in from a `Box` that outlives the code holding it.
fn stop(rt: *mut Runtime, trap: qir::Trap) {
    unsafe { (*rt).stopped = trap as i64 };
}

/// Called by compiled code. Not called by anything else.
extern "C" fn array_new(_rt: *mut Runtime, len: i64) -> i64 {
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        heap.push(vec![0; len.max(0) as usize]);
        heap.len() as i64 - 1
    })
}

/// Called by compiled code. Not called by anything else.
extern "C" fn array_set(rt: *mut Runtime, handle: i64, at: i64, value: i64) -> i64 {
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        let array = &mut heap[handle as usize];
        match at.checked_sub(1).and_then(|i| usize::try_from(i).ok()).and_then(|i| array.get_mut(i))
        {
            Some(cell) => *cell = value,
            None => stop(rt, qir::Trap::OutsideTheArray),
        }
        0
    })
}

/// Called by compiled code. Not called by anything else.
extern "C" fn array_get(rt: *mut Runtime, handle: i64, at: i64) -> i64 {
    HEAP.with(|heap| {
        let heap = heap.borrow();
        match at.checked_sub(1).and_then(|i| usize::try_from(i).ok()).and_then(|i| heap[handle as usize].get(i))
        {
            Some(value) => *value,
            // Nothing sensible to hand back, so zero -- and the generated code is about
            // to notice the flag and stop before it can use this.
            None => {
                stop(rt, qir::Trap::OutsideTheArray);
                0
            }
        }
    })
}

/// Called by compiled code. Not called by anything else.
extern "C" fn array_len(_rt: *mut Runtime, handle: i64) -> i64 {
    HEAP.with(|heap| heap.borrow()[handle as usize].len() as i64)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn array_push(_rt: *mut Runtime, handle: i64, value: i64) -> i64 {
    HEAP.with(|heap| heap.borrow_mut()[handle as usize].push(value));
    0
}

/// Called by compiled code. Not called by anything else.
extern "C" fn array_copy(_rt: *mut Runtime, handle: i64) -> i64 {
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        let of = heap[handle as usize].clone();
        heap.push(of);
        heap.len() as i64 - 1
    })
}

/// One piece of the module's text, as bytes.
fn piece_of(rt: *mut Runtime, index: i64) -> &'static [u8] {
    let piece = unsafe { &*(*rt).pieces.add(index as usize) };
    unsafe { std::slice::from_raw_parts(piece.at, piece.len) }
}

/// Called by compiled code. Not called by anything else.
extern "C" fn array_equal(rt: *mut Runtime, a: i64, b: i64, kind: i64, depth: i64) -> i64 {
    let kind = qir::Elements::from_code(kind).expect("the lowering wrote this constant");
    i64::from(alike(rt, a, b, kind, depth))
}

/// Whether two arrays hold the same things, following handles as far down as they go.
fn alike(rt: *mut Runtime, a: i64, b: i64, kind: qir::Elements, depth: i64) -> bool {
    let (left, right) = HEAP.with(|heap| {
        let heap = heap.borrow();
        (heap[a as usize].clone(), heap[b as usize].clone())
    });
    if left.len() != right.len() {
        return false;
    }
    left.iter().zip(&right).all(|(x, y)| {
        if depth > 0 {
            return alike(rt, *x, *y, kind, depth - 1);
        }
        match kind {
            qir::Elements::Exact => {
                EXACTS.with(|e| e.borrow()[*x as usize] == e.borrow()[*y as usize])
            }
            qir::Elements::Text => piece_of(rt, *x) == piece_of(rt, *y),
            _ => x == y,
        }
    })
}

/// Called by compiled code. Not called by anything else.
extern "C" fn print_array(
    rt: *mut Runtime,
    stream: i64,
    handle: i64,
    kind: i64,
    depth: i64,
) -> i64 {
    let kind = qir::Elements::from_code(kind).expect("the lowering wrote this constant");
    write_out(stream, shown(rt, handle, kind, depth).as_bytes());
    0
}

/// What one array says, following handles as far down as it goes.
fn shown(rt: *mut Runtime, handle: i64, kind: qir::Elements, depth: i64) -> String {
    let elements = HEAP.with(|heap| heap.borrow()[handle as usize].clone());
    let parts: Vec<String> = elements
        .iter()
        .map(|value| {
            if depth > 0 {
                return shown(rt, *value, kind, depth - 1);
            }
            match kind {
                qir::Elements::I64 => value.to_string(),
                qir::Elements::Bool => if *value != 0 { "true" } else { "false" }.to_string(),
                qir::Elements::Text => {
                    format!("*{}*", String::from_utf8_lossy(piece_of(rt, *value)))
                }
                qir::Elements::Exact => EXACTS.with(|e| e.borrow()[*value as usize].to_string()),
            }
        })
        .collect();
    qir::show_array(&parts)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn print_bool(_rt: *mut Runtime, stream: i64, value: i64) -> i64 {
    write_out(stream, if value != 0 { b"true" } else { b"false" });
    0
}

/// Called by compiled code. Not called by anything else.
extern "C" fn print_i64(_rt: *mut Runtime, stream: i64, value: i64) -> i64 {
    write_out(stream, value.to_string().as_bytes());
    0
}

/// Called by compiled code. Not called by anything else.
extern "C" fn print_text(rt: *mut Runtime, stream: i64, index: i64) -> i64 {
    // Safe because `compile` verified the index against the module's text before any of
    // this existed, and the table outlives the code that names it.
    let piece = unsafe { &*(*rt).pieces.add(index as usize) };
    let bytes = unsafe { std::slice::from_raw_parts(piece.at, piece.len) };
    write_out(stream, bytes);
    0
}

fn write_out(stream: i64, bytes: &[u8]) {
    let to_err = stream == qir::Stream::Err as i64;
    SINK.with(|sink| match &mut *sink.borrow_mut() {
        Some((out, err)) => {
            if to_err { err } else { out }.extend_from_slice(bytes)
        }
        None => {
            use std::io::Write as _;
            if to_err {
                let _ = std::io::stderr().write_all(bytes);
            } else {
                let _ = std::io::stdout().write_all(bytes);
            }
        }
    });
}

fn cond(op: qir::CmpOp) -> IntCC {
    match op {
        qir::CmpOp::Eq => IntCC::Equal,
        qir::CmpOp::Ne => IntCC::NotEqual,
        qir::CmpOp::Lt => IntCC::SignedLessThan,
        qir::CmpOp::Le => IntCC::SignedLessThanOrEqual,
        qir::CmpOp::Gt => IntCC::SignedGreaterThan,
        qir::CmpOp::Ge => IntCC::SignedGreaterThanOrEqual,
    }
}

/// Compile a whole module at [`Optimise::None`], which is what the Dev JIT is.
pub fn compile(module: &qir::Module) -> Result<Compiled, Error> {
    compile_with(module, Optimise::None)
}

/// Compile at a chosen level.
///
/// The Dev JIT itself never uses anything but [`Optimise::None`] — see the module docs
/// for why that is the point rather than a limitation. This exists for the oracle, which
/// checks that every level answers the same, and for which a level that is never
/// compiled at is a level never tested.
pub fn compile_with(module: &qir::Module, optimise: Optimise) -> Result<Compiled, Error> {
    qir::verify(module).map_err(Error::Invalid)?;

    let entry_id = module.entry.ok_or(Error::NoEntry)?;
    let entry_fn = module.func(entry_id);
    if !entry_fn.params.is_empty() {
        return Err(Error::Entry(format!(
            "`{}` takes {} argument(s), and an entry is called with none",
            entry_fn.name,
            entry_fn.params.len()
        )));
    }
    if entry_fn.ret != qir::Ty::I64 {
        return Err(Error::Entry(format!(
            "`{}` returns {}, and an entry has to return i64",
            entry_fn.name,
            entry_fn.ret.name()
        )));
    }

    let mut flags = settings::builder();
    let level = match optimise {
        Optimise::None => "none",
        Optimise::Speed => "speed",
        Optimise::SpeedAndSize => "speed_and_size",
    };
    flags.set("opt_level", level).map_err(|e| Error::Backend(e.to_string()))?;
    let isa = cranelift_native::builder()
        .map_err(|e| Error::Backend(e.to_string()))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| Error::Backend(e.to_string()))?;
    let mut builder = JITBuilder::with_isa(isa, default_libcall_names());
    // The one symbol generated code is allowed to reach outside itself for.
    builder.symbol("quench_print_text", print_text as *const u8);
    builder.symbol("quench_print_i64", print_i64 as *const u8);
    builder.symbol("quench_print_bool", print_bool as *const u8);
    builder.symbol("quench_array_new", array_new as *const u8);
    builder.symbol("quench_array_set", array_set as *const u8);
    builder.symbol("quench_array_get", array_get as *const u8);
    builder.symbol("quench_array_len", array_len as *const u8);
    builder.symbol("quench_print_array", print_array as *const u8);
    builder.symbol("quench_array_copy", array_copy as *const u8);
    builder.symbol("quench_array_push", array_push as *const u8);
    builder.symbol("quench_array_equal", array_equal as *const u8);
    builder.symbol("quench_exact_read", exact_read as *const u8);
    builder.symbol("quench_exact_add", exact_add as *const u8);
    builder.symbol("quench_exact_sub", exact_sub as *const u8);
    builder.symbol("quench_exact_mul", exact_mul as *const u8);
    builder.symbol("quench_exact_div", exact_div as *const u8);
    builder.symbol("quench_exact_compare", exact_compare as *const u8);
    builder.symbol("quench_print_exact", print_exact as *const u8);
    builder.symbol("quench_text_compare", text_compare as *const u8);
    builder.symbol("quench_exact_pow", exact_pow as *const u8);
    builder.symbol("quench_pow_i64", pow_i64 as *const u8);
    builder.symbol("quench_pow_i64_trapping", pow_i64_trapping as *const u8);
    let mut jit = JITModule::new(builder);

    // The text, owned here and pointed at by a table whose address the code will carry.
    let text: Vec<String> = module.text.clone();
    let pieces: Box<[Piece]> = text
        .iter()
        .map(|s| Piece { at: s.as_ptr(), len: s.len() })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let runtime = Box::new(Runtime { pieces: pieces.as_ptr(), stopped: 0 });
    let table = &*runtime as *const Runtime as i64;

    // Declare everything before defining anything, so a call can name a function that has
    // not been compiled yet -- including the one making the call.
    let mut declared = Vec::with_capacity(module.functions.len());
    for func in &module.functions {
        let mut sig = jit.make_signature();
        for param in &func.params {
            sig.params.push(AbiParam::new(clif_ty(*param)));
        }
        sig.returns.push(AbiParam::new(clif_ty(func.ret)));
        let id = jit
            .declare_function(&func.name, Linkage::Export, &sig)
            .map_err(|e| Error::Backend(e.to_string()))?;
        declared.push((id, sig));
    }

    let target = jit.target_config();
    // One declaration per host function, each with its own shape: the table first, then
    // whatever that one takes. They all give back an i64 -- a handle is one too.
    let mut hosts = Vec::new();
    for (host, symbol) in [
        (qir::Host::PrintText, "quench_print_text"),
        (qir::Host::PrintI64, "quench_print_i64"),
        (qir::Host::PrintBool, "quench_print_bool"),
        (qir::Host::ArrayNew, "quench_array_new"),
        (qir::Host::ArraySet, "quench_array_set"),
        (qir::Host::ArrayGet, "quench_array_get"),
        (qir::Host::ArrayLen, "quench_array_len"),
        (qir::Host::PrintArray, "quench_print_array"),
        (qir::Host::ArrayCopy, "quench_array_copy"),
        (qir::Host::ArrayPush, "quench_array_push"),
        (qir::Host::ArrayEqual, "quench_array_equal"),
        (qir::Host::ExactRead, "quench_exact_read"),
        (qir::Host::ExactAdd, "quench_exact_add"),
        (qir::Host::ExactSub, "quench_exact_sub"),
        (qir::Host::ExactMul, "quench_exact_mul"),
        (qir::Host::ExactDiv, "quench_exact_div"),
        (qir::Host::ExactCompare, "quench_exact_compare"),
        (qir::Host::PrintExact, "quench_print_exact"),
        (qir::Host::TextCompare, "quench_text_compare"),
        (qir::Host::ExactPow, "quench_exact_pow"),
        (qir::Host::PowI64, "quench_pow_i64"),
        (qir::Host::PowI64Trapping, "quench_pow_i64_trapping"),
    ] {
        let mut sig = jit.make_signature();
        sig.params.push(AbiParam::new(types::I64)); // the table
        for _ in host.params() {
            sig.params.push(AbiParam::new(types::I64));
        }
        sig.returns.push(AbiParam::new(types::I64));
        let id = jit
            .declare_function(symbol, Linkage::Import, &sig)
            .map_err(|e| Error::Backend(e.to_string()))?;
        hosts.push((host, id));
    }

    let mut ctx = jit.make_context();
    let mut fctx = FunctionBuilderContext::new();
    for (i, func) in module.functions.iter().enumerate() {
        ctx.func.signature = declared[i].1.clone();
        // Taken before the builder borrows the function, since both want it mutably.
        let refs: Vec<FuncRef> =
            declared.iter().map(|(id, _)| jit.declare_func_in_func(*id, &mut ctx.func)).collect();
        let host_refs: Vec<(qir::Host, FuncRef)> = hosts
            .iter()
            .map(|(host, id)| (*host, jit.declare_func_in_func(*id, &mut ctx.func)))
            .collect();
        lower(func, &mut ctx.func, &mut fctx, &refs, &host_refs, table, target);
        jit.define_function(declared[i].0, &mut ctx).map_err(|e| Error::Backend(e.to_string()))?;
        jit.clear_context(&mut ctx);
    }

    jit.finalize_definitions().map_err(|e| Error::Backend(e.to_string()))?;
    let entry = jit.get_finalized_function(declared[entry_id.0 as usize].0);
    let callable = module
        .functions
        .iter()
        .enumerate()
        .filter(|(_, f)| f.params.is_empty() && f.ret == qir::Ty::I64)
        .map(|(i, f)| (f.name.clone(), jit.get_finalized_function(declared[i].0)))
        .collect();
    Ok(Compiled {
        _jit: jit,
        _text: text,
        _pieces: pieces,
        _tables: module.tables.clone(),
        runtime,
        entry,
        callable,
    })
}

/// Lower one QIR function into one Cranelift function.
///
/// A near-transliteration, which is the intent: QIR is already SSA with block parameters,
/// which is what Cranelift wants, so there is nothing to decide here. Anything that felt
/// like a decision at this point would belong further up.
fn lower(
    func: &qir::Function,
    clif: &mut ClifFunction,
    fctx: &mut FunctionBuilderContext,
    refs: &[FuncRef],
    hosts: &[(qir::Host, FuncRef)],
    table: i64,
    target: TargetFrontendConfig,
) {
    let mut b = FunctionBuilder::new(clif, fctx);

    let blocks: Vec<_> = func.blocks.iter().map(|_| b.create_block()).collect();
    // Where the program goes when the runtime has written down a reason to stop. It
    // returns whatever the signature needs and means nothing; `Compiled::outcome` reads
    // the reason rather than the value.
    let stopping = b.create_block();
    b.append_block_params_for_function_params(blocks[0]);
    for (i, block) in func.blocks.iter().enumerate().skip(1) {
        for param in &block.params {
            b.append_block_param(blocks[i], clif_ty(func.ty_of(*param)));
        }
    }

    let mut vals: Vec<Option<ClifValue>> = vec![None; func.value_tys.len()];

    for (i, block) in func.blocks.iter().enumerate() {
        b.switch_to_block(blocks[i]);

        let incoming = b.block_params(blocks[i]).to_vec();
        for (n, param) in block.params.iter().enumerate() {
            vals[param.0 as usize] = Some(incoming[n]);
        }

        for (result, inst) in &block.insts {
            let v = match inst {
                qir::Inst::ConstI64(n) => b.ins().iconst(types::I64, *n),
                qir::Inst::ConstBool(t) => b.ins().iconst(types::I8, i64::from(*t)),
                qir::Inst::ConstText(at) | qir::Inst::ConstHandle(at) => {
                    b.ins().iconst(types::I64, i64::from(*at))
                }
                qir::Inst::CallHost { host, args } => {
                    let which = hosts
                        .iter()
                        .find(|(known, _)| known == host)
                        .map(|(_, r)| *r)
                        .expect("every host function is declared");
                    // The table's address is a constant of this compilation. QIR carried
                    // an index; the pointer is put on here and goes no further.
                    let mut given = vec![b.ins().iconst(types::I64, table)];
                    for arg in args {
                        let value = vals[arg.0 as usize].unwrap();
                        // A bool lives in an i8 here and the runtime takes an i64, so it
                        // is widened at the boundary rather than anywhere QIR can see.
                        // Keyed on what the value *is*, not on what the host's table
                        // says: one argument is whatever the array holds.
                        given.push(if func.ty_of(*arg) == qir::Ty::Bool {
                            b.ins().uextend(types::I64, value)
                        } else {
                            value
                        });
                    }
                    let call = b.ins().call(which, &given);
                    let mut answer = b.inst_results(call)[0];
                    // And narrowed on the way back, for the same reason: the runtime
                    // answers in an i64 and a bool lives in an i8 here.
                    if func.ty_of(*result) == qir::Ty::Bool {
                        answer = b.ins().ireduce(types::I8, answer);
                    }
                    if host.can_stop() {
                        // A load and a branch, which is what a baked-in runtime address
                        // buys: asking whether to stop costs no call.
                        stop_if_stopped(&mut b, table, stopping);
                    }
                    answer
                }
                qir::Inst::Bin { op, lhs, rhs } => {
                    let (l, r) = (vals[lhs.0 as usize].unwrap(), vals[rhs.0 as usize].unwrap());
                    match op {
                        // A `Bool` is already nought or one, so this is the whole
                        // operation and needs no normalising afterwards.
                        qir::BinOp::And => b.ins().band(l, r),
                        qir::BinOp::Or => b.ins().bor(l, r),
                        qir::BinOp::Add => b.ins().iadd(l, r),
                        qir::BinOp::Sub => b.ins().isub(l, r),
                        qir::BinOp::Mul => b.ins().imul(l, r),
                        qir::BinOp::AddTrapping
                        | qir::BinOp::SubTrapping
                        | qir::BinOp::MulTrapping => {
                            trapping(&mut b, table, stopping, *op, l, r)
                        }
                        // The processor's own division, which rounds toward zero --
                        // after the two cases it would fault on have been refused.
                        qir::BinOp::DivTruncated => {
                            guarded_division(&mut b, table, stopping, l, r);
                            b.ins().sdiv(l, r)
                        }
                        qir::BinOp::RemTruncated => {
                            guarded_division(&mut b, table, stopping, l, r);
                            b.ins().srem(l, r)
                        }
                        // No processor floors, so it is built: divide, then correct by
                        // one step when the remainder disagrees in sign with the divisor,
                        // which is exactly when truncation rounded the wrong way.
                        //
                        // `sdiv` and `srem` still carry the traps, so a zero divisor and
                        // `i64::MIN / -1` stop here as they do everywhere else.
                        qir::BinOp::DivFloored => {
                            guarded_division(&mut b, table, stopping, l, r);
                            let quotient = b.ins().sdiv(l, r);
                            let rest = b.ins().srem(l, r);
                            let wrong_way = signs_disagree(&mut b, rest, r);
                            let one_less = b.ins().iadd_imm_s(quotient, -1);
                            b.ins().select(wrong_way, one_less, quotient)
                        }
                        qir::BinOp::RemFloored => {
                            guarded_division(&mut b, table, stopping, l, r);
                            let rest = b.ins().srem(l, r);
                            let wrong_way = signs_disagree(&mut b, rest, r);
                            let shifted = b.ins().iadd(rest, r);
                            b.ins().select(wrong_way, shifted, rest)
                        }
                    }
                }
                qir::Inst::Cmp { op, lhs, rhs } => {
                    let (l, r) = (vals[lhs.0 as usize].unwrap(), vals[rhs.0 as usize].unwrap());
                    b.ins().icmp(cond(*op), l, r)
                }
                // Bool is 0 or 1 and nothing else, so flipping the low bit is negation.
                qir::Inst::Not(v) => b.ins().bxor_imm_u(vals[v.0 as usize].unwrap(), 1),
                qir::Inst::Call { func: callee, args } => {
                    let args: Vec<ClifValue> =
                        args.iter().map(|a| vals[a.0 as usize].unwrap()).collect();
                    let call = b.ins().call(refs[callee.0 as usize], &args);
                    let answer = b.inst_results(call)[0];
                    // A callee that stopped leaves the flag set, and its caller has to
                    // stop as well rather than carrying on with what it handed back.
                    stop_if_stopped(&mut b, table, stopping);
                    answer
                }
            };
            vals[result.0 as usize] = Some(v);
        }

        match &block.term {
            qir::Term::Ret(v) => {
                let v = vals[v.0 as usize].unwrap();
                b.ins().return_(&[v]);
            }
            qir::Term::Jump { to, args } => {
                let args: Vec<BlockArg> =
                    args.iter().map(|a| vals[a.0 as usize].unwrap().into()).collect();
                b.ins().jump(blocks[to.0 as usize], &args);
            }
            qir::Term::BrIf { cond, then, otherwise } => {
                let c = vals[cond.0 as usize].unwrap();
                let t: Vec<BlockArg> =
                    then.args.iter().map(|a| vals[a.0 as usize].unwrap().into()).collect();
                let o: Vec<BlockArg> =
                    otherwise.args.iter().map(|a| vals[a.0 as usize].unwrap().into()).collect();
                b.ins().brif(c, blocks[then.block.0 as usize], &t, blocks[otherwise.block.0 as usize], &o);
            }
        }
    }

    // Whatever the signature promised, given back so the frame can be left. It is not
    // an answer and nothing reads it.
    b.switch_to_block(stopping);
    let nothing = match clif_ty(func.ret) {
        types::I8 => b.ins().iconst(types::I8, 0),
        ty => b.ins().iconst(ty, 0),
    };
    b.ins().return_(&[nothing]);

    b.seal_all_blocks();
    b.finalize(target);
}
