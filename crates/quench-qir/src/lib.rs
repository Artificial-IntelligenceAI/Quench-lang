//! QIR — the typed IR that every way of running Quench compiles from.
//!
//! There are three execution methods and they are required to agree. The cheapest way to
//! make that likely, rather than merely hoped for, is to give them nothing to disagree
//! about: one IR, fully typed, fully explicit, with every decision already taken by the
//! time a backend sees it. A backend's job is to lower QIR. It is never to work out what
//! the frontend meant.
//!
//! So QIR is deliberately dull:
//!
//! - **Every value has one type**, written down, and no instruction converts between
//!   types on its own. If a conversion is meant to happen, an instruction says so.
//! - **Every value is assigned exactly once**, and block parameters carry values between
//!   blocks. There are no variables and no phi nodes to reconstruct.
//! - **Control flow is explicit.** A block ends in a terminator, always, and the
//!   terminator names its successors and the arguments it passes them.
//!
//! # What is not here yet
//!
//! This is the seed. It holds `i64` and `bool` because that is what the Dev JIT needed to
//! prove it could compile and run something, and it will grow as the language does. Two
//! absences are deliberate rather than accidental:
//!
//! - **Ownership** is not represented, because there is nothing to own yet. When it
//!   arrives, moves, borrows and drops become *instructions* — resolved by the frontend
//!   and written down — not properties for a backend to infer.
//! - **Serialisation** is not written, because the only consumer so far is in the same
//!   process. It arrives with the C++ side, which is the point at which a format has a
//!   second reader and can therefore be designed against a real requirement rather than
//!   an imagined one. [`VERSION`] exists from the start so that there is never a QIR
//!   without one.

/// The version of the IR itself.
///
/// The C++ backends will refuse a module whose version they do not know, rather than
/// guess at it. Bump this whenever the meaning of anything below changes.
pub const VERSION: u32 = 0;

/// The name of the function a program starts at.
///
/// `start`, because that is what it does. `main` is a convention rather than a
/// description — it says a function is important without saying why, and a reader who
/// has not met it before cannot work out what it means from the word. Quench does not
/// have C's split between `_start` (the real entry, which sets a runtime up) and `main`
/// (the one you write), so there is no second thing the name has to stay clear of.
///
/// A backend never uses this: it takes [`Module::entry`], which is an id, because that
/// is what a call needs. The name is how the *frontend* finds the function in the first
/// place.
pub const ENTRY: &str = "start";

/// The type of a value. Every value has exactly one, and it never changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    I64,
    Bool,
}

impl Ty {
    pub fn name(self) -> &'static str {
        match self {
            Ty::I64 => "i64",
            Ty::Bool => "bool",
        }
    }
}

/// A value, assigned exactly once. The index is into [`Function::value_tys`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Value(pub u32);

/// A block, by index into [`Function::blocks`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BlockId(pub u32);

/// A function, by index into [`Module::functions`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FuncId(pub u32);

/// Arithmetic. Both operands and the result are `i64`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    /// Truncating division. Division by zero is a trap, not a value.
    Div,
    /// Remainder, with the sign of the dividend. By zero it traps, as `Div` does.
    Rem,
}

/// Comparison. Both operands are `i64`; the result is `bool`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

/// An instruction. Each one produces exactly one value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Inst {
    ConstI64(i64),
    ConstBool(bool),
    Bin { op: BinOp, lhs: Value, rhs: Value },
    Cmp { op: CmpOp, lhs: Value, rhs: Value },
    /// Boolean negation.
    Not(Value),
    Call { func: FuncId, args: Vec<Value> },
}

/// Where a block goes when it is done. Every block has exactly one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Term {
    Ret(Value),
    Jump { to: BlockId, args: Vec<Value> },
    BrIf { cond: Value, then: Target, otherwise: Target },
}

/// A block and the arguments passed to it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Target {
    pub block: BlockId,
    pub args: Vec<Value>,
}

impl Target {
    pub fn new(block: BlockId, args: Vec<Value>) -> Self {
        Self { block, args }
    }
}

/// A basic block: parameters in, instructions, then exactly one terminator.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Block {
    /// Values bound on entry. The entry block's parameters are the function's parameters.
    pub params: Vec<Value>,
    /// Each instruction and the value it defines, in order.
    pub insts: Vec<(Value, Inst)>,
    pub term: Term,
}

/// A function. Values are numbered across the whole function, not per block.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Function {
    pub name: String,
    pub params: Vec<Ty>,
    pub ret: Ty,
    pub blocks: Vec<Block>,
    /// The type of every value in the function, indexed by [`Value`].
    pub value_tys: Vec<Ty>,
}

impl Function {
    /// The block a call starts in. Always the first.
    pub fn entry(&self) -> BlockId {
        BlockId(0)
    }

    pub fn block(&self, id: BlockId) -> &Block {
        &self.blocks[id.0 as usize]
    }

    pub fn ty_of(&self, value: Value) -> Ty {
        self.value_tys[value.0 as usize]
    }
}

/// A whole program.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Module {
    pub functions: Vec<Function>,
    /// The function a run of this module starts at.
    pub entry: Option<FuncId>,
}

impl Module {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, function: Function) -> FuncId {
        self.functions.push(function);
        FuncId(self.functions.len() as u32 - 1)
    }

    /// The id the next function added will be given, for a body that has to name itself.
    pub fn next_id(&self) -> FuncId {
        FuncId(self.functions.len() as u32)
    }

    /// Find a function by name.
    pub fn find(&self, name: &str) -> Option<FuncId> {
        self.functions.iter().position(|f| f.name == name).map(|i| FuncId(i as u32))
    }

    /// Point the module at the function called [`ENTRY`], if it has one.
    ///
    /// This is what a frontend calls once it has compiled a whole program: the entry is
    /// not marked by anything, it is simply the function with that name.
    pub fn set_entry_to_start(&mut self) -> Option<FuncId> {
        let id = self.find(ENTRY)?;
        self.entry = Some(id);
        Some(id)
    }

    pub fn func(&self, id: FuncId) -> &Function {
        &self.functions[id.0 as usize]
    }

    /// Name the function a run starts at. See [`Module::entry`].
    pub fn set_entry(&mut self, id: FuncId) {
        self.entry = Some(id);
    }
}

pub mod builder;
pub mod verify;

pub use builder::Builder;
pub use verify::{verify, Invalid};
