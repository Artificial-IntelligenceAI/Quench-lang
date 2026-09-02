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
use cranelift_codegen::ir::{types, AbiParam, BlockArg, FuncRef, InstBuilder};
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
        // Safe for the same reasons `run` is: the signature was checked at compile time
        // and the code is kept alive by `_jit` for as long as `self` exists.
        let f: extern "C" fn() -> i64 = unsafe { std::mem::transmute(*code) };
        Some(f())
    }

    /// Run the entry, keeping whatever it printed rather than letting it out.
    pub fn run_capturing(&self) -> (i64, String) {
        SINK.with(|sink| *sink.borrow_mut() = Some(Vec::new()));
        let answer = self.run();
        let written = SINK.with(|sink| sink.borrow_mut().take()).unwrap_or_default();
        (answer, String::from_utf8_lossy(&written).into_owned())
    }

    /// Run the entry and hand back what it returned.
    pub fn run(&self) -> i64 {
        // Safe because `compile` checked the entry takes nothing and returns i64, and the
        // code is kept alive by `_jit` for as long as `self` exists.
        let entry: extern "C" fn() -> i64 = unsafe { std::mem::transmute(self.entry) };
        entry()
    }
}

impl std::fmt::Debug for Compiled {
    /// Enough to tell two apart in a failing assertion. The machine code behind the
    /// pointer is not something a test wants printed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Compiled {{ entry: {:p} }}", self.entry)
    }
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

thread_local! {
    /// Where a running program's output goes, when something is collecting it.
    ///
    /// A thread-local rather than an argument because the generated code calls a plain
    /// C function and there is nowhere to put a writer. Provisional, and the reason it
    /// is tolerable is that a worker in the oracle owns its thread.
    static SINK: std::cell::RefCell<Option<Vec<u8>>> = const { std::cell::RefCell::new(None) };
}

/// Called by compiled code. Not called by anything else.
extern "C" fn print_i64(_table: *const Piece, value: i64) -> i64 {
    write_out(value.to_string().as_bytes());
    0
}

/// Called by compiled code. Not called by anything else.
extern "C" fn print_text(table: *const Piece, index: i64) -> i64 {
    // Safe because `compile` verified the index against the module's text before any of
    // this existed, and the table outlives the code that names it.
    let piece = unsafe { &*table.add(index as usize) };
    let bytes = unsafe { std::slice::from_raw_parts(piece.at, piece.len) };
    write_out(bytes);
    0
}

fn write_out(bytes: &[u8]) {
    SINK.with(|sink| match &mut *sink.borrow_mut() {
        Some(kept) => kept.extend_from_slice(bytes),
        None => {
            use std::io::Write as _;
            let _ = std::io::stdout().write_all(bytes);
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
    let mut jit = JITModule::new(builder);

    // The text, owned here and pointed at by a table whose address the code will carry.
    let text: Vec<String> = module.text.clone();
    let pieces: Box<[Piece]> = text
        .iter()
        .map(|s| Piece { at: s.as_ptr(), len: s.len() })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let table = pieces.as_ptr() as i64;

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
    // Declared once for the whole module: the runtime function every `CallHost` reaches.
    let mut host_sig = jit.make_signature();
    host_sig.params.push(AbiParam::new(types::I64)); // the table
    host_sig.params.push(AbiParam::new(types::I64)); // which piece
    host_sig.returns.push(AbiParam::new(types::I64));
    // One declaration per host function. They share a signature -- table, one argument,
    // an answer nothing uses -- so the lowering only has to pick which.
    let mut hosts = Vec::new();
    for (host, symbol) in
        [(qir::Host::PrintText, "quench_print_text"), (qir::Host::PrintI64, "quench_print_i64")]
    {
        let id = jit
            .declare_function(symbol, Linkage::Import, &host_sig)
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
    Ok(Compiled { _jit: jit, _text: text, _pieces: pieces, entry, callable })
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
                qir::Inst::ConstText(at) => b.ins().iconst(types::I64, i64::from(*at)),
                qir::Inst::CallHost { host, args } => {
                    let which = hosts
                        .iter()
                        .find(|(known, _)| known == host)
                        .map(|(_, r)| *r)
                        .expect("every host function is declared");
                    // The table's address is a constant of this compilation. QIR carried
                    // an index; the pointer is put on here and goes no further.
                    let mut given = vec![b.ins().iconst(types::I64, table)];
                    given.extend(args.iter().map(|a| vals[a.0 as usize].unwrap()));
                    let call = b.ins().call(which, &given);
                    b.inst_results(call)[0]
                }
                qir::Inst::Bin { op, lhs, rhs } => {
                    let (l, r) = (vals[lhs.0 as usize].unwrap(), vals[rhs.0 as usize].unwrap());
                    match op {
                        qir::BinOp::Add => b.ins().iadd(l, r),
                        qir::BinOp::Sub => b.ins().isub(l, r),
                        qir::BinOp::Mul => b.ins().imul(l, r),
                        // The processor's own division, which rounds toward zero.
                        qir::BinOp::DivTruncated => b.ins().sdiv(l, r),
                        qir::BinOp::RemTruncated => b.ins().srem(l, r),
                        // No processor floors, so it is built: divide, then correct by
                        // one step when the remainder disagrees in sign with the divisor,
                        // which is exactly when truncation rounded the wrong way.
                        //
                        // `sdiv` and `srem` still carry the traps, so a zero divisor and
                        // `i64::MIN / -1` stop here as they do everywhere else.
                        qir::BinOp::DivFloored => {
                            let quotient = b.ins().sdiv(l, r);
                            let rest = b.ins().srem(l, r);
                            let wrong_way = signs_disagree(&mut b, rest, r);
                            let one_less = b.ins().iadd_imm_s(quotient, -1);
                            b.ins().select(wrong_way, one_less, quotient)
                        }
                        qir::BinOp::RemFloored => {
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
                    b.inst_results(call)[0]
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

    b.seal_all_blocks();
    b.finalize(target);
}
