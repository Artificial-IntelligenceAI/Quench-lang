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
use quench_qir as qir;

/// Why a module could not be compiled.
///
/// None of these are things a Quench program can cause. A program that does not make
/// sense is stopped by the frontend, with a diagnostic; anything here is a bug in the
/// compiler, and says so plainly rather than apologising to someone who cannot fix it.
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
    entry: *const u8,
}

impl Compiled {
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

fn clif_ty(ty: qir::Ty) -> types::Type {
    match ty {
        qir::Ty::I64 => types::I64,
        // Cranelift has no one-bit type: a comparison yields an i8 that is 0 or 1.
        qir::Ty::Bool => types::I8,
    }
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

/// Compile a whole module. The IR is verified first, so lowering can assume it is sound.
pub fn compile(module: &qir::Module) -> Result<Compiled, Error> {
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
    // See the module docs: this is the reference engine, so it stays out of the way.
    flags.set("opt_level", "none").map_err(|e| Error::Backend(e.to_string()))?;
    let isa = cranelift_native::builder()
        .map_err(|e| Error::Backend(e.to_string()))?
        .finish(settings::Flags::new(flags))
        .map_err(|e| Error::Backend(e.to_string()))?;
    let mut jit = JITModule::new(JITBuilder::with_isa(isa, default_libcall_names()));

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
    let mut ctx = jit.make_context();
    let mut fctx = FunctionBuilderContext::new();
    for (i, func) in module.functions.iter().enumerate() {
        ctx.func.signature = declared[i].1.clone();
        // Taken before the builder borrows the function, since both want it mutably.
        let refs: Vec<FuncRef> =
            declared.iter().map(|(id, _)| jit.declare_func_in_func(*id, &mut ctx.func)).collect();
        lower(func, &mut ctx.func, &mut fctx, &refs, target);
        jit.define_function(declared[i].0, &mut ctx).map_err(|e| Error::Backend(e.to_string()))?;
        jit.clear_context(&mut ctx);
    }

    jit.finalize_definitions().map_err(|e| Error::Backend(e.to_string()))?;
    let entry = jit.get_finalized_function(declared[entry_id.0 as usize].0);
    Ok(Compiled { _jit: jit, entry })
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
                qir::Inst::Bin { op, lhs, rhs } => {
                    let (l, r) = (vals[lhs.0 as usize].unwrap(), vals[rhs.0 as usize].unwrap());
                    match op {
                        qir::BinOp::Add => b.ins().iadd(l, r),
                        qir::BinOp::Sub => b.ins().isub(l, r),
                        qir::BinOp::Mul => b.ins().imul(l, r),
                        qir::BinOp::Div => b.ins().sdiv(l, r),
                        qir::BinOp::Rem => b.ins().srem(l, r),
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
