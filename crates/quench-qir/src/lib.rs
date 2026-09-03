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
//! - **Collection** is not represented, because nothing allocates yet. When it arrives,
//!   which values are references the collector must know about, and where the safepoints
//!   are, become things QIR *says* — resolved by the frontend and written down — not
//!   properties for a backend to infer. Stack maps travel back the other way, normalised
//!   by both backends into one format so the collector reads a single thing.
//! - **Serialisation** is not written yet, but it is not optional and it is not late.
//!   Serialised QIR is Quench's distributable artefact — compile once, run anywhere —
//!   as well as the format the C++ backend reads, so it has two readers and always did.
//!   [`VERSION`] exists from the start so that there is never a QIR without one.
//!
//! # The constraint that follows from being portable
//!
//! Because the machine that compiles is not the machine that runs, **QIR may not know
//! what machine it is for**. No pointer width, no word size, no host-sized integer, no
//! calling convention, no struct layout, no register class, no target-specific type.
//! Those belong to whichever backend eventually runs the module, and every one of them
//! is easy to admit by accident and very hard to remove once files exist that somebody
//! else compiled.

/// A reason a program stops.
///
/// Part of the IR rather than of any engine, because *stopping in the same place for the
/// same reason* is as much a thing the engines must agree about as printing the same
/// number. An engine that invented its own list could not be compared with one that did
/// not.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(i64)]
pub enum Trap {
    /// Division or remainder by zero.
    DividedByZero = 1,
    /// `i64::MIN / -1`, whose answer is one larger than an `i64` holds.
    DivisionOverflowed = 2,
    /// An index outside the array it was given to. Counted from one, so `0` is one of
    /// these and so is one past the end.
    OutsideTheArray = 3,
    /// Calls nested deeper than an engine will follow.
    TooDeep = 4,
    /// An answer too large for the type it was going into.
    Overflowed = 5,
    /// A whole number raised to a negative power, whose answer is a fraction.
    NegativePower = 6,
    /// An exact number raised to a fraction, whose answer is generally not a ratio —
    /// the square root of two is the oldest number known not to be one.
    FractionalPower = 7,
}

impl Trap {
    /// What to call this when telling somebody.
    pub fn describe(self) -> &'static str {
        match self {
            Trap::DividedByZero => "divided by zero",
            Trap::DivisionOverflowed => "a division too large to hold",
            Trap::OutsideTheArray => "an index outside the array",
            Trap::TooDeep => "calls nested too deep",
            Trap::Overflowed => "a number too large to hold",
            Trap::NegativePower => "a whole number raised to a negative power",
            Trap::FractionalPower => "an exact number raised to a fraction",
        }
    }

    /// The number compiled code writes down, back into a reason.
    pub fn from_code(code: i64) -> Option<Trap> {
        Some(match code {
            1 => Trap::DividedByZero,
            2 => Trap::DivisionOverflowed,
            3 => Trap::OutsideTheArray,
            4 => Trap::TooDeep,
            5 => Trap::Overflowed,
            6 => Trap::NegativePower,
            7 => Trap::FractionalPower,
            _ => return None,
        })
    }
}

/// How a run ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// It finished, and this is what the entry gave back.
    Returned(i64),
    /// It stopped. Every engine must stop here too, and for this reason.
    Trapped(Trap),
}

/// The version of the IR itself.
///
/// The C++ backends will refuse a module whose version they do not know, rather than
/// guess at it. Bump this whenever the meaning of anything below changes.
pub const VERSION: u32 = 0;

/// The name of the function a program starts at.
///
/// `START`, because that is what it does. `main` is a convention rather than a
/// description — it says a function is important without saying why, and a reader who
/// has not met it before cannot work out what it means from the word. Quench does not
/// have C's split between `_start` (the real entry, which sets a runtime up) and `main`
/// (the one you write), so there is no second thing the name has to stay clear of.
///
/// A backend never uses this: it takes [`Module::entry`], which is an id, because that
/// is what a call needs. The name is how the *frontend* finds the function in the first
/// place.
pub const ENTRY: &str = "START";

/// The type of a value. Every value has exactly one, and it never changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    I64,
    Bool,
    /// A handle to something on the heap. An array, so far.
    ///
    /// A handle, not a pointer — the same reason [`Ty::Text`] is an index. What a handle
    /// *is* belongs to whichever runtime is holding the heap.
    Handle,
    /// A number held exactly, however large it grows. An `e`.
    ///
    /// A handle like [`Ty::Handle`] is, and for the same reason: what an exact number
    /// *is* belongs to whichever runtime is holding it, and no two engines may hold it
    /// differently in any way a program can see. They do not — every engine hands the
    /// arithmetic to the same code, which is why `e` cannot make them disagree.
    Exact,
    /// A piece of text the program was written with.
    ///
    /// A `Text` value is an index into [`Module::text`], not a pointer — QIR may not
    /// know what machine it is for, and a pointer is the most machine-specific thing
    /// there is. What an index *becomes* is each backend's business: the Dev JIT hands
    /// the runtime an index into a pool it can see, and ahead-of-time output will emit a
    /// data symbol. The IR says which piece of text; it does not say where it lives.
    Text,
}

impl Ty {
    pub fn name(self) -> &'static str {
        match self {
            Ty::I64 => "i64",
            Ty::Bool => "bool",
            Ty::Text => "text",
            Ty::Handle => "handle",
            Ty::Exact => "exact",
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
///
/// There are four divisions rather than one with a setting attached, because QIR says
/// what it means and a backend never works it out. Which one a Quench program gets is
/// decided by `[defaults] division` in `QNL-Config.toml`, and by the time the frontend
/// is finished that decision is written down here as an instruction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    /// Round into the type, as the processor does.
    Add,
    Sub,
    Mul,
    /// Stop rather than round. Which a program gets is `[defaults] overflow`, decided by
    /// the frontend and written down here, so no backend learns a setting existed.
    AddTrapping,
    SubTrapping,
    MulTrapping,
    /// Toward zero, and the remainder follows the dividend: `-7 / 2` is `-3`.
    DivTruncated,
    /// The remainder that goes with [`BinOp::DivTruncated`]: `-7 % 2` is `-1`.
    RemTruncated,
    /// Toward negative infinity, and the remainder follows the divisor: `-7 / 2` is `-4`.
    DivFloored,
    /// The remainder that goes with [`BinOp::DivFloored`]: `-7 % 2` is `1`.
    RemFloored,
    /// Both, and either. On `Bool`, and always asking both sides — the form
    /// `[defaults] logic = "asks-both"` lowers to. Stopping early is control flow and
    /// is built out of blocks instead, because that is what stopping early *is*.
    And,
    Or,
}

impl BinOp {
    /// Whether this stops on a zero divisor, and on `i64::MIN / -1`.
    pub fn can_trap(self) -> bool {
        !matches!(self, BinOp::Add | BinOp::Sub | BinOp::Mul)
    }
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

/// Where written output goes.
///
/// Named in the source and carried here, because a program that prints should say where
/// to — Go's built-in `println` writes to standard error and nothing about writing it
/// says so, which is the sort of surprise a language can simply not have.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(i64)]
pub enum Stream {
    Out = 1,
    Err = 2,
}

impl Stream {
    pub fn name(self) -> &'static str {
        match self {
            Stream::Out => "stdout",
            Stream::Err => "stderr",
        }
    }

    pub fn from_name(word: &str) -> Option<Stream> {
        match word {
            "stdout" => Some(Stream::Out),
            "stderr" => Some(Stream::Err),
            _ => None,
        }
    }

    /// Every one there is, for saying so when a name is not one of them.
    pub const ALL: [Stream; 2] = [Stream::Out, Stream::Err];
}

/// Something outside the program that it can ask for.
///
/// A fixed list rather than arbitrary symbol names, for the same reason QIR carries no
/// pointers: a module travels, and a name it expects to find on the other machine is a
/// promise it cannot keep. Everything here is something every Quench runtime has.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Host {
    /// `(stream, text)` — write a piece of text.
    ///
    /// The stream is an argument rather than three hosts per destination, because the
    /// list of destinations will grow and the list of things to write will not. It is
    /// always a constant by the time it gets here, so a backend that wanted to
    /// specialise on it could.
    PrintText,
    /// `(stream, number)`
    PrintI64,
    /// `(stream, bool)`
    PrintBool,

    /// Make an array of that many elements, all zero. Gives back a handle.
    ///
    /// This is where Quench first asks for memory. There is no collector behind it yet —
    /// it allocates and never frees, which is deliberately the first of the three stages
    /// in `notes/the-collector-earns-its-place.md`, and the one that needs no stack maps
    /// and no cooperation from any backend.
    ArrayNew,
    /// `(handle, index, value)` — put a value in. Counted from one. Can stop.
    ArraySet,
    /// `(handle, index)` — take one out. Counted from one. Can stop.
    ArrayGet,
    /// How many elements it has.
    ArrayLen,
    /// `(handle)` — a new array holding the same things. What `copy` costs, said out
    /// loud at the place that pays it.
    ArrayCopy,
    /// `(a, b)` — whether two arrays hold the same things, element by element. Not
    /// whether they are the same array: two names for one array is what `share` makes,
    /// and this question is the other one.
    ArrayEqual,
    /// `(stream, handle)` — every element it holds, in the order they are laid out,
    /// between brackets and separated by spaces.
    ///
    /// Flat, however many dimensions the array has, because that is how the elements
    /// are written in the first place: `[*1* *2* *3* *4* *5* *6*]` is a `(2 3)`, and
    /// nesting the output would show a shape the input deliberately does not.
    PrintArray,

    /// `(text)` — read an exact number from a piece of text the program was written
    /// with. `12`, `-3/4` and `0.1` are all exact, and the last is the point of the
    /// decimal point: one tenth, rather than the `b64` nearest to it.
    ExactRead,
    /// `(a, b)` — exactly, and reduced.
    ExactAdd,
    ExactSub,
    ExactMul,
    /// `(a, b)` — exactly. Can stop, on a divisor of zero.
    ExactDiv,
    /// `(a, b)` — `-1`, `0` or `1`. One host rather than six, because a comparison of
    /// exact numbers is a comparison of the answer against zero and always was.
    ExactCompare,
    /// `(a, b)` — `-1`, `0` or `1`, comparing what two pieces of text hold rather than
    /// which pieces they are. Interning would make the indices agree today, and would
    /// stop doing so the moment text can be built while a program runs.
    TextCompare,
    /// `(stream, exact)` — a whole number wears no denominator.
    PrintExact,
    /// `(base, exponent)` — exactly. A negative exponent is fine and gives a ratio.
    /// Can stop, on a fractional exponent or one too large to finish.
    ExactPow,

    /// `(base, exponent)` — by squaring, wrapping where it does not fit. Can stop, on a
    /// negative exponent: the answer to that is a fraction and this is a whole number.
    PowI64,
    /// The same, stopping rather than wrapping.
    PowI64Trapping,
}

impl Host {
    pub fn name(self) -> &'static str {
        match self {
            Host::PrintText => "print-text",
            Host::PrintI64 => "print-i64",
            Host::PrintBool => "print-bool",
            Host::ArrayNew => "array-new",
            Host::ArraySet => "array-set",
            Host::ArrayGet => "array-get",
            Host::ArrayLen => "array-len",
            Host::ArrayCopy => "array-copy",
            Host::ArrayEqual => "array-equal",
            Host::PrintArray => "print-array",
            Host::ExactRead => "exact-read",
            Host::ExactAdd => "exact-add",
            Host::ExactSub => "exact-sub",
            Host::ExactMul => "exact-mul",
            Host::ExactDiv => "exact-div",
            Host::ExactCompare => "exact-compare",
            Host::TextCompare => "text-compare",
            Host::PrintExact => "print-exact",
            Host::ExactPow => "exact-pow",
            Host::PowI64 => "pow-i64",
            Host::PowI64Trapping => "pow-i64-trapping",
        }
    }

    /// What it takes, in order.
    pub fn params(self) -> &'static [Ty] {
        match self {
            Host::PrintText => &[Ty::I64, Ty::Text],
            Host::PrintI64 => &[Ty::I64, Ty::I64],
            Host::PrintBool => &[Ty::I64, Ty::Bool],
            Host::ArrayNew => &[Ty::I64],
            Host::ArraySet => &[Ty::Handle, Ty::I64, Ty::I64],
            Host::ArrayGet => &[Ty::Handle, Ty::I64],
            Host::ArrayLen => &[Ty::Handle],
            Host::ArrayCopy => &[Ty::Handle],
            // The last is which [`Elements`] it holds, always a constant by the time it
            // arrives here.
            Host::ArrayEqual => &[Ty::Handle, Ty::Handle, Ty::I64],
            Host::PrintArray => &[Ty::I64, Ty::Handle, Ty::I64],
            Host::ExactRead => &[Ty::Text],
            Host::ExactAdd | Host::ExactSub | Host::ExactMul | Host::ExactDiv => {
                &[Ty::Exact, Ty::Exact]
            }
            Host::ExactCompare => &[Ty::Exact, Ty::Exact],
            Host::TextCompare => &[Ty::Text, Ty::Text],
            Host::PrintExact => &[Ty::I64, Ty::Exact],
            Host::ExactPow => &[Ty::Exact, Ty::Exact],
            Host::PowI64 | Host::PowI64Trapping => &[Ty::I64, Ty::I64],
        }
    }

    /// Which parameter is *whatever the array holds*, rather than a type fixed here.
    ///
    /// A slot is an `i64` however wide the thing in it is, so no runtime ever needs
    /// telling. The IR does, because a value coming back out of one has to have a type
    /// before anything can use it — which is why [`Host::ArrayGet`] is asked for its
    /// answer's type at the point it is called.
    pub fn takes_an_element(self) -> Option<usize> {
        match self {
            Host::ArraySet => Some(2),
            _ => None,
        }
    }

    /// Whether this can stop the program rather than answering.
    ///
    /// What it costs is a check afterwards in compiled code, so it is worth knowing
    /// which calls need one rather than guarding all of them.
    pub fn can_stop(self) -> bool {
        matches!(
            self,
            Host::ArrayGet
                | Host::ArraySet
                | Host::ExactDiv
                | Host::ExactPow
                | Host::PowI64
                | Host::PowI64Trapping
        )
    }

    /// What it gives back. Most give an `i64` nothing is expected to use.
    pub fn result(self) -> Ty {
        match self {
            Host::ArrayNew | Host::ArrayCopy => Ty::Handle,
            Host::ArrayEqual => Ty::Bool,
            Host::ExactRead
            | Host::ExactAdd
            | Host::ExactSub
            | Host::ExactMul
            | Host::ExactDiv
            | Host::ExactPow => Ty::Exact,
            _ => Ty::I64,
        }
    }
}

/// What an array's elements are, as compiled code carries it: a number an engine is
/// handed alongside the handle, because a slot is an `i64` whatever is in it.
///
/// The heap holds `i64`s. What is *in* one depends on the type — a `bool` is nought or
/// one, a `str` is which piece of text, an `e` is which exact number — and both showing
/// and comparing have to know which.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Elements {
    I64 = 0,
    Bool = 1,
    Text = 2,
    Exact = 3,
}

impl Elements {
    pub fn from_code(code: i64) -> Option<Elements> {
        Some(match code {
            0 => Elements::I64,
            1 => Elements::Bool,
            2 => Elements::Text,
            3 => Elements::Exact,
            _ => return None,
        })
    }
}

/// How an array is shown, written once so that no engine can have its own idea of it.
///
/// Flat and bracketed: `[1 2 3]`. A `(2 3)` shows six numbers rather than two rows,
/// because six numbers is how one is written. Each engine works out what its own
/// elements say; this decides only how they are put together, which is the part two
/// engines could otherwise disagree about.
pub fn show_array(shown: &[String]) -> String {
    let mut out = String::from("[");
    for (n, part) in shown.iter().enumerate() {
        if n > 0 {
            out.push(' ');
        }
        out.push_str(part);
    }
    out.push(']');
    out
}

/// An instruction. Each one produces exactly one value.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Inst {
    ConstI64(i64),
    ConstBool(bool),
    /// A piece of text, by index into [`Module::text`].
    ConstText(u32),
    Bin { op: BinOp, lhs: Value, rhs: Value },
    Cmp { op: CmpOp, lhs: Value, rhs: Value },
    /// Boolean negation.
    Not(Value),
    Call { func: FuncId, args: Vec<Value> },
    /// Ask the runtime for something.
    ///
    /// What it produces is [`Host::result`] — usually an `i64` nothing is expected to
    /// use, since every instruction here defines exactly one value and a host call
    /// having nothing to give back is not a good enough reason to make that untrue
    /// everywhere else.
    CallHost { host: Host, args: Vec<Value> },
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
    /// Every piece of text the module mentions, once each.
    ///
    /// Held here rather than inside instructions so that a module is one thing to send
    /// somewhere, and so the same text written twice is stored once.
    pub text: Vec<String>,
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

    /// Add a piece of text, or find it if the module already has it.
    pub fn intern(&mut self, text: &str) -> u32 {
        if let Some(at) = self.text.iter().position(|held| held == text) {
            return at as u32;
        }
        self.text.push(text.to_string());
        self.text.len() as u32 - 1
    }
}

pub mod builder;
pub mod verify;

pub use builder::Builder;
pub use verify::{diagnose, verify, Audience, Invalid};
