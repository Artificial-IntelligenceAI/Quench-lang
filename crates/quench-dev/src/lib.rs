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
use cranelift_codegen::ir::{
    condcodes::FloatCC, types, AbiParam, BlockArg, FuncRef, InstBuilder, MemFlagsData,
};
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
    /// Where compiled code writes the handles it is holding. Boxed and kept here so the
    /// address baked into the code outlives the code.
    _roots: Box<[i64]>,
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
        HEAP.with(|heap| {
            *heap.borrow_mut() = quench_heap::Heap::laid_out(&self._tables, &self._text)
        });
        unsafe { (*(&*self.runtime as *const Runtime as *mut Runtime)).used = 0 };
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

    /// The same, keeping whatever it printed rather than letting it out.
    ///
    /// What a program *prints* is the only thing it says that nothing else can see:
    /// two engines agreeing on an answer and differing on the text beside it is a
    /// disagreement, and until this there was no way to look.
    pub fn call_capturing(&self, name: &str) -> Option<(qir::Outcome, Printed)> {
        SINK.with(|sink| *sink.borrow_mut() = Some((Vec::new(), Vec::new())));
        let outcome = self.call_outcome(name);
        let (out, err) = SINK.with(|sink| sink.borrow_mut().take()).unwrap_or_default();
        outcome.map(|outcome| {
            (
                outcome,
                Printed {
                    out: String::from_utf8_lossy(&out).into_owned(),
                    err: String::from_utf8_lossy(&err).into_owned(),
                },
            )
        })
    }

    /// What the heap kept, which is the one thing the oracle cannot see.
    pub fn kept(&self) -> (usize, usize, usize, usize) {
        HEAP.with(|heap| {
            let heap = heap.borrow();
            let (a, t, e) = heap.live();
            (a, t, e, heap.collections)
        })
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
        HEAP.with(|heap| {
            *heap.borrow_mut() = quench_heap::Heap::laid_out(&self._tables, &self._text)
        });
        unsafe { (*(&*self.runtime as *const Runtime as *mut Runtime)).used = 0 };
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
        // The same three read as unsigned, which only `u64` needs: every narrower
        // unsigned type notices its own overflow when it is narrowed.
        qir::BinOp::AddTrappingU => {
            let r = b.ins().uadd_overflow(lhs, rhs);
            (r.0, r.1)
        }
        qir::BinOp::SubTrappingU => {
            let r = b.ins().usub_overflow(lhs, rhs);
            (r.0, r.1)
        }
        qir::BinOp::MulTrappingU => {
            let r = b.ins().umul_overflow(lhs, rhs);
            (r.0, r.1)
        }
        _ => unreachable!("only the trapping six reach here"),
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
        qir::Ty::Handle | qir::Ty::Exact | qir::Ty::Decimal => types::I64,
        qir::Ty::F64 => types::F64,
        // A `b16` is carried in an `f32`, because there is no half to put in a
        // register and the carrier gives binary16's own answers anyway.
        qir::Ty::F32 | qir::Ty::F16 => types::F32,
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
    /// Every handle compiled code is holding, and which space each is in.
    ///
    /// This is the thing the interpreter never needed: its call stack is a list it owns,
    /// so its roots are simply there. Compiled code keeps its handles in registers and
    /// stack slots that only the machine knows about, so it writes them out here as it
    /// goes — one slot per reference-typed value in the function, filled where the value
    /// is defined. See `notes/the-collector-earns-its-place.md`.
    roots: *mut i64,
    /// How many of `roots` are in use. Every function moves this on the way in and puts
    /// it back on the way out, which is what makes the frames a stack.
    used: i64,
    /// Zero while the program is running. A [`qir::Trap`] code once it has stopped.
    ///
    /// This is the answer to the thing compiled code could not do: it has nowhere to
    /// put a failure and no way to unwind, so the failure is written down here and the
    /// generated code checks afterwards. Aborting the process was the previous answer,
    /// and it lost the whole run rather than one program.
    stopped: i64,
}

/// The byte order of the machine this is compiling for, which is the machine it is
/// running on: a JIT's target is its host. Cranelift's `bitcast` insists on being told,
/// and moving the bits of a float into an integer register is not a reordering.
fn native_order() -> cranelift_codegen::ir::Endianness {
    if cfg!(target_endian = "little") {
        cranelift_codegen::ir::Endianness::Little
    } else {
        cranelift_codegen::ir::Endianness::Big
    }
}

/// Where each field sits inside [`Runtime`], for the loads and stores generated code
/// emits. A pointer and an `i64` are both eight bytes on everything Quench targets.
const ROOTS_AT: i32 = 8;
const USED_AT: i32 = 16;
const STOPPED_AT: i32 = 24;

/// How many roots there is room for. A program deeper than this has too many frames to
/// be doing anything sensible, and stops the same way one that recursed too far does.
const ROOM: usize = 1 << 18;

thread_local! {
    /// Everything compiled code has made, and everything the module was written with.
    ///
    /// The same heap the interpreter uses, because the object model is a contract
    /// between the engines rather than each one's own idea. Thread-local for the same
    /// reason the sink is, and tolerable for the same reason: a worker in the oracle
    /// owns its thread.
    static HEAP: std::cell::RefCell<quench_heap::Heap> =
        std::cell::RefCell::new(quench_heap::Heap::default());

    /// Where a running program's output goes, when something is collecting it.
    ///
    /// Two of them, because a program says which it means. A thread-local rather than an
    /// argument because the generated code calls a plain C function and there is nowhere
    /// to put a writer; tolerable because a worker in the oracle owns its thread.
    static SINK: std::cell::RefCell<Option<(Vec<u8>, Vec<u8>)>> =
        const { std::cell::RefCell::new(None) };


}

/// One piece of text, whatever it was made by.
fn text_of(index: i64) -> String {
    HEAP.with(|heap| heap.borrow().said(index).to_string())
}

/// Collect, if enough has been made to be worth it.
///
/// Called at the *top* of every host call that allocates, and nowhere else. Before one
/// is the moment when everything compiled code holds has already been written to its
/// slot and the thing about to be made does not exist yet — so nothing is missed and
/// nothing brand new is swept.
fn maybe_collect(rt: *mut Runtime) {
    if !HEAP.with(|heap| heap.borrow().worth_collecting()) {
        return;
    }
    let (roots, used) = unsafe { ((*rt).roots, (*rt).used) };
    let packed = unsafe { std::slice::from_raw_parts(roots, used.max(0) as usize) };
    HEAP.with(|heap| heap.borrow_mut().collect_packed(packed));
}

/// Put an array away, and give back the handle to it.
fn keep_array(holds: qir::Elements, depth: i64, values: Vec<i64>) -> i64 {
    HEAP.with(|heap| heap.borrow_mut().make(holds, depth, values))
}

/// Put an exact number away, and give back the handle to it.
fn keep(value: quench_num::Exact) -> i64 {
    HEAP.with(|heap| heap.borrow_mut().exact(value))
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_read(rt: *mut Runtime, index: i64) -> i64 {
    maybe_collect(rt);
    let read = quench_num::Exact::parse(&text_of(index))
        .expect("refused by the checker: an `e` that is not a number");
    keep(read)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_add(rt: *mut Runtime, a: i64, b: i64) -> i64 {
    maybe_collect(rt);
    let answer = HEAP.with(|h| {
        let h = h.borrow();
        h.exactly(a).add(h.exactly(b))
    });
    keep(answer)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_sub(rt: *mut Runtime, a: i64, b: i64) -> i64 {
    maybe_collect(rt);
    let answer = HEAP.with(|h| {
        let h = h.borrow();
        h.exactly(a).sub(h.exactly(b))
    });
    keep(answer)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_mul(rt: *mut Runtime, a: i64, b: i64) -> i64 {
    maybe_collect(rt);
    let answer = HEAP.with(|h| {
        let h = h.borrow();
        h.exactly(a).mul(h.exactly(b))
    });
    keep(answer)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_div(rt: *mut Runtime, a: i64, b: i64) -> i64 {
    maybe_collect(rt);
    let answer = HEAP.with(|h| {
        let h = h.borrow();
        h.exactly(a).div(h.exactly(b))
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

/// Put a decimal away, and give back the handle to it.
fn keep_decimal(value: quench_num::Decimal) -> i64 {
    HEAP.with(|heap| heap.borrow_mut().decimal(value))
}

/// Which decimal format a digit count names. The lowering only ever writes the two.
fn decimal_format(digits: i64) -> quench_num::Format {
    if digits == 7 { quench_num::D32 } else { quench_num::D64 }
}

/// Called by compiled code. Not called by anything else.
extern "C" fn decimal_read(rt: *mut Runtime, index: i64, digits: i64) -> i64 {
    maybe_collect(rt);
    let read = quench_num::Decimal::parse(&text_of(index), decimal_format(digits))
        .expect("refused by the checker: a decimal that is not a number");
    keep_decimal(read)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn decimal_add(rt: *mut Runtime, a: i64, b: i64, digits: i64) -> i64 {
    maybe_collect(rt);
    let answer =
        HEAP.with(|h| {
            let h = h.borrow();
            h.decimally(a).add(h.decimally(b), decimal_format(digits))
        });
    keep_decimal(answer)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn decimal_sub(rt: *mut Runtime, a: i64, b: i64, digits: i64) -> i64 {
    maybe_collect(rt);
    let answer =
        HEAP.with(|h| {
            let h = h.borrow();
            h.decimally(a).sub(h.decimally(b), decimal_format(digits))
        });
    keep_decimal(answer)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn decimal_mul(rt: *mut Runtime, a: i64, b: i64, digits: i64) -> i64 {
    maybe_collect(rt);
    let answer =
        HEAP.with(|h| {
            let h = h.borrow();
            h.decimally(a).mul(h.decimally(b), decimal_format(digits))
        });
    keep_decimal(answer)
}

/// Called by compiled code. Not called by anything else.
///
/// No trap on a divisor of nought, unlike [`exact_div`]: a decimal float answers that
/// with infinity, which is the difference between a float and a ratio.
extern "C" fn decimal_div(rt: *mut Runtime, a: i64, b: i64, digits: i64) -> i64 {
    maybe_collect(rt);
    let answer =
        HEAP.with(|h| {
            let h = h.borrow();
            h.decimally(a).div(h.decimally(b), decimal_format(digits))
        });
    keep_decimal(answer)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn decimal_compare(_rt: *mut Runtime, a: i64, b: i64) -> i64 {
    HEAP.with(|h| {
        let h = h.borrow();
        match h.decimally(a).compare(h.decimally(b)) {
            Some(std::cmp::Ordering::Less) => -1,
            Some(std::cmp::Ordering::Equal) => 0,
            Some(std::cmp::Ordering::Greater) => 1,
            // Not-a-number, which is none of the three.
            None => 2,
        }
    })
}

/// Called by compiled code. Not called by anything else.
extern "C" fn print_decimal(_rt: *mut Runtime, stream: i64, value: i64) -> i64 {
    let shown = HEAP.with(|h| h.borrow().decimally(value).to_string());
    write_out(stream, shown.as_bytes());
    0
}

/// Put a new piece of text away, and give back which piece it is.
fn keep_text(written: String) -> i64 {
    HEAP.with(|h| h.borrow_mut().text(written))
}

/// Called by compiled code. Not called by anything else.
///
/// The `say_*` family: what the matching `print_*` would have written, kept rather than
/// let out. Each is the same expression as the one it mirrors.
extern "C" fn say_i64(rt: *mut Runtime, value: i64) -> i64 {
    maybe_collect(rt);
    keep_text(value.to_string())
}

/// Called by compiled code. Not called by anything else.
extern "C" fn say_u64(rt: *mut Runtime, value: i64) -> i64 {
    maybe_collect(rt);
    keep_text((value as u64).to_string())
}

/// Called by compiled code. Not called by anything else.
extern "C" fn say_bool(rt: *mut Runtime, value: i64) -> i64 {
    maybe_collect(rt);
    keep_text(if value != 0 { "true" } else { "false" }.to_string())
}

/// Called by compiled code. Not called by anything else.
extern "C" fn say_float(rt: *mut Runtime, bits: i64, width: i64) -> i64 {
    maybe_collect(rt);
    let shown = match width {
        64 => quench_num::show_f64(f64::from_bits(bits as u64)),
        _ => quench_num::show_f32(f32::from_bits(bits as u32)),
    };
    keep_text(shown)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn say_exact(rt: *mut Runtime, value: i64) -> i64 {
    maybe_collect(rt);
    let shown = HEAP.with(|h| h.borrow().exactly(value).to_string());
    keep_text(shown)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn say_decimal(rt: *mut Runtime, value: i64) -> i64 {
    maybe_collect(rt);
    let shown = HEAP.with(|h| h.borrow().decimally(value).to_string());
    keep_text(shown)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn say_array(rt: *mut Runtime, handle: i64, kind: i64, depth: i64) -> i64 {
    let elements = qir::Elements::from_code(kind).expect("the lowering wrote this");
    let written = shown(rt, handle, elements, depth);
    maybe_collect(rt);
    keep_text(written)
}

/// The maths functions, in the order the lowering writes their numbers. Shared with the
/// interpreter by both reading `quench_num`, which is what makes them one implementation
/// rather than two that have to be kept level.
const ALONE: [quench_num::Alone; 6] = [
    quench_num::Alone::Sqrt,
    quench_num::Alone::Abs,
    quench_num::Alone::Floor,
    quench_num::Alone::Ceiling,
    quench_num::Alone::Round,
    quench_num::Alone::Truncate,
];

const PAIRED: [quench_num::Paired; 4] = [
    quench_num::Paired::CopySign,
    quench_num::Paired::Minimum,
    quench_num::Paired::Maximum,
    quench_num::Paired::Remainder,
];

/// Called by compiled code. Not called by anything else.
extern "C" fn float_alone(_rt: *mut Runtime, bits: i64, which: i64, width: i64) -> i64 {
    let op = ALONE[which as usize];
    match width {
        64 => quench_num::maths::alone64(op, f64::from_bits(bits as u64)).to_bits() as i64,
        _ => i64::from(
            quench_num::maths::alone32(op, f32::from_bits(bits as u32)).to_bits(),
        ),
    }
}

/// Called by compiled code. Not called by anything else.
extern "C" fn float_paired(_rt: *mut Runtime, a: i64, b: i64, which: i64, width: i64) -> i64 {
    let op = PAIRED[which as usize];
    match width {
        64 => quench_num::maths::paired64(op, f64::from_bits(a as u64), f64::from_bits(b as u64))
            .to_bits() as i64,
        _ => i64::from(
            quench_num::maths::paired32(op, f32::from_bits(a as u32), f32::from_bits(b as u32))
                .to_bits(),
        ),
    }
}

/// Called by compiled code. Not called by anything else.
extern "C" fn float_fused(_rt: *mut Runtime, a: i64, b: i64, c: i64, width: i64) -> i64 {
    match width {
        64 => quench_num::maths::fused64(
            f64::from_bits(a as u64),
            f64::from_bits(b as u64),
            f64::from_bits(c as u64),
        )
        .to_bits() as i64,
        _ => i64::from(quench_num::maths::fused32(
            f32::from_bits(a as u32),
            f32::from_bits(b as u32),
            f32::from_bits(c as u32),
        )
        .to_bits()),
    }
}

/// Called by compiled code. Not called by anything else.
extern "C" fn float_slow(_rt: *mut Runtime, bits: i64, which: i64) -> i64 {
    let x = f64::from_bits(bits as u64);
    let answer = match which {
        0 => quench_num::transcend::exp(x),
        1 => quench_num::transcend::ln(x),
        2 => quench_num::transcend::sin(x),
        3 => quench_num::transcend::cos(x),
        4 => quench_num::transcend::tan(x),
        5 => quench_num::transcend::atan(x),
        6 => quench_num::transcend::asin(x),
        7 => quench_num::transcend::acos(x),
        8 => quench_num::transcend::sinh(x),
        9 => quench_num::transcend::cosh(x),
        10 => quench_num::transcend::tanh(x),
        11 => quench_num::transcend::asinh(x),
        12 => quench_num::transcend::acosh(x),
        13 => quench_num::transcend::atanh(x),
        _ => quench_num::transcend::cbrt(x),
    };
    answer.to_bits() as i64
}

/// Called by compiled code. Not called by anything else.
extern "C" fn float_power(_rt: *mut Runtime, a: i64, b: i64, which: i64) -> i64 {
    let (a, b) = (f64::from_bits(a as u64), f64::from_bits(b as u64));
    let answer = match which {
        0 => quench_num::transcend::pow(a, b),
        1 => quench_num::transcend::atan2(a, b),
        _ => quench_num::transcend::hypot(a, b),
    };
    answer.to_bits() as i64
}

/// Called by compiled code. Not called by anything else.
extern "C" fn text_clusters(_rt: *mut Runtime, at: i64) -> i64 {
    HEAP.with(|h| quench_text::grapheme::count(h.borrow().said(at)) as i64)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn text_letters(_rt: *mut Runtime, at: i64) -> i64 {
    HEAP.with(|h| h.borrow().said(at).chars().count() as i64)
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
    maybe_collect(rt);
    let answer = HEAP.with(|h| {
        let h = h.borrow();
        h.exactly(a).power(h.exactly(b))
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
extern "C" fn text_join(rt: *mut Runtime, a: i64, b: i64) -> i64 {
    maybe_collect(rt);
    let joined = HEAP.with(|h| {
        let h = h.borrow();
        format!("{}{}", h.said(a), h.said(b))
    });
    HEAP.with(|h| h.borrow_mut().text(joined))
}

/// Called by compiled code. Not called by anything else.
extern "C" fn text_compare(_rt: *mut Runtime, a: i64, b: i64) -> i64 {
    match text_of(a).cmp(&text_of(b)) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Called by compiled code. Not called by anything else.
extern "C" fn exact_compare(_rt: *mut Runtime, a: i64, b: i64) -> i64 {
    HEAP.with(|h| {
        let h = h.borrow();
        match h.exactly(a).cmp(h.exactly(b)) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        }
    })
}

/// Called by compiled code. Not called by anything else.
extern "C" fn print_float(_rt: *mut Runtime, stream: i64, bits: i64, width: i64) -> i64 {
    let shown = match width {
        64 => quench_num::show_f64(f64::from_bits(bits as u64)),
        _ => quench_num::show_f32(f32::from_bits(bits as u32)),
    };
    write_out(stream, shown.as_bytes());
    0
}

/// Called by compiled code. Not called by anything else.
extern "C" fn to_b16(_rt: *mut Runtime, bits: i64) -> i64 {
    i64::from(quench_num::to_b16(f32::from_bits(bits as u32)).to_bits())
}

/// Called by compiled code. Not called by anything else.
extern "C" fn print_exact(_rt: *mut Runtime, stream: i64, value: i64) -> i64 {
    let shown = HEAP.with(|h| h.borrow().exactly(value).to_string());
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
extern "C" fn array_new(rt: *mut Runtime, len: i64, holds: i64, depth: i64) -> i64 {
    maybe_collect(rt);
    let holds = qir::Elements::from_code(holds).expect("the lowering wrote this constant");
    keep_array(holds, depth, vec![0; len.max(0) as usize])
}

/// Called by compiled code. Not called by anything else.
extern "C" fn array_set(rt: *mut Runtime, handle: i64, at: i64, value: i64) -> i64 {
    HEAP.with(|heap| {
        let mut heap = heap.borrow_mut();
        let array = &mut heap.at_mut(handle).values;
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
        match at.checked_sub(1).and_then(|i| usize::try_from(i).ok()).and_then(|i| heap.at(handle).values.get(i))
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
    HEAP.with(|heap| heap.borrow().at(handle).values.len() as i64)
}

/// Write a handle to the slot that stands for it, where there is one.
///
/// One store, at the point the value is made. What it buys is that a collection can
/// happen anywhere without a stack to walk: whatever compiled code is holding, it has
/// already said so.
fn root_it(
    b: &mut FunctionBuilder<'_>,
    frame: ClifValue,
    slot_of: &[Option<i32>],
    func: &qir::Function,
    value: qir::Value,
    made: ClifValue,
) {
    let Some(slot) = slot_of[value.0 as usize] else { return };
    let space = match func.ty_of(value) {
        qir::Ty::Handle => 1i64,
        qir::Ty::Text => 2,
        qir::Ty::Exact => 3,
        qir::Ty::Decimal => 4,
        _ => return,
    };
    // Which space it is in rides in the top byte, because a root in compiled code is an
    // `i64` in an array and nothing else there says what it points at.
    let packed = b.ins().bor_imm_s(made, space << quench_heap::SPACE_SHIFT);
    b.ins().store(MemFlagsData::trusted(), packed, frame, slot * 8);
}

/// Called by compiled code. Not called by anything else.
extern "C" fn array_push(_rt: *mut Runtime, handle: i64, value: i64) -> i64 {
    HEAP.with(|heap| heap.borrow_mut().at_mut(handle).values.push(value));
    0
}

/// Called by compiled code. Not called by anything else.
extern "C" fn array_copy(rt: *mut Runtime, handle: i64) -> i64 {
    maybe_collect(rt);
    let (holds, depth, values) = HEAP.with(|heap| {
        let heap = heap.borrow();
        let of = heap.at(handle);
        (of.holds, of.depth, of.values.clone())
    });
    keep_array(holds, depth, values)
}

/// Called by compiled code. Not called by anything else.
extern "C" fn array_equal(rt: *mut Runtime, a: i64, b: i64, kind: i64, depth: i64) -> i64 {
    let kind = qir::Elements::from_code(kind).expect("the lowering wrote this constant");
    i64::from(alike(rt, a, b, kind, depth))
}

/// Whether two arrays hold the same things, following handles as far down as they go.
fn alike(rt: *mut Runtime, a: i64, b: i64, kind: qir::Elements, depth: i64) -> bool {
    let _ = rt;
    let (left, right) = HEAP.with(|heap| {
        let heap = heap.borrow();
        (heap.at(a).values.clone(), heap.at(b).values.clone())
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
                HEAP.with(|h| h.borrow().exactly(*x) == h.borrow().exactly(*y))
            }
            // By value: `2.50` and `2.5` are one number written two ways, and a
            // not-a-number is equal to nothing including itself.
            qir::Elements::Decimal => HEAP.with(|h| {
                let h = h.borrow();
                h.decimally(*x).compare(h.decimally(*y)) == Some(std::cmp::Ordering::Equal)
            }),
            qir::Elements::Float => f64::from_bits(*x as u64) == f64::from_bits(*y as u64),
            qir::Elements::Text => text_of(*x) == text_of(*y),
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
    let _ = rt;
    let elements = HEAP.with(|heap| heap.borrow().at(handle).values.clone());
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
                    format!("*{}*", text_of(*value))
                }
                qir::Elements::Exact => HEAP.with(|h| h.borrow().exactly(*value).to_string()),
                qir::Elements::Decimal => {
                    HEAP.with(|h| h.borrow().decimally(*value).to_string())
                }
                qir::Elements::Float => quench_num::show_f64(f64::from_bits(*value as u64)),
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
extern "C" fn print_u64(_rt: *mut Runtime, stream: i64, value: i64) -> i64 {
    write_out(stream, (value as u64).to_string().as_bytes());
    0
}

/// Called by compiled code. Not called by anything else.
extern "C" fn print_text(_rt: *mut Runtime, stream: i64, index: i64) -> i64 {
    write_out(stream, text_of(index).as_bytes());
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
    builder.symbol("quench_print_u64", print_u64 as *const u8);
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
    builder.symbol("quench_decimal_read", decimal_read as *const u8);
    builder.symbol("quench_decimal_add", decimal_add as *const u8);
    builder.symbol("quench_decimal_sub", decimal_sub as *const u8);
    builder.symbol("quench_decimal_mul", decimal_mul as *const u8);
    builder.symbol("quench_decimal_div", decimal_div as *const u8);
    builder.symbol("quench_decimal_compare", decimal_compare as *const u8);
    builder.symbol("quench_print_decimal", print_decimal as *const u8);
    builder.symbol("quench_say_i64", say_i64 as *const u8);
    builder.symbol("quench_say_u64", say_u64 as *const u8);
    builder.symbol("quench_say_bool", say_bool as *const u8);
    builder.symbol("quench_say_float", say_float as *const u8);
    builder.symbol("quench_say_exact", say_exact as *const u8);
    builder.symbol("quench_say_decimal", say_decimal as *const u8);
    builder.symbol("quench_say_array", say_array as *const u8);
    builder.symbol("quench_float_alone", float_alone as *const u8);
    builder.symbol("quench_float_paired", float_paired as *const u8);
    builder.symbol("quench_float_fused", float_fused as *const u8);
    builder.symbol("quench_float_slow", float_slow as *const u8);
    builder.symbol("quench_float_power", float_power as *const u8);
    builder.symbol("quench_text_clusters", text_clusters as *const u8);
    builder.symbol("quench_text_letters", text_letters as *const u8);
    builder.symbol("quench_print_float", print_float as *const u8);
    builder.symbol("quench_to_b16", to_b16 as *const u8);
    builder.symbol("quench_text_compare", text_compare as *const u8);
    builder.symbol("quench_text_join", text_join as *const u8);
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
    let mut roots: Box<[i64]> = vec![0; ROOM].into_boxed_slice();
    let runtime = Box::new(Runtime {
        pieces: pieces.as_ptr(),
        roots: roots.as_mut_ptr(),
        used: 0,
        stopped: 0,
    });
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
        (qir::Host::PrintU64, "quench_print_u64"),
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
        (qir::Host::DecimalRead, "quench_decimal_read"),
        (qir::Host::DecimalAdd, "quench_decimal_add"),
        (qir::Host::DecimalSub, "quench_decimal_sub"),
        (qir::Host::DecimalMul, "quench_decimal_mul"),
        (qir::Host::DecimalDiv, "quench_decimal_div"),
        (qir::Host::DecimalCompare, "quench_decimal_compare"),
        (qir::Host::PrintDecimal, "quench_print_decimal"),
        (qir::Host::SayI64, "quench_say_i64"),
        (qir::Host::SayU64, "quench_say_u64"),
        (qir::Host::SayBool, "quench_say_bool"),
        (qir::Host::SayFloat, "quench_say_float"),
        (qir::Host::SayExact, "quench_say_exact"),
        (qir::Host::SayDecimal, "quench_say_decimal"),
        (qir::Host::SayArray, "quench_say_array"),
        (qir::Host::FloatAlone, "quench_float_alone"),
        (qir::Host::FloatPaired, "quench_float_paired"),
        (qir::Host::FloatFused, "quench_float_fused"),
        (qir::Host::FloatSlow, "quench_float_slow"),
        (qir::Host::FloatPower, "quench_float_power"),
        (qir::Host::TextClusters, "quench_text_clusters"),
        (qir::Host::TextLetters, "quench_text_letters"),
        (qir::Host::PrintFloat, "quench_print_float"),
        (qir::Host::ToB16, "quench_to_b16"),
        (qir::Host::TextCompare, "quench_text_compare"),
        (qir::Host::TextJoin, "quench_text_join"),
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
        jit.define_function(declared[i].0, &mut ctx)
            .map_err(|e| Error::Backend(format!("{e:?}")))?;
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
        _roots: roots,
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
    // A block in front of the entry, holding the function's parameters, so that the
    // frame can be taken before anything in the body runs. The entry block cannot do it
    // itself: it is where the parameters arrive, and they arrive before any instruction.
    let prologue = b.create_block();
    b.append_block_params_for_function_params(prologue);
    for (i, block) in func.blocks.iter().enumerate() {
        for param in &block.params {
            b.append_block_param(blocks[i], clif_ty(func.ty_of(*param)));
        }
    }

    let mut vals: Vec<Option<ClifValue>> = vec![None; func.value_tys.len()];

    // One shadow slot per reference-typed value, so that wherever compiled code is when
    // a collection happens, every handle it holds has been written somewhere the runtime
    // can read. Filled where the value is defined and never cleared, so a slot may hold
    // a handle whose value is dead — that keeps something alive a little longer than it
    // has to and cannot free something too early, which is the direction to be wrong in.
    let mut slot_of: Vec<Option<i32>> = vec![None; func.value_tys.len()];
    let mut slots = 0i32;
    for (n, ty) in func.value_tys.iter().enumerate() {
        if matches!(ty, qir::Ty::Handle | qir::Ty::Text | qir::Ty::Exact | qir::Ty::Decimal) {
            slot_of[n] = Some(slots);
            slots += 1;
        }
    }

    // The frame: take the slots on the way in, and put them back on every way out.
    b.switch_to_block(prologue);
    let table_at = b.ins().iconst(types::I64, table);
    let base = b.ins().load(types::I64, MemFlagsData::trusted(), table_at, USED_AT);
    let roots = b.ins().load(types::I64, MemFlagsData::trusted(), table_at, ROOTS_AT);
    let scaled = b.ins().imul_imm_s(base, 8);
    let frame = b.ins().iadd(roots, scaled);
    let carried: Vec<BlockArg> =
        b.block_params(prologue).iter().map(|v| (*v).into()).collect();
    if slots > 0 {
        let taken = b.ins().iadd_imm_s(base, i64::from(slots));
        b.ins().store(MemFlagsData::trusted(), taken, table_at, USED_AT);
        // Nothing stale: a slot not yet written must not look like a handle to
        // something that has since been reused.
        let zero = b.ins().iconst(types::I64, 0);
        for k in 0..slots {
            b.ins().store(MemFlagsData::trusted(), zero, frame, k * 8);
        }
        // Deeper than there is room for is the same answer as deeper than an engine
        // will follow, and it is the same trap.
        let room = b.ins().iconst(types::I64, ROOM as i64);
        let fits = b.ins().icmp(IntCC::UnsignedLessThan, taken, room);
        let carry_on = b.create_block();
        let too_deep = b.create_block();
        b.ins().brif(fits, carry_on, &[], too_deep, &[]);
        b.switch_to_block(too_deep);
        let code = b.ins().iconst(types::I64, qir::Trap::TooDeep as i64);
        b.ins().store(MemFlagsData::trusted(), code, table_at, STOPPED_AT);
        b.ins().jump(stopping, &[]);
        b.switch_to_block(carry_on);
    }
    b.ins().jump(blocks[0], &carried);

    for (i, block) in func.blocks.iter().enumerate() {
        b.switch_to_block(blocks[i]);

        let incoming = b.block_params(blocks[i]).to_vec();
        for (n, param) in block.params.iter().enumerate() {
            vals[param.0 as usize] = Some(incoming[n]);
            root_it(&mut b, frame, &slot_of, func, *param, incoming[n]);
        }

        for (result, inst) in &block.insts {
            let v = match inst {
                qir::Inst::ConstI64(n) => b.ins().iconst(types::I64, *n),
                qir::Inst::ConstBool(t) => b.ins().iconst(types::I8, i64::from(*t)),
                qir::Inst::ConstText(at) | qir::Inst::ConstHandle(at) => {
                    b.ins().iconst(types::I64, i64::from(*at))
                }
                qir::Inst::ConstFloat(bits) => {
                    if func.ty_of(*result) == qir::Ty::F64 {
                        b.ins().f64const(f64::from_bits(*bits))
                    } else {
                        b.ins().f32const(f32::from_bits(*bits as u32))
                    }
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
                        given.push(match func.ty_of(*arg) {
                            qir::Ty::Bool => b.ins().uextend(types::I64, value),
                            // A `b64` lives in a float register and the runtime takes
                            // an integer, so its bits move across rather than its value.
                            // The bits, not the value — the runtime takes an integer
                            // and a `b64` lives in a float register. Native order,
                            // because both ends of this call are the same machine.
                            // A float's bits move into an integer register, and the
                            // narrow ones are widened after the cast rather than during
                            // it: a bitcast is a reinterpretation and not a resize.
                            ty @ (qir::Ty::F64 | qir::Ty::F32 | qir::Ty::F16) => {
                                let order =
                                    MemFlagsData::new().with_endianness(native_order());
                                if ty == qir::Ty::F64 {
                                    b.ins().bitcast(types::I64, order, value)
                                } else {
                                    let narrow = b.ins().bitcast(types::I32, order, value);
                                    b.ins().uextend(types::I64, narrow)
                                }
                            }
                            _ => value,
                        });
                    }
                    let call = b.ins().call(which, &given);
                    let mut answer = b.inst_results(call)[0];
                    // And narrowed on the way back, for the same reason: the runtime
                    // answers in an i64 and a bool lives in an i8 here.
                    match func.ty_of(*result) {
                        qir::Ty::Bool => answer = b.ins().ireduce(types::I8, answer),
                        // A float comes back in an integer register too, and goes back
                        // into a float one the same way it left.
                        ty @ (qir::Ty::F64 | qir::Ty::F32 | qir::Ty::F16) => {
                            let narrow = if ty == qir::Ty::F64 {
                                answer
                            } else {
                                b.ins().ireduce(types::I32, answer)
                            };
                            answer = b.ins().bitcast(
                                clif_ty(ty),
                                MemFlagsData::new().with_endianness(native_order()),
                                narrow,
                            );
                        }
                        _ => {}
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
                        // Plain IEEE. Cranelift fuses nothing on its own and is asked
                        // for nothing here, which is the whole of what keeps three
                        // engines to the same bits.
                        qir::BinOp::FAdd => b.ins().fadd(l, r),
                        qir::BinOp::FSub => b.ins().fsub(l, r),
                        qir::BinOp::FMul => b.ins().fmul(l, r),
                        qir::BinOp::FDiv => b.ins().fdiv(l, r),
                        qir::BinOp::FAddChecked
                        | qir::BinOp::FSubChecked
                        | qir::BinOp::FMulChecked
                        | qir::BinOp::FDivChecked => {
                            let answer = match op {
                                qir::BinOp::FAddChecked => b.ins().fadd(l, r),
                                qir::BinOp::FSubChecked => b.ins().fsub(l, r),
                                qir::BinOp::FMulChecked => b.ins().fmul(l, r),
                                _ => b.ins().fdiv(l, r),
                            };
                            // Finite or it stops: an infinity and a not-a-number both
                            // fail a comparison against themselves after subtraction,
                            // but the plain way is to ask whether it is finite.
                            let fits = b.create_block();
                            let does_not = b.create_block();
                            // The comparison has to be in the answer's own width; an
                            // `f32` against an `f64` infinity is not a wrong answer,
                            // it is not a program.
                            let big = if func.ty_of(*result) == qir::Ty::F64 {
                                b.ins().f64const(f64::INFINITY)
                            } else {
                                b.ins().f32const(f32::INFINITY)
                            };
                            let magnitude = b.ins().fabs(answer);
                            let finite = b.ins().fcmp(FloatCC::LessThan, magnitude, big);
                            b.ins().brif(finite, fits, &[], does_not, &[]);
                            b.switch_to_block(does_not);
                            let code = b.ins().iconst(types::I64, qir::Trap::NoNumber as i64);
                            let at = b.ins().iconst(types::I64, table);
                            b.ins().store(MemFlagsData::trusted(), code, at, STOPPED_AT);
                            b.ins().jump(stopping, &[]);
                            b.switch_to_block(fits);
                            answer
                        }
                        qir::BinOp::And => b.ins().band(l, r),
                        qir::BinOp::Or => b.ins().bor(l, r),
                        qir::BinOp::Add => b.ins().iadd(l, r),
                        qir::BinOp::Sub => b.ins().isub(l, r),
                        qir::BinOp::Mul => b.ins().imul(l, r),
                        qir::BinOp::AddTrapping
                        | qir::BinOp::SubTrapping
                        | qir::BinOp::MulTrapping
                        | qir::BinOp::AddTrappingU
                        | qir::BinOp::SubTrappingU
                        | qir::BinOp::MulTrappingU => {
                            trapping(&mut b, table, stopping, *op, l, r)
                        }
                        // Unsigned division faults on nought and on nothing else, so
                        // the guard is the smaller half of the signed one.
                        qir::BinOp::DivU | qir::BinOp::RemU => {
                            let zero = b.ins().iconst(types::I64, 0);
                            let by_zero = b.create_block();
                            let not_zero = b.create_block();
                            let is_zero = b.ins().icmp(IntCC::Equal, r, zero);
                            b.ins().brif(is_zero, by_zero, &[], not_zero, &[]);
                            b.switch_to_block(by_zero);
                            let code =
                                b.ins().iconst(types::I64, qir::Trap::DividedByZero as i64);
                            let at = b.ins().iconst(types::I64, table);
                            b.ins().store(MemFlagsData::trusted(), code, at, STOPPED_AT);
                            b.ins().jump(stopping, &[]);
                            b.switch_to_block(not_zero);
                            if *op == qir::BinOp::DivU {
                                b.ins().udiv(l, r)
                            } else {
                                b.ins().urem(l, r)
                            }
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
                // Signed types are sign-extended and unsigned ones zero-extended, so
                // whatever is in a register orders and prints the same however it got
                // there. Two shifts, or a branch when it has to stop instead.
                qir::Inst::Narrow { of, bits, signed, checked } => {
                    let value = vals[of.0 as usize].unwrap();
                    if *bits >= 64 {
                        value
                    } else {
                        let spare = i64::from(64 - u32::from(*bits));
                        let up = b.ins().ishl_imm_s(value, spare);
                        let put = if *signed {
                            b.ins().sshr_imm_s(up, spare)
                        } else {
                            b.ins().ushr_imm_s(up, spare)
                        };
                        if *checked {
                            let same = b.ins().icmp(IntCC::Equal, put, value);
                            let fits = b.create_block();
                            let does_not = b.create_block();
                            b.ins().brif(same, fits, &[], does_not, &[]);
                            b.switch_to_block(does_not);
                            let code =
                                b.ins().iconst(types::I64, qir::Trap::Overflowed as i64);
                            let at = b.ins().iconst(types::I64, table);
                            b.ins().store(MemFlagsData::trusted(), code, at, STOPPED_AT);
                            b.ins().jump(stopping, &[]);
                            b.switch_to_block(fits);
                        }
                        put
                    }
                }
                qir::Inst::CmpU { op, lhs, rhs } => {
                    let (l, r) = (vals[lhs.0 as usize].unwrap(), vals[rhs.0 as usize].unwrap());
                    let how = match op {
                        qir::CmpOp::Eq => IntCC::Equal,
                        qir::CmpOp::Ne => IntCC::NotEqual,
                        qir::CmpOp::Lt => IntCC::UnsignedLessThan,
                        qir::CmpOp::Le => IntCC::UnsignedLessThanOrEqual,
                        qir::CmpOp::Gt => IntCC::UnsignedGreaterThan,
                        qir::CmpOp::Ge => IntCC::UnsignedGreaterThanOrEqual,
                    };
                    b.ins().icmp(how, l, r)
                }
                qir::Inst::FCmp { op, lhs, rhs } => {
                    let (l, r) = (vals[lhs.0 as usize].unwrap(), vals[rhs.0 as usize].unwrap());
                    let how = match op {
                        qir::CmpOp::Eq => FloatCC::Equal,
                        qir::CmpOp::Ne => FloatCC::NotEqual,
                        qir::CmpOp::Lt => FloatCC::LessThan,
                        qir::CmpOp::Le => FloatCC::LessThanOrEqual,
                        qir::CmpOp::Gt => FloatCC::GreaterThan,
                        qir::CmpOp::Ge => FloatCC::GreaterThanOrEqual,
                    };
                    b.ins().fcmp(how, l, r)
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
            root_it(&mut b, frame, &slot_of, func, *result, v);
        }

        match &block.term {
            qir::Term::Ret(v) => {
                let v = vals[v.0 as usize].unwrap();
                // The frame goes back the way it came, on every way out.
                if slots > 0 {
                    b.ins().store(MemFlagsData::trusted(), base, table_at, USED_AT);
                }
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
    // an answer and nothing reads it -- but it has to be *of the type promised*, and a
    // float cannot be made with `iconst`: Cranelift's verifier refuses that outright
    // rather than quietly building the wrong thing.
    b.switch_to_block(stopping);
    let nothing = match clif_ty(func.ret) {
        types::F64 => b.ins().f64const(0.0),
        types::F32 => b.ins().f32const(0.0),
        ty => b.ins().iconst(ty, 0),
    };
    b.ins().return_(&[nothing]);

    b.seal_all_blocks();
    b.finalize(target);
}
