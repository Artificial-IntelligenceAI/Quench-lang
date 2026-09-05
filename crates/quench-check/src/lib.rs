//! Names resolved and types checked.
//!
//! The syntax tree says what somebody typed. This says what it *means*: which type each
//! declaration has, which declaration each use refers to, and whether every value suits
//! the type it was given to. What comes out is a second, smaller tree with no syntax
//! left in it — no marks, no chain, no escapes — which is what makes lowering afterwards
//! a transliteration rather than a second round of decisions.
//!
//! Almost every error a person will ever see is decided here. The lexer can say a mark
//! is unclosed and the parser can say a comma is missing, but "you called that `b16` and
//! gave it text" is this, and so is "you have declared that twice".

mod holes;

use quench_diag::{Diagnostic, Span};
use quench_num::{whole_range, Whole};
use quench_parse::{ast, counted, listed, Parsed};
use std::collections::HashMap;

/// A type, as far as the checker is concerned.
///
/// Quench's type list is longer than this. These are the ones that exist all the way
/// down — anything else is read, understood, and then honestly refused as not built.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ty {
    /// Every whole-number type: how many bits, and whether one of them is a sign.
    ///
    /// All of them ride in an `i64`, held normalised — sign-extended when signed and
    /// zero-extended when not — so comparing and printing need know nothing about
    /// width. Putting a value back in that shape after an operation is the whole of
    /// what makes a `u8` a `u8`, and under `overflow = "trap"` it is where one that
    /// reached 256 stops rather than becoming nought.
    Int { bits: u8, signed: bool },
    Str,
    Bool,
    /// IEEE 754 binary64, binary32 and binary16.
    F64,
    F32,
    /// Carried in a `b32` holding a value binary16 can represent, and put back in that
    /// set after every operation. See `notes/what-a-float-is-allowed-to-do.md`.
    F16,
    /// `e` — a number held exactly, however large it grows.
    ///
    /// Not a size. Every other number type in Quench says how many bits it has, and
    /// this one says instead that it never rounds and never overflows: `*1*` divided by
    /// `*3*` is a third, and a third times three is one. What that costs is that an `e`
    /// lives on the heap and a `b64` lives in a register.
    Exact,
    /// `d32` and `d64` — a decimal float, which rounds in the base it was written in.
    ///
    /// The difference from a `b64` is not accuracy, it is *which* tenth: `0.1` in a
    /// `b64` is the nearest binary fraction to a tenth and in a `d64` it is a tenth,
    /// held to sixteen digits. What it costs is the same thing an `e` costs — a
    /// coefficient and an exponent do not fit in a register — and what it buys over an
    /// `e` is that it stops growing.
    Decimal { digits: u32 },
    /// `any` or `number` — the hole a generic function left for its caller.
    ///
    /// Gone by the time anything runs. The checker fills it in with whatever the call
    /// site supplied and writes out one real function per type it was used at, so QIR,
    /// the interpreter and the Dev JIT never learn the word — which they could not, since
    /// a slot is an `i64` whatever is in it and only the type says whether the collector
    /// should follow it.
    Hole(Hole),
    /// `arr.i64 (2 3)` — one allocation, laid out row by row.
    ///
    /// One `arr` link is one allocation however many dimensions it has, which is what
    /// makes indexing arithmetic. Two `arr` links are two allocations, with handles in
    /// the outer one.
    ///
    /// `length` is what the first size said when it was not a number, and only the
    /// first may: indexing is `(i - 1) x stride + j`, and a stride is the sizes *under*
    /// a dimension. The outermost has nothing above it to be a stride for, so it is the
    /// one dimension whose size the arithmetic never needs.
    Arr { of: Box<Ty>, shape: Vec<usize>, length: Length },
}

impl Ty {
    pub fn name(&self) -> String {
        match self {
            Ty::Int { bits, signed } => {
                format!("{}{bits}", if *signed { "i" } else { "u" })
            }
            Ty::Str => "str".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Hole(hole) => hole.word().to_string(),
            Ty::Exact => "e".to_string(),
            // Named for the bits the format has, the way `b32` is, even though what
            // Quench keeps is the digits: `d32` is what IEEE calls it.
            Ty::Decimal { digits } => {
                format!("d{}", if *digits == 7 { 32 } else { 64 })
            }
            Ty::F64 => "b64".to_string(),
            Ty::F32 => "b32".to_string(),
            Ty::F16 => "b16".to_string(),
            // Named the way it was written: every `arr` link, then every size in one
            // pair of brackets, outside in. Which is also the order they were read in.
            Ty::Arr { .. } => {
                let (mut links, mut sizes) = (0, Vec::new());
                let mut walking = self;
                while let Ty::Arr { of, shape, length } = walking {
                    links += 1;
                    if let Some(word) = length.word() {
                        sizes.push(word.to_string());
                    }
                    sizes.extend(shape.iter().map(usize::to_string));
                    walking = of;
                }
                format!("{}{} ({})", "arr.".repeat(links), walking.name(), sizes.join(" "))
            }
        }
    }

    /// `a` or `an`, for the name of this type.
    ///
    /// Small, and worth having: a language whose selling point is its errors cannot
    /// write "a i64" in them.
    pub fn article(&self) -> &'static str {
        match self {
            // `an i8`, `an i64` — and `a u8`, `a u64`, because the letter is said
            // aloud and `u` starts with a consonant when it is.
            Ty::Int { signed: true, .. } | Ty::Arr { .. } | Ty::Exact => "an",
            Ty::Int { signed: false, .. } => "a",
            Ty::Hole(Hole::Any) => "an",
            Ty::Hole(Hole::Number) => "a",
            Ty::Str | Ty::Bool | Ty::F64 | Ty::F32 | Ty::F16 | Ty::Decimal { .. } => "a",
        }
    }

    /// Whether a value of `found` may be given where this is what was declared.
    ///
    /// Equality everywhere but one place. An `arr.i64 (any)` is what an `arr.i64 (3)`
    /// and an `arr.i64 (grow)` both are, because `any` claims to know nothing about the
    /// length and neither of them contradicts it. Nothing goes the other way: a slot
    /// declared `(3)` may not be handed a length nobody counted.
    pub fn accepts(&self, found: &Ty) -> bool {
        match (self, found) {
            (
                Ty::Arr { of: wanted, shape, length: Length::Unknown },
                Ty::Arr { of: brought, shape: sizes, length },
            ) => {
                // The first size is the one this says it was not told, so it is the one
                // left out of the comparison -- and it is only *in* `sizes` when the
                // other side said a number for it.
                let rest: &[usize] =
                    if length.known() { sizes.get(1..).unwrap_or(&[]) } else { sizes };
                shape == rest && wanted.accepts(brought)
            }
            (Ty::Arr { of: wanted, shape, length }, Ty::Arr { of: brought, shape: sizes, length: brought_length }) => {
                length == brought_length && shape == sizes && wanted.accepts(brought)
            }
            _ => self == found,
        }
    }

    /// Whether every allocation in this, from the top down, said how big it is.
    ///
    /// When one did not, nothing can say where one row of the thing above it ends —
    /// which is why such an array can only be written empty and filled afterwards.
    pub fn settled(&self) -> bool {
        match self {
            Ty::Arr { of, length, .. } => length.known() && of.settled(),
            _ => true,
        }
    }

    /// How many written values one of these takes, all told and all the way down.
    ///
    /// An array of arrays is written flat like any other, so this counts through every
    /// allocation rather than stopping at the top one.
    pub fn count(&self) -> usize {
        match self {
            // A growing allocation holds however many it has been given, so what one
            // *takes* when it is written is a multiple of what lies under it rather
            // than a number.
            Ty::Arr { of, shape, .. } => shape.iter().product::<usize>() * of.count(),
            _ => 1,
        }
    }

    /// Every type that is one word, which is the list `simple` below answers to.
    pub const NAMES: &[&str] = &[
        "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "b16", "b32", "b64",
        "d32", "d64", "e", "bool", "str",
    ];

    fn simple(word: &str) -> Option<Ty> {
        match word {
            "i8" => Some(Ty::Int { bits: 8, signed: true }),
            "i16" => Some(Ty::Int { bits: 16, signed: true }),
            "i32" => Some(Ty::Int { bits: 32, signed: true }),
            "i64" => Some(Ty::Int { bits: 64, signed: true }),
            "u8" => Some(Ty::Int { bits: 8, signed: false }),
            "u16" => Some(Ty::Int { bits: 16, signed: false }),
            "u32" => Some(Ty::Int { bits: 32, signed: false }),
            "u64" => Some(Ty::Int { bits: 64, signed: false }),
            "str" => Some(Ty::Str),
            "bool" => Some(Ty::Bool),
            "e" => Some(Ty::Exact),
            "b64" => Some(Ty::F64),
            "b32" => Some(Ty::F32),
            "b16" => Some(Ty::F16),
            "d32" => Some(Ty::Decimal { digits: 7 }),
            "d64" => Some(Ty::Decimal { digits: 16 }),
            _ => None,
        }
    }
}

/// Who can name a thing. Required on everything at the top of a file, because a
/// missing one would be a fourth answer given by silence.
///
/// With one file and no linking these are not yet told apart by anything that runs —
/// there is nowhere for `file` and `program` to differ. They are checked and recorded
/// now so that the answer is written down before it matters, rather than defaulted
/// into existence later.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Visibility {
    /// This module, and everything nested inside it. Not a preference: a helper
    /// declared in `maths` and called from `maths.trig` is the case modules exist for,
    /// so a child sees its ancestors. The consequence — a parent cannot see into a
    /// child — is what `Parent` is for.
    Module,
    /// The module around this one, and everything under that, siblings included.
    /// Rust's `pub(super)`. One rung up and no further.
    Parent,
    File,
    Program,
    Export,
}

impl Visibility {
    /// Every visibility there is, narrowest first, which is the list `from_word` below
    /// answers to. See `notes/five-lines-a-name-can-cross.md`.
    pub const ALL: &[&str] = &["module", "parent", "file", "program", "export"];

    fn from_word(word: &str) -> Option<Visibility> {
        match word {
            "module" => Some(Visibility::Module),
            "parent" => Some(Visibility::Parent),
            "file" => Some(Visibility::File),
            "program" => Some(Visibility::Program),
            "export" => Some(Visibility::Export),
            _ => None,
        }
    }

    /// Whether code in `from` may name something declared in `at` with this word.
    ///
    /// A module path is a list of names, outermost first, and "inside" is the prefix
    /// relation — which is the whole of the rule.
    pub fn reaches(self, from: (&str, &[String]), at: (&str, &[String])) -> bool {
        let (from_file, from) = from;
        let (at_file, at) = at;
        let same_file = from_file == at_file;
        match self {
            Visibility::Module => same_file && from.starts_with(at),
            // The module around the declaring one. A declaration at the top of a file
            // has none, and is refused where it is written rather than here.
            Visibility::Parent => match at.split_last() {
                Some((_, around)) => same_file && from.starts_with(around),
                None => false,
            },
            // Finally a real question rather than an inert one: until a program could be
            // more than one file there was nowhere for this to be false.
            Visibility::File => same_file,
            // And these two still cannot be told apart, because telling them apart needs
            // a *second program* using this one as a library, which is further off than
            // a second file was. See `notes/three-lines-a-name-can-cross.md`.
            Visibility::Program | Visibility::Export => true,
        }
    }

    pub fn word(self) -> &'static str {
        Visibility::ALL[self as usize]
    }
}

/// What the first size of an array says, when it is not a number.
///
/// The other sizes are always numbers and always in `shape`, because finding an element
/// is `(i - 1) x stride + j` and a stride is the sizes *under* a dimension — so the
/// outermost is the one whose size the arithmetic never asks for, and the only one that
/// can be left unsaid.
///
/// Two ways of not saying it, and they are not the same thing. See
/// `notes/a-hole-is-not-a-name.md`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Length {
    /// Every size is a number.
    Said,
    /// `grow` — there is no number yet, and this array may be added to.
    Grows,
    /// `any` — a length whoever wrote this was not told.
    ///
    /// It may be read, indexed and counted, and it may **not** be added to: an array
    /// handed in may be one that grows or one that does not, and a function that assumed
    /// the first would be writing off the end of the second.
    Unknown,
}

impl Length {
    /// Whether the number is known here, which is what lets `count` fold to one.
    pub fn known(self) -> bool {
        self == Length::Said
    }

    /// The word written where the number would have been.
    pub fn word(self) -> Option<&'static str> {
        match self {
            Length::Said => None,
            Length::Grows => Some("grow"),
            Length::Unknown => Some("any"),
        }
    }
}

/// What a function left open for the caller to fill.
///
/// Not a name, because it does not name anything: a hole is a place a type goes, and
/// the caller's argument is what decides which. So it is a bare word the language
/// provides, like `arr` and `nothing`, and there is exactly one per function — every
/// `any` in a signature is the *same* `any`, which is the whole of what makes
/// `[immut.arr.any 'xs']` and an answer of `any` mean "one of those".
///
/// Two words rather than one, because the two things Quench can say about a type it has
/// not seen are the two things it currently knows: everything compares for equality, and
/// only numbers order. See `notes/a-hole-is-not-a-name.md`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hole {
    /// Any of the sixteen. The body may hold it, copy it, hand it back and `==` it, and
    /// nothing else — because that is all that works on every type.
    Any,
    /// The number types only, which buys the body `<`, `>`, `+`, `-`, `x` and `/` and
    /// costs it every caller holding a `str`, a `bool` or an array.
    Number,
}

impl Hole {
    /// Every hole word there is, which is the list `from_word` below answers to.
    pub const ALL: &[&str] = &["any", "number"];

    fn from_word(word: &str) -> Option<Hole> {
        match word {
            "any" => Some(Hole::Any),
            "number" => Some(Hole::Number),
            _ => None,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Hole::Any => "any",
            Hole::Number => "number",
        }
    }

    /// Whether a type may fill it.
    pub fn takes(self, ty: &Ty) -> bool {
        match self {
            // A hole holds one value, and an array of holes is one allocation of them.
            // Nothing is refused here: what a body may *do* with it is the restriction.
            Hole::Any => true,
            Hole::Number => matches!(
                ty,
                Ty::Int { .. }
                    | Ty::Exact
                    | Ty::F64
                    | Ty::F32
                    | Ty::F16
                    | Ty::Decimal { .. }
            ),
        }
    }
}

/// One function.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Func {
    pub name: String,
    /// `None` for `START`, which nothing may call and so has nobody to be visible to.
    pub visibility: Option<Visibility>,
    /// `None` is `nothing` — the function gives no answer back.
    pub returns: Option<Ty>,
    /// The hole this function opened, if any. `None` on everything handed to the
    /// lowering, because a function with a hole in it is not something to compile — it
    /// is a pattern, and what gets compiled are the copies made from it.
    pub hole: Option<Hole>,
    /// How many of `locals` are parameters. They are the first ones, in order.
    pub takes: usize,
    pub locals: Vec<Local>,
    pub body: Vec<Stmt>,
    /// The name, for pointing at.
    pub at: Span,
}

/// One top-level constant.
///
/// A constant has no storage: its value is worked out here and written in wherever it
/// is named. Which is what the word means — anything needing code to run to produce it
/// would need that code to run before `START`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Constant {
    pub name: String,
    /// The file it was declared in.
    pub in_file: String,
    /// The module it was declared in, outermost first. Empty at the top of a file.
    pub in_module: Vec<String>,
    pub visibility: Visibility,
    pub ty: Ty,
    pub value: Value,
    pub at: Span,
}

/// One entry in a written shape: a number, or the word that says there is no number.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Size {
    Fixed(usize),
    /// The span of the `grow`, for pointing at when it is in the wrong place.
    Grows(Span),
    /// The span of the `any`, the same way.
    Unknown(Span),
}

impl Size {
    /// The word that stands where a number would, if it is not a number.
    fn word(self) -> Option<&'static str> {
        match self {
            Size::Fixed(_) => None,
            Size::Grows(_) => Some("grow"),
            Size::Unknown(_) => Some("any"),
        }
    }

    fn at(self) -> Option<Span> {
        match self {
            Size::Fixed(_) => None,
            Size::Grows(at) | Size::Unknown(at) => Some(at),
        }
    }
}

/// Every word that may stand in a declaration's chain, and where each may stand.
///
/// Three readers match these — a declaration's, a loop's, and a function signature's —
/// and no one of them could hold the others without lying about where the word means
/// something. So this is a list beside the code rather than the code itself, and
/// `tests/means.rs` earns it back by putting every word through the position it belongs
/// to and refusing to let one go missing.
pub const CHAIN_LINKS: &[&str] = &[
    "mut", "immut", "arr", "grow", "temp", "perm", "range", "while", "nothing", "any",
    "number",
];

/// What a written value may be when the type reading it is `bool`.
pub const LITERALS: &[&str] = &["true", "false"];

/// Every module the language provides. Bare, because they are Quench's.
///
/// Two so far, and between them they hold all but four of the provided functions. The
/// first exists because twenty-eight of what was then thirty-two were trigonometry, so a
/// reader of the top-level list would reasonably have concluded that Quench is a
/// calculator with a compiler attached — and `sin` read like a keyword when it is a
/// library. `count`, `stitch`, `is` and `as` are what is left in front.
///
/// None of these could be an `import`. `sin` wants a mantissa that grows, an exponent
/// and Ziv's retry loop, and the language can express none of it, so every one of them
/// is a host call whichever side of a namespace it sits on. What a namespace changes is
/// what a reader has to look at, which was the whole complaint.
pub const MODULES: &[&str] = &["maths", "text", "input"];

/// The functions the language provides, and what each takes.
///
/// A bare word after `call` is one of these and nothing else — anything a writer named
/// is written between marks. Kept in one place so that a diagnostic listing them cannot
/// go stale, which is what happened to the list in E0104 twice in one day.
///
/// The maths here is the half of IEEE 754 that the standard *requires* to be correctly
/// rounded, which is what makes it safe: every engine must give identical bits. `sin`,
/// `log` and the rest are only *recommended*, so three engines calling three C libraries
/// would be three answers, and they wait for one implementation written here.
///
/// The last column is the module each is in, empty for the four at the top level.
pub const PROVIDED: &[(&str, &str, Provides)] = &[
    ("", "count", Provides::Count),
    ("", "stitch", Provides::Stitch),
    // The two that read text, and the only two that carry a chain. `stitch` goes the
    // other way and needs none: what it is given says what to write, and text is the
    // only thing it makes. Coming back, the text says nothing at all — `12` is an `i64`
    // and a `b64` and an `e` — so the type has to be asked for.
    ("", "is", Provides::Reads),
    ("", "as", Provides::Becomes),
    // Taking text apart. `count` and `stitch` stay at the top because they are not only
    // about text -- `count` counts an array too, and `stitch` makes text out of anything.
    // These five are text and nothing else.
    ("text", "slice", Provides::Pieces(0)),
    ("text", "has", Provides::Pieces(1)),
    ("text", "find", Provides::Pieces(2)),
    ("text", "split", Provides::Pieces(3)),
    ("text", "trim", Provides::Pieces(4)),
    // What arrived from outside before the program did anything. `more` is the question
    // and `line` is the answer, the third pair of that shape -- and the check is honest
    // here because nothing races: standard input is read by one program in one order.
    ("input", "all", Provides::Given(0)),
    ("input", "line", Provides::Given(1)),
    ("input", "more", Provides::Given(2)),
    ("input", "arguments", Provides::Given(3)),
    ("maths", "sqrt", Provides::Alone(0)),
    ("maths", "abs", Provides::Alone(1)),
    ("maths", "floor", Provides::Alone(2)),
    ("maths", "ceil", Provides::Alone(3)),
    ("maths", "round", Provides::Alone(4)),
    ("maths", "trunc", Provides::Alone(5)),
    ("maths", "copysign", Provides::Paired(0)),
    ("maths", "min", Provides::Paired(1)),
    ("maths", "max", Provides::Paired(2)),
    ("maths", "remainder", Provides::Paired(3)),
    ("maths", "fma", Provides::Fused),
    // The half IEEE only recommends, which Quench works out itself rather than asking a
    // library that is a little bit wrong in its own way. `b64` only, for now: rounding a
    // correctly-rounded `b64` down to a `b32` rounds twice, and twice is once too many.
    ("maths", "exp", Provides::Slow(0)),
    ("maths", "ln", Provides::Slow(1)),
    ("maths", "sin", Provides::Slow(2)),
    ("maths", "cos", Provides::Slow(3)),
    ("maths", "tan", Provides::Slow(4)),
    ("maths", "atan", Provides::Slow(5)),
    ("maths", "asin", Provides::Slow(6)),
    ("maths", "acos", Provides::Slow(7)),
    ("maths", "sinh", Provides::Slow(8)),
    ("maths", "cosh", Provides::Slow(9)),
    ("maths", "tanh", Provides::Slow(10)),
    ("maths", "asinh", Provides::Slow(11)),
    ("maths", "acosh", Provides::Slow(12)),
    ("maths", "atanh", Provides::Slow(13)),
    ("maths", "cbrt", Provides::Slow(14)),
    ("maths", "atan2", Provides::Power(1)),
    ("maths", "hypot", Provides::Power(2)),
];
/// What one of [`PROVIDED`] is, and how the checker reads it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provides {
    /// How many things are in something.
    Count,
    /// A list of pieces, joined, converting whatever is not text.
    Stitch,
    /// `call is.i64['line']` — whether that text could be read as that type.
    Reads,
    /// `call as.i64['line']` — that text read as that type, stopping when it is not one.
    ///
    /// Which is the whole failure model: nothing here recovers, and the writer is
    /// expected to have asked `is` first. See `notes/checking-comes-first.md`.
    Becomes,
    /// One float in, one out. The number is which.
    Alone(u8),
    /// Two floats in, one out.
    Paired(u8),
    /// Three floats in, one out, rounded once.
    Fused,
    /// One `b64` in, one out, worked out until the rounding is certain.
    Slow(u8),
    /// Two `b64`s in, one out, the same way.
    Power(u8),
    /// One of the five that take text apart. The number is which.
    Pieces(u8),
    /// One of the four that read what arrived from outside. The number is which, and
    /// none of them takes anything.
    Given(u8),
}

/// One variable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Local {
    pub name: String,
    pub ty: Ty,
    pub mutable: bool,
    /// Where the name was written, for pointing at later.
    pub at: Span,
    /// The whole chain that declared it, so an error about `mut` can point at where
    /// `mut` was not, and offer the line with it put in.
    pub chain: Span,
    /// True for a loop's counter, which no `set` may touch. It is not `immut` — the
    /// loop changes it every pass — so the two need telling apart to say why.
    pub counter: bool,
}

/// Which variable. An index into [`Checked::locals`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LocalId(pub u32);

/// A value, with the syntax taken off it and the tree built.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Value {
    /// Text, with every piece already joined and every escape already meant.
    Text(String),
    /// A whole number, and how wide the type reading it is — so that the same digits
    /// are one value under `u8` and a different mistake under it.
    Number { value: i64, bits: u8, signed: bool },
    Bool(bool),
    /// A binary float, kept as its bits so that a checked tree compares like everything
    /// else in it — a float does not — and carrying which of the three it is, since all
    /// three arrive here written the same way.
    Float { bits: u64, width: u8 },
    /// An `e`, kept as the text it was written with. Reading it is the runtime's job,
    /// because the answer does not fit in anything the IR can carry.
    Exact(String),
    /// A `d32` or a `d64`, the same way and for the same reason — with how many digits
    /// to keep, since that is the whole difference between the two.
    Decimal { written: String, digits: u32 },
    /// The value another variable holds.
    Copy(LocalId),
    Binary { op: OpKind, lhs: Box<Value>, rhs: Box<Value> },
    /// The elements of an array, flat and in order however many dimensions it has,
    /// and what one of them is.
    ///
    /// The type is carried because a collector needs it and the elements cannot always
    /// supply it: an empty array has none, and an array written empty is exactly what a
    /// growing one starts as.
    Array { of: Box<Ty>, elements: Vec<Value> },
    /// One element. The shape is carried so the lowering can work out where it is
    /// without going back to the type.
    At { array: Box<Value>, indices: Vec<Value>, shape: Vec<usize> },
    /// Pieces of text, one after another. What juxtaposition has always meant — this
    /// is only the case where a piece is not known until the program runs.
    Join(Vec<Value>),
    /// The text of something that is not text. What `stitch` is made of, and the only
    /// conversion in the language — which is why it has to be asked for by name.
    ///
    /// The type is kept because it is what decides how the value is written down, and
    /// by the time the lowering sees it there is nothing else left to ask.
    Said { of: Box<Value>, ty: Ty },
    /// `not 'ready'` — the opposite of a `bool`.
    Not(Box<Value>),
    /// `copy 'xs'` — a new array holding the same things. `share 'xs'` needs no node of
    /// its own: it is the handle, which is what naming a variable already gives.
    Copied(Box<Value>),
    /// One of the maths functions IEEE 754 requires, on however wide a float it was
    /// given. `which` is the number the lowering writes beside it.
    Maths { which: u8, of: Vec<Value>, width: u8 },
    /// One of the ones it only recommends, which Quench works out itself. A `b64`, and
    /// only a `b64`.
    Slowly { which: u8, of: Vec<Value> },
    /// How many characters a piece of text has, asked while it runs. Which of the two
    /// answers it gives is `[defaults] characters`, and the lowering picks it.
    CountText(Box<Value>),
    /// `call text.slice[…]` and its four neighbours. `which` is the number the lowering
    /// writes beside it, and what each answers with is fixed by that.
    Pieces { which: u8, of: Vec<Value> },
    /// `call input.line[]` and its three neighbours. Takes nothing, which is why there
    /// is nothing under it.
    Given { which: u8 },
    /// How many an array holds, asked while it runs — which is what `count` becomes on
    /// an array that grows, and what it never becomes on one that does not.
    Count(Box<Value>),
    /// `call is.i64['line']` — whether text holds one of those. Always gives an answer.
    CanRead { ty: Ty, text: Box<Value> },
    /// `call as.i64['line']` — text read as one of those, stopping when it is not one.
    ///
    /// The two carry the same type and reach the same reader, which is what makes the
    /// promise `is` gives about `as` a true one rather than a documented one.
    Read { ty: Ty, text: Box<Value> },
    /// A top-level constant, written in where it was named.
    Const(u32),
    /// `add[*1*, *2*]` — the answer a function gave back.
    ///
    /// `fill` is what the callee's hole turned out to be, worked out from the arguments
    /// at the call site. It is `Some` only between the checker deciding it and the
    /// copies being made — after that the call names a real function and there is
    /// nothing left to fill.
    Call { func: u32, args: Vec<Value>, fill: Option<Ty> },
}

pub use quench_parse::OpKind;
pub use quench_qir::Stream;

/// How many indices may be given before an allocation ends, counting outside in.
///
/// `arr.i64 (2 3)` is one allocation and takes two. `arr.arr.i64 (2 3)` is two, and
/// takes one — handing back the inner array — or two.
fn boundaries(ty: &Ty) -> Vec<usize> {
    let mut stops = Vec::new();
    let mut at = 0;
    let mut walking = ty;
    while let Ty::Arr { of, shape, length } = walking {
        at += shape.len() + usize::from(!length.known());
        stops.push(at);
        walking = of;
    }
    stops
}

/// Whether any allocation in this says `any` for its length.
///
/// Different from [`Ty::settled`], which a `grow` fails too: a growing array is one a
/// program makes and fills, and an `any` one is only ever one it was handed.
fn unsaid(ty: &Ty) -> bool {
    match ty {
        Ty::Arr { of, length, .. } => *length == Length::Unknown || unsaid(of),
        _ => false,
    }
}

/// What a provided module holds, for a diagnostic to list.
fn provided_in(module: &str) -> String {
    // `listed` puts the marks on, so they are not put on twice here.
    let all: Vec<&str> =
        PROVIDED.iter().filter(|(held, _, _)| *held == module).map(|(_, said, _)| *said).collect();
    listed(&all)
}

/// The number type at the bottom of this, for saying which one it was.
fn made_number(ty: &Ty) -> &Ty {
    match ty {
        Ty::Arr { of, .. } => made_number(of),
        other => other,
    }
}

/// Whether a number the runtime has to build turns up anywhere inside this.
///
/// An `e` and a decimal are both handles to something made while running, so neither
/// can be written into a table before anything runs.
fn holds_a_made_number(ty: &Ty) -> bool {
    match ty {
        Ty::Exact | Ty::Decimal { .. } => true,
        Ty::Arr { of, .. } => holds_a_made_number(of),
        _ => false,
    }
}

/// Whether every way out of this body ends in a `give`.
///
/// An `if` counts only when it has an `else` and every arm gives, because otherwise
/// there is a way through it that reaches the bottom with no answer. A loop never
/// counts: nothing here knows it runs.
fn gives(body: &[Stmt]) -> bool {
    body.iter().any(|stmt| match stmt {
        Stmt::Give(_) => true,
        Stmt::If { arms, otherwise, .. } => match otherwise {
            Some(body) => arms.iter().all(|arm| gives(&arm.body)) && gives(body),
            None => false,
        },
        _ => false,
    })
}

/// How tightly an operator binds — but only where mathematics settled it.
///
/// A smaller number binds tighter. `None` means nobody ever agreed, so it never binds
/// against anything: brackets say what was meant instead. See
/// `notes/precedence-stops-where-maths-stopped.md`.
fn tier(op: OpKind) -> Option<u8> {
    match op {
        OpKind::Pow => Some(1),
        OpKind::Mul | OpKind::Div => Some(2),
        OpKind::Add | OpKind::Sub => Some(3),
        // Comparison looser than all arithmetic, which is settled. Two comparisons
        // against each other is not, and is caught separately.
        OpKind::Lt | OpKind::Gt | OpKind::Le | OpKind::Ge | OpKind::Eq | OpKind::Ne => Some(4),
        // `mod` written infix, and the logical operators, were invented rather than
        // derived. C and Python chose opposite orders for `&` and both produced famous
        // traps; there was never a right answer to inherit.
        OpKind::Mod | OpKind::And | OpKind::Or => None,
    }
}

fn is_comparison(op: OpKind) -> bool {
    tier(op) == Some(4)
}

/// One thing to print.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Printed {
    Text(String),
    Value { value: Value, ty: Ty },
}

/// A statement, resolved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Stmt {
    Declare { local: LocalId, value: Value },
    /// Arms asked in order; exactly one body runs, or none if nothing held and there is
    /// no `else`.
    If {
        arms: Vec<Arm>,
        otherwise: Option<Vec<Stmt>>,
        /// How many locals existed before this. Anything declared inside an arm is gone
        /// at the closing brace, so only these have to be carried across the join.
        live: u32,
    },
    /// A loop. Its body runs until something stops it, and what stops it is the whole
    /// difference between the two kinds.
    Loop {
        flow: Flow,
        body: Vec<Stmt>,
        /// How many locals existed before the loop. A counting loop's counter is this
        /// index, declared just above the body and living exactly as long as the chain
        /// said. Anything the body declares is gone at the closing brace.
        live: u32,
    },
    /// `break;` — leave the innermost loop now.
    Break,
    /// `give [ … ];`. `None` from a function that gives `nothing` back, which is an
    /// early way out rather than an answer.
    Give(Option<Value>),
    /// A call written for what it does. The answer, if there is one, is dropped.
    Do { func: u32, args: Vec<Value>, fill: Option<Ty> },
    /// `set` — changing something that already exists.
    Assign { to: Place, value: Value },
    /// `add` — one more on the end of a growing array.
    Extend { array: Value, value: Value },
    Print { to: Stream, pieces: Vec<Printed> },
}

/// What drives a loop.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Flow {
    /// `range` — a counter walking from one bound to the other, both ends included.
    /// Both bounds are worked out once, before the first pass.
    Range {
        from: Value,
        to: Value,
        /// `perm`. The counter outlives the loop, holding the last value it took —
        /// which is the only thing in Quench that escapes the block it was declared in.
        keeps: bool,
    },
    /// `while` — a question asked again before every pass, and no counter at all.
    While(Value),
}

/// One `if` or `else-if`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Arm {
    pub condition: Value,
    pub body: Vec<Stmt>,
}

/// Somewhere a value can be put.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Place {
    Local(LocalId),
    /// One element of an array. `array` is what holds it — a variable, or another
    /// element of an array of arrays, which is the same thing one allocation deeper.
    Element { array: Value, indices: Vec<Value>, shape: Vec<usize> },
}

/// What a program means.
pub struct Checked {
    /// Every function, in the order written. `START` is one of them.
    pub funcs: Vec<Func>,
    pub constants: Vec<Constant>,
    /// Which of `funcs` is `START`, if the file has one.
    pub start: Option<usize>,
    pub errors: Vec<Diagnostic>,
}

impl Checked {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn has_start(&self) -> bool {
        self.start.is_some()
    }

    /// `START`'s locals. Most of what there is to look at, in a file that is one.
    pub fn locals(&self) -> &[Local] {
        self.start.map_or(&[], |n| &self.funcs[n].locals)
    }

    /// What `START` does.
    pub fn body(&self) -> &[Stmt] {
        self.start.map_or(&[], |n| &self.funcs[n].body)
    }
}

/// What a function looks like from the outside. Collected before any body is read, so
/// that two functions may call each other and one may call itself without the order
/// they were written in deciding whether that works.
struct Signature {
    /// The name as it was written, which is what a message about it should say. Nobody
    /// writing a one-file program wants to be told about `'main.f'`. The whole key —
    /// file, modules, name — is what `named` is keyed by and what the checked function
    /// is called, and it is not kept twice.
    said: String,
    /// The file it was declared in, by the module name that file gives its declarations.
    in_file: String,
    /// The module it was declared in, outermost first. Empty at the top of a file.
    in_module: Vec<String>,
    visibility: Option<Visibility>,
    returns: Option<Ty>,
    /// The hole this function opened, if it opened one. At most one, always.
    hole: Option<Hole>,
    /// One per parameter, in order.
    takes: Vec<Ty>,
    /// The name, for pointing at.
    at: Span,
    /// The parameter list, for pointing at when a call brings the wrong number.
    list: Span,
}

/// One module, as far as reaching into it is concerned.
#[derive(Clone, Debug)]
struct Known {
    visibility: Visibility,
    /// The file it was declared in.
    in_file: String,
    /// The module it was declared in, outermost first.
    in_module: Vec<String>,
    /// The name, for pointing at.
    at: Span,
}

/// One file of a program: where its text begins in the whole, and the module its
/// declarations go into.
///
/// A file is a module — that is what makes `call 'maths'.'sin'` reach across one — and
/// the name is the file's own, decided by whoever laid the files out rather than written
/// in the source. Which is the one thing about a module that is not written down where
/// a reader is, and it is why `import` names the file at the top of every file that uses
/// it. See `notes/five-lines-a-name-can-cross.md`.
pub struct Part {
    pub at: usize,
    pub name: String,
}

/// Read one file, parse it, and work out what it means.
pub fn check(source: &str) -> Checked {
    check_across(source, &[Part { at: 0, name: "main".to_string() }])
}

/// The same for a program of several files, laid end to end.
///
/// One lex and one parse over the whole thing, and every item is attributed to a file by
/// where it sits. Which is why nothing below here had to learn about files: a `Span` is
/// a range into the concatenation and always was a range into *something*.
pub fn check_across(source: &str, parts: &[Part]) -> Checked {
    debug_assert!(!parts.is_empty(), "a program is at least one file");
    let Parsed { program, errors } = quench_parse::parse(source);
    let mut checker = Checker {
        source,
        locals: Vec::new(),
        scope: vec![HashMap::new()],
        depth: 0,
        returns: None,
        at_file: String::new(),
        imports: HashMap::new(),
        libraries: HashMap::new(),
        at_module: Vec::new(),
        modules: HashMap::new(),
        hole: None,
        opening: false,
        signatures: Vec::new(),
        named: HashMap::new(),
        constants: Vec::new(),
        known: HashMap::new(),
        at_top: false,
        reading: Ty::Int { bits: 64, signed: true },
        in_start: false,
        body: Vec::new(),
        errors,
    };

    // Modules are flattened away first, into a list of what was declared, which file it
    // was in and which module inside that file. Written order is kept, so a constant may
    // still be built out of the ones above it.
    let mut flat: Vec<(String, Vec<String>, &ast::Item)> = Vec::new();
    for item in &program.items {
        let which = parts.partition_point(|part| part.at <= item.span().start);
        checker.at_file = parts[which.saturating_sub(1)].name.clone();
        checker.flatten(std::slice::from_ref(item), &mut Vec::new(), &mut flat);
    }

    // A file always names itself, so that everything in it reaches everything else in it
    // without an import saying so.
    for part in parts {
        checker.imports.entry(part.name.clone()).or_default().push(part.name.clone());
    }
    for (file, at, item) in &flat {
        if matches!(item, ast::Item::Import { .. }) {
            checker.at_file.clone_from(file);
            checker.at_module.clone_from(at);
            checker.imported(item, parts);
        }
    }

    // Constants first, in the order written: one may be built out of those above it,
    // and out of nothing else, because there is nothing else yet worked out.
    for (file, at, item) in &flat {
        if let ast::Item::Const(declaration) = item {
            checker.at_file.clone_from(file);
            checker.at_module.clone_from(at);
            checker.constant(declaration);
        }
    }

    // Then every signature, before any body. Which is what lets `even` call `odd` when
    // `odd` is written underneath it -- and now, in another file.
    for (file, at, item) in &flat {
        if let ast::Item::Func(func) = item {
            checker.at_file.clone_from(file);
            checker.at_module.clone_from(at);
            checker.signature(func);
        }
    }

    let mut funcs = Vec::new();
    for (file, at, item) in &flat {
        if let ast::Item::Func(func) = item {
            checker.at_file.clone_from(file);
            checker.at_module.clone_from(at);
            if let Some(checked) = checker.function(func) {
                funcs.push(checked);
            }
        }
    }
    checker.at_module.clear();
    checker.at_file.clear();

    let start = program.start.as_ref().map(|start| {
        let which = parts.partition_point(|part| part.at <= start.word.start);
        checker.at_file = parts[which.saturating_sub(1)].name.clone();
        checker.in_start = true;
        let body = checker.in_a_function(None, &[], &start.body);
        checker.in_start = false;
        funcs.push(Func {
            name: quench_qir::ENTRY.to_string(),
            visibility: None,
            returns: None,
            takes: 0,
            locals: std::mem::take(&mut checker.locals),
            body,
            at: start.word,
            hole: None,
        });
        funcs.len() - 1
    });

    // The copies, made here rather than anywhere later, so that what leaves the checker
    // is a program with no holes in it. Skipped when something is already wrong: a call
    // whose argument did not check has no fill worked out, and copying against that
    // would turn one error into several.
    let mut errors = checker.errors;
    let (funcs, start) = if errors.is_empty() {
        holes::fill_in(funcs, start, &mut errors)
    } else {
        (funcs, start)
    };

    Checked { funcs, constants: checker.constants, start, errors }
}

struct Checker<'a> {
    source: &'a str,
    locals: Vec<Local>,
    /// Name to declaration, innermost last.
    ///
    /// A block is a scope: a variable declared inside an arm is gone at the closing
    /// brace, because an `if` introduces nothing of its own and so has nothing to say
    /// about how long what is inside it lives.
    scope: Vec<HashMap<String, LocalId>>,
    /// How many loops enclose what is being checked. Nothing but `break` cares, and
    /// what it cares about is whether the number is zero.
    depth: u32,
    /// What the function being checked gives back. `None` is `nothing`.
    returns: Option<Ty>,
    signatures: Vec<Signature>,
    /// Function name to its place in `signatures`.
    named: HashMap<String, u32>,
    constants: Vec<Constant>,
    /// Constant name to its place in `constants`.
    known: HashMap<String, u32>,
    /// Which file is being read or checked, by the module name it gives its
    /// declarations. Every key begins with this.
    at_file: String,
    /// Which file may name which, from `import`. A file always names itself.
    imports: HashMap<String, Vec<String>>,
    /// Which of the language's own modules each file said it uses. The same `import`,
    /// told apart by the same marks: a bare word there is Quench's.
    libraries: HashMap<String, Vec<String>>,
    /// Which module is being read or checked, outermost first, empty at the top of a
    /// file. A declaration's key is this joined with its name, and a name written
    /// without a path is looked for here first and then outward.
    at_module: Vec<String>,
    /// Every module in the file, by its full path. A module is a name like any other,
    /// so reaching into one is checked the way reaching a function in it is.
    modules: HashMap<String, Known>,
    /// The hole the function being read or checked opened, if it opened one.
    ///
    /// One per function, so this is a single slot rather than a list. While a signature
    /// is being read it is being *discovered*, and while a body is being checked it is
    /// already known and every `any` written inside has to be that same one.
    hole: Option<Hole>,
    /// True only while a signature is being read, which is the one place a hole word may
    /// introduce a hole rather than refer to one.
    opening: bool,
    /// True while a top-level constant is being read, where the chain carries a
    /// visibility and carries no answer about changing.
    at_top: bool,
    /// Which number type a bare written value in a sum is read as. A chain that says
    /// `e` makes `*0.1*` one tenth; a chain that says nothing about numbers leaves it
    /// `i64`, and a value that wants otherwise says so itself with `e:*0.1*`.
    reading: Ty,
    /// True while `START` is being read. It answers with `nothing` like any other
    /// function that does, but for a different reason, and so gets a different sentence.
    in_start: bool,
    body: Vec<Stmt>,
    errors: Vec<Diagnostic>,
}

impl<'a> Checker<'a> {
    fn text(&self, span: Span) -> &'a str {
        &self.source[span.start..span.end]
    }

    /// A name with its quotes taken off.
    fn named(&self, span: Span) -> String {
        let raw = self.text(span);
        unmarked(raw)
    }

    fn statement(&mut self, stmt: &ast::Stmt) {
        match stmt {
            ast::Stmt::Var(var) => self.declare(var),
            ast::Stmt::Print(print) => self.print(print),
            ast::Stmt::Set(set) => self.set(set, false),
            ast::Stmt::Add(add) => self.set(add, true),
            ast::Stmt::If(conditional) => self.conditional(conditional),
            ast::Stmt::Loop(repeat) => self.repeat(repeat),
            ast::Stmt::Break(span) => self.leave(*span),
            ast::Stmt::Give(give) => self.answer(give),
            ast::Stmt::Do(call) => self.perform(call),
        }
    }

    // --- modules ------------------------------------------------------------------------

    /// Every declaration in a file, paired with the module it was declared in.
    ///
    /// Modules exist for the checker and for nothing below it: what a module *does* is
    /// decide which names reach which code, and that is settled here. By the time
    /// anything is lowered a function is a function with a longer name.
    fn flatten<'b>(
        &mut self,
        items: &'b [ast::Item],
        at: &mut Vec<String>,
        out: &mut Vec<(String, Vec<String>, &'b ast::Item)>,
    ) {
        for item in items {
            match item {
                ast::Item::Module(module) => {
                    let held = at.clone();
                    let word = module.chain[0];
                    let outer = std::mem::replace(&mut self.at_module, held.clone());
                    let visibility = self.seen_by(&module.chain, word, "module");
                    self.at_module = outer;
                    at.push(self.named(module.name));
                    if let Some(visibility) = visibility {
                        self.modules.insert(
                            Checker::qualified(&self.at_file, &at[..at.len() - 1], &at[at.len() - 1]),
                            Known {
                                visibility,
                                in_file: self.at_file.clone(),
                                in_module: held,
                                at: module.name,
                            },
                        );
                    }
                    self.flatten(&module.items, at, out);
                    at.pop();
                }
                other => out.push((self.at_file.clone(), at.clone(), other)),
            }
        }
    }

    /// `import ['maths'];` — another file of this program, made nameable here.
    ///
    /// What a program *is* comes from `[program] files`, so this cannot add a file. What
    /// it does is say which of them this file uses, which is a use-site record of where
    /// a name came from — the same argument that made `call` mandatory.
    fn imported(&mut self, item: &ast::Item, parts: &[Part]) {
        let ast::Item::Import { name, marked, span, .. } = item else {
            unreachable!("only an import is imported");
        };
        let (name, span) = (*name, *span);
        if !marked {
            return self.library(name, span);
        }
        if !self.at_module.is_empty() {
            self.errors.push(
                Diagnostic::new("E0514", "an `import` belongs to a file, not to a module inside one.")
                    .primary(span, "here")
                    .rule("`import` says which other files this file uses, and a module is not a file")
                    .tip("everything in a file may name everything the file imported.")
                    .fix("move it to the top of the file"),
            );
            return;
        }
        let wanted = self.named(name);
        if wanted == self.at_file {
            self.errors.push(
                Diagnostic::new("E0515", format!("`'{wanted}'` is this file."))
                    .primary(span, "here")
                    .rule("a file names everything in itself already, so importing it says nothing")
                    .fix("remove the line"),
            );
            return;
        }
        if !parts.iter().any(|part| part.name == wanted) {
            let all: Vec<String> = parts
                .iter()
                .filter(|part| part.name != self.at_file)
                .map(|part| format!("`'{}'`", part.name))
                .collect();
            self.errors.push(
                Diagnostic::new("E0516", format!("`'{wanted}'` is not a file of this program."))
                    .primary(name, "here")
                    .rule("what a program is made of is `[program] files` in `QNL-Config.toml`, and an import names one of them")
                    .tip(if all.is_empty() {
                        "this program is one file, so there is nothing to import.".to_string()
                    } else {
                        format!("the others are {}.", all.join(", "))
                    })
                    .fix("add it to `[program] files`, or check the spelling"),
            );
            return;
        }
        let already = self.imports.entry(self.at_file.clone()).or_default();
        if already.contains(&wanted) {
            self.errors.push(
                Diagnostic::new("E0517", format!("`'{wanted}'` is imported twice."))
                    .primary(span, "here")
                    .rule("one import makes a file nameable, and a second says the same thing again")
                    .fix("remove one of them"),
            );
            return;
        }
        already.push(wanted);
    }

    /// `import [maths];` — one of the language's own modules.
    ///
    /// The same word and the same marks rule as a file's, because it is the same idea: a
    /// **library** is imported, whoever wrote it. What the top level holds — `count`,
    /// `stitch`, `is`, `as` — is the language rather than a library, and is always there
    /// the way `if` and `i64` are.
    ///
    /// An import nothing uses is not refused. It costs nothing: a module is host calls
    /// already compiled into every engine, so importing one adds no byte to the artefact
    /// and no work to the build. Go refuses an unused import because its imports really
    /// do drag a package into the build; Rust, whose `use` is aliasing like this one,
    /// only warns — and Quench has no warning, every diagnostic being a refusal. So the
    /// choice was refuse or nothing, and refusing would stop somebody writing the import
    /// before the call, which is how a file gets written.
    fn library(&mut self, name: Span, span: Span) {
        let wanted = self.text(name).to_string();
        if !MODULES.contains(&wanted.as_str()) {
            self.errors.push(
                Diagnostic::new("E0522", format!("`{wanted}` is not a module the language has."))
                    .primary(name, "here")
                    .rule("a bare word in an import is one of Quench's own modules")
                    .tip(format!("they are {}. A file of your own is a marked name instead — `import ['{wanted}'];`.", listed(MODULES)))
                    .fix("check the spelling, or put marks round it"),
            );
            return;
        }
        let already = self.libraries.entry(self.at_file.clone()).or_default();
        if already.contains(&wanted) {
            self.errors.push(
                Diagnostic::new("E0517", format!("`{wanted}` is imported twice."))
                    .primary(span, "here")
                    .rule("one import makes a module nameable, and a second says the same thing again")
                    .fix("remove one of them"),
            );
            return;
        }
        already.push(wanted);
    }

    /// A name as it is keyed: the file, the modules inside it, then the name.
    ///
    /// The file is a module too — that is what makes `call 'maths'.'sin'` work across
    /// files — but it is kept apart from `at_module` rather than being its first
    /// element, so that `module` and `parent` still count the modules a writer wrote.
    /// Otherwise `parent` at the top of a module block would mean the file, which is
    /// what `file` already means, and the language would have two words for one thing.
    fn qualified(file: &str, at: &[String], name: &str) -> String {
        let mut key = String::from(file);
        for module in at {
            key.push('.');
            key.push_str(module);
        }
        key.push('.');
        key.push_str(name);
        key
    }

    /// Where a name is looked for: this module, then the one around it, and outward to
    /// the top of the file. The innermost match wins.
    ///
    /// A path goes through the same walk, so `'trig'.'reduce'` written inside `maths`
    /// finds `maths.trig.reduce` — one rule rather than two, and no word needed for
    /// "start from the top".
    fn reachable(&self, name: &str) -> Option<u32> {
        for depth in (0..=self.at_module.len()).rev() {
            let key = Checker::qualified(&self.at_file, &self.at_module[..depth], name);
            if let Some(&which) = self.named.get(&key) {
                return Some(which);
            }
        }
        // And then a path whose first link is a file this one imported, read from that
        // file's top. Only a *path* -- a bare name never quietly finds another file's
        // function, because then a reader could not tell where it came from, which is
        // the whole reason a file is a module at all.
        if self.imports_a(name) {
            return self.named.get(name).copied();
        }
        None
    }

    /// Whether the first link of this path is a file this one may name.
    ///
    /// A file always names itself, so `'main'.'x'` works from `main` as well.
    fn imports_a(&self, path: &str) -> bool {
        let first = path.split('.').next().unwrap_or(path);
        self.imports
            .get(&self.at_file)
            .is_some_and(|used| used.iter().any(|file| file == first))
    }

    fn reachable_constant(&self, name: &str) -> Option<u32> {
        for depth in (0..=self.at_module.len()).rev() {
            let key = Checker::qualified(&self.at_file, &self.at_module[..depth], name);
            if let Some(&which) = self.known.get(&key) {
                return Some(which);
            }
        }
        if self.imports_a(name) {
            return self.known.get(name).copied();
        }
        None
    }

    /// Whether every module on the way to something may be named from here.
    ///
    /// A path names a module before it names what is inside one, and a module says who
    /// may see it like everything else at the top of a file. So `'maths'.'tables'.'x'`
    /// has three questions in it and this asks the first two.
    fn modules_reach(&mut self, file: &str, at: &[String], whole: Span) -> bool {
        for depth in 1..=at.len() {
            let key = Checker::qualified(file, &at[..depth - 1], &at[depth - 1]);
            let Some(known) = self.modules.get(&key).cloned() else { continue };
            // Named the way a reader would write it: the modules inside the file, and
            // the file itself only when it is not this one.
            let shown = if file == self.at_file {
                at[..depth].join(".")
            } else {
                Checker::qualified(file, &at[..depth - 1], &at[depth - 1])
            };
            if known.visibility.reaches(
                (&self.at_file, &self.at_module),
                (&known.in_file, &known.in_module),
            ) {
                continue;
            }
            let here = format!(
                "`{}`",
                Checker::qualified(&self.at_file, &self.at_module, "").trim_end_matches('.')
            );
            let said = known.visibility.word();
            self.errors.push(
                Diagnostic::new("E0513", format!("the module `'{shown}'` says `{said}`, and this is written in {here}."))
                    .secondary(known.at, format!("`{said}` here"))
                    .primary(whole, "and reached into here")
                    .rule("a module says who may see it, the way everything else at the top of a file does")
                    .tip("what is inside it may say something wider, and never reach further than the module around it does.")
                    .fix("widen what the module says, or move this inside it"),
            );
            return false;
        }
        true
    }

    /// Which function a marked call names, and whether it may be named from here.
    ///
    /// Then it has to get past what the declaration said about who may name it.
    fn function_named(&mut self, call: &ast::Call) -> Option<u32> {
        let mut path = vec![self.named(call.name)];
        for link in &call.chain {
            path.push(self.named(*link));
        }
        let spelt = path.join(".");
        // One rule for a bare name and for a path: look here, then outward. Which is
        // what lets `maths` call `'trig'.'reduce'` without saying `maths` again, and
        // still lets the top of the file say `'maths'.'trig'.'reduce'` in full.
        let found = self.reachable(&spelt);

        let Some(which) = found else {
            // Three reasons a name is not here, and they want different answers. The
            // first is the one a multi-file program meets constantly.
            let first = path[0].clone();
            let unimported = path.len() > 1
                && self.imports.contains_key(&first)
                && first != self.at_file
                && !self.imports_a(&spelt);
            if unimported {
                self.errors.push(
                    Diagnostic::new("E0518", format!("`'{first}'` is a file of this program, and this file does not import it."))
                        .primary(call.name.to(call.close), "here")
                        .rule("`[program] files` says what the program is made of, and `import` says which of them a file uses")
                        .tip("saying it at the top is what lets a reader see where a name came from without leaving the file.")
                        .fix(format!("`import ['{first}'];` at the top of this file")),
                );
                return None;
            }
            // Or it exists somewhere else in this file, and the fix is a path rather
            // than a spelling.
            let elsewhere: Option<String> = self
                .named
                .keys()
                .find(|key| key.rsplit('.').next() == Some(path[path.len() - 1].as_str()))
                .cloned();
            let mut diagnostic =
                Diagnostic::new("E0455", format!("there is nothing called `'{spelt}'`."))
                    .primary(call.name.to(call.close), "here")
                    .rule("a name between marks after `call` is a function the writer declared")
                    .tip("a bare word there is one of Quench's own, which is how a reader tells them apart.");
            diagnostic = match elsewhere {
                Some(key) => {
                    // The key begins with the file, which a reader writing a path inside
                    // that same file would not say.
                    let shown = key.strip_prefix(&format!("{}.", self.at_file)).unwrap_or(&key);
                    diagnostic.fix(format!(
                        "`call '{}'[…]`, which is where that name is",
                        shown.replace('.', "'.'")
                    ))
                }
                None => diagnostic.fix("check the spelling, or declare it with `fn`"),
            };
            self.errors.push(diagnostic);
            return None;
        };

        let signature = &self.signatures[which as usize];
        let (held, held_file) = (signature.in_module.clone(), signature.in_file.clone());
        if !self.modules_reach(&held_file, &held, call.name.to(call.close)) {
            return None;
        }
        let signature = &self.signatures[which as usize];
        if let Some(visibility) = signature.visibility {
            if !visibility
                .reaches((&self.at_file, &self.at_module), (&held_file, &signature.in_module))
            {
                let (said, at, held) =
                    (visibility.word(), signature.at, signature.in_module.join("."));
                let reaches = match visibility {
                    Visibility::Module => {
                        format!("`module` reaches `{held}` and everything nested inside it")
                    }
                    Visibility::Parent => {
                        format!("`parent` reaches the module around `{held}`, and everything under that")
                    }
                    // The rung that was inert until a program could be more than one
                    // file, and is the first thing anybody meets now that it can.
                    _ => format!("`file` reaches the file it is written in, which is `{held_file}`"),
                };
                let here = if self.at_module.is_empty() {
                    "the top of the file".to_string()
                } else {
                    format!("`{}`", self.at_module.join("."))
                };
                self.errors.push(
                    Diagnostic::new("E0511", format!("`'{spelt}'` says `{said}`, and this is written in {here}."))
                        .secondary(at, format!("`{said}` here"))
                        .primary(call.name.to(call.close), "and named here")
                        .rule(reaches)
                        .tip("the ladder is `module`, `parent`, `file`, `program`, `export`, narrowest first.")
                        .fix("widen what it says, or move this inside"),
                );
                return None;
            }
        }
        Some(which)
    }

    // --- the top of a file ------------------------------------------------------------

    /// Read a chain's visibility link, and complain when there is none or two.
    fn seen_by(&mut self, chain: &[Span], word: Span, what: &str) -> Option<Visibility> {
        let mut found: Option<(Visibility, Span)> = None;
        for link in chain.iter().skip(1) {
            let Some(visibility) = Visibility::from_word(self.text(*link)) else { continue };
            match found {
                None => found = Some((visibility, *link)),
                Some((_, first)) => self.errors.push(
                    Diagnostic::new("E0458", "this says twice who can see it.")
                        .secondary(first, format!("`{}` here", self.text(first)))
                        .primary(*link, format!("and `{}` here", self.text(*link)))
                        .rule(format!("a declaration says one of {}, once", listed(Visibility::ALL)))
                        .fix("keep the one that was meant"),
                ),
            }
        }
        match found {
            Some((visibility, at)) => {
                // Two of the five name a boundary that may not be there. Refused where
                // it is written, the way `any` outside a function is, rather than
                // quietly widened into the next rung up.
                let missing = match visibility {
                    Visibility::Module if self.at_module.is_empty() => {
                        Some(("module", "there is no module around this"))
                    }
                    // At one level deep the module around this one *is* the file, and
                    // `parent` would then be a second spelling of `file`.
                    Visibility::Parent if self.at_module.len() < 2 => {
                        Some(("parent", "there is no module around the one this is in"))
                    }
                    _ => None,
                };
                if let Some((said, why)) = missing {
                    self.errors.push(
                        Diagnostic::new("E0512", format!("`{said}` says a boundary that is not here."))
                            .primary(at, why)
                            .rule(format!("{} are the five, narrowest first, and each names a boundary a name may cross", listed(Visibility::ALL)))
                            .tip("`module` wants a module around it, and `parent` wants one around that.")
                            .fix("`file` here"),
                    );
                    return None;
                }
                Some(visibility)
            }
            None => {
                self.errors.push(
                    Diagnostic::new("E0459", format!("this {what} does not say who can see it."))
                        .primary(word, "here")
                        .rule(format!("everything at the top of a file or a module says one of {}, and silence is not one of them", listed(Visibility::ALL)))
                        .tip("`file` is the careful answer: nothing outside this file can name it.")
                        .fix("`file` here, `module` for this module alone, `export` for anything outside the program"),
                );
                None
            }
        }
    }

    /// `const.export.i64 ['LIMIT'] = [*100*];`
    fn constant(&mut self, declaration: &ast::Var) {
        let word = declaration.chain[0];
        let visibility = self.seen_by(&declaration.chain, word, "constant");

        // Checked as a declaration, because it is one -- which means the chain, the
        // shape and the count of names against values all get the errors they already
        // had. The visibility link is skipped over on the way past.
        let before = self.locals.len();
        let outer = std::mem::take(&mut self.body);
        self.at_top = true;
        self.declare(declaration);
        self.at_top = false;
        let statements = std::mem::replace(&mut self.body, outer);

        for statement in statements {
            let Stmt::Declare { local, value } = statement else { continue };
            let local = &self.locals[local.0 as usize];
            let (name, ty, at) = (local.name.clone(), local.ty.clone(), local.at);

            // A constant array lives in the module, beside the text — every engine
            // lays those out before anything runs, so its handle is known here. What
            // it may not do is grow: a shape that says `grow` says there is no number
            // yet, and a table is a number of things written down.
            if !ty.settled() {
                self.errors.push(
                    Diagnostic::new("E0460", "a constant array cannot grow.")
                        .primary(at, "here")
                        .rule("a constant is written into the module before anything runs, and what is written down is however many were written")
                        .tip("`grow` says there is no number yet, and a constant is the answer to a question nobody is going to ask again.")
                        .fix("give it a size, or declare it inside a function with `var`"),
                );
                continue;
            }
            if holds_a_made_number(&ty) {
                let made = made_number(&ty);
                let (article, name) = (made.article(), made.name());
                let said = if matches!(ty, Ty::Arr { .. }) {
                    format!("a constant array of `{name}` is not built yet.")
                } else {
                    format!("a constant `{name}` is not built yet.")
                };
                self.errors.push(
                    Diagnostic::new("E0485", said)
                        .primary(at, "here")
                        .rule(format!("a constant is written into the module before anything runs, and {article} `{name}` is a handle the runtime makes rather than a number the module can carry"))
                        .tip("every other number type fits in the module as itself — a whole number, or the bits of a binary float — all of them known before anything runs.")
                        .fix("declare it inside a function with `var` for now"),
                );
                continue;
            }

            let at_index = self.constants.len() as u32;
            let key = Checker::qualified(&self.at_file, &self.at_module, &name);
            self.known.insert(key.clone(), at_index);
            self.constants.push(Constant {
                name: key,
                in_file: self.at_file.clone(),
                in_module: self.at_module.clone(),
                visibility: visibility.unwrap_or(Visibility::File),
                ty,
                value,
                at,
            });
        }

        // A constant has no storage, so it leaves no local behind. The scope entry the
        // declaration made goes with it -- `known` is what names it from here on.
        self.locals.truncate(before);
        for scope in &mut self.scope {
            scope.retain(|_, id| (id.0 as usize) < before);
        }
    }

    /// What a function looks like from the outside, read before any body is.
    fn signature(&mut self, func: &ast::Func) {
        // The one place a hole word opens a hole rather than referring to one. Set and
        // cleared out here rather than inside, because `read_signature` leaves by
        // several doors and a flag cleared at three of four is a flag that is wrong.
        self.hole = None;
        self.opening = true;
        self.read_signature(func);
        self.opening = false;
        self.hole = None;
    }

    fn read_signature(&mut self, func: &ast::Func) {
        let name = self.named(func.name);
        let key = Checker::qualified(&self.at_file, &self.at_module, &name);
        let word = func.chain[0];
        let visibility = self.seen_by(&func.chain, word, "function");

        if let Some(&first) = self.named.get(&key) {
            let at = self.signatures[first as usize].at;
            self.errors.push(
                Diagnostic::new("E0461", format!("`'{name}'` is declared twice."))
                    .secondary(at, "declared here first")
                    .primary(func.name, "and declared again here")
                    .rule("one name means one thing, and two functions of a name means neither does")
                    .fix("rename one of them"),
            );
            return;
        }

        // The type is whatever the chain says that is not `fn`, not a visibility and
        // not an `arr` link.
        let mut returns = None;
        let mut said = false;
        let mut arrays = Vec::new();
        let mut ty_span = word;
        for link in func.chain.iter().skip(1) {
            let word = self.text(*link);
            if Visibility::from_word(word).is_some() {
                continue;
            }
            if word == "arr" && !said {
                arrays.push(*link);
                continue;
            }
            if said {
                self.errors.push(
                    Diagnostic::new("E0463", format!("`{word}` comes after what the function gives back."))
                        .primary(*link, "here")
                        .rule("the last link of a function's chain is what it answers with")
                        .fix("move it before, or remove it"),
                );
                continue;
            }
            said = true;
            ty_span = *link;
            if word == "nothing" {
                continue;
            }
            match self.a_type(*link) {
                Some(ty) => returns = Some(ty),
                None => return,
            }
        }
        if returns.is_some() || !arrays.is_empty() {
            match self.arrayed(returns, ty_span, &arrays, &func.shape, func.shape_span) {
                Some(ty) => returns = Some(ty),
                None => return,
            }
        }
        if !said {
            self.errors.push(
                Diagnostic::new("E0464", "this function does not say what it gives back.")
                    .primary(*func.chain.last().unwrap_or(&word), "the chain ends here")
                    .rule("a function always says what it answers with, and `nothing` is one of the answers")
                    .tip("`nothing` is a real link, not an omission -- a reader should not have to read the body to find out.")
                    .fix("`fn.<visibility>.nothing [...]` if it gives nothing back"),
            );
            return;
        }

        let mut takes = Vec::new();
        let mut ok = true;
        for param in &func.params {
            match self.parameter_ty(param) {
                Some(ty) => takes.push(ty),
                None => ok = false,
            }
        }
        if !ok {
            return;
        }

        let hole = self.hole;
        self.named.insert(key.clone(), self.signatures.len() as u32);
        self.signatures.push(Signature {
            said: name.clone(),
            in_file: self.at_file.clone(),
            in_module: self.at_module.clone(),
            visibility,
            returns,
            hole,
            takes,
            at: func.name,
            list: func.takes,
        });
    }

    /// A parameter's chain: the same links `var` would carry, said the same way.
    fn parameter_ty(&mut self, param: &ast::Param) -> Option<Ty> {
        let mut mutable = None;
        let mut ty_span = None;
        let mut arrays = Vec::new();
        for link in &param.chain {
            match self.text(*link) {
                word @ ("mut" | "immut") if ty_span.is_none() => mutable = Some(word == "mut"),
                "arr" if ty_span.is_none() => arrays.push(*link),
                word => {
                    if ty_span.is_some() {
                        self.errors.push(
                            Diagnostic::new("E0403", format!("`{word}` comes after the type."))
                                .primary(*link, "here")
                                .rule("the type is the last link of a parameter's chain")
                                .fix("move it before the type, or remove it"),
                        );
                        return None;
                    }
                    ty_span = Some(*link);
                }
            }
        }
        if mutable.is_none() {
            self.errors.push(
                Diagnostic::new("E0465", "this parameter does not say whether it can change.")
                    .primary(param.span, "here")
                    .rule("a parameter is a variable, and a variable says `mut` or `immut`")
                    .tip("`immut` is nearly always the one, and saying so is the point.")
                    .fix(format!("`immut.{}`", self.text(*param.chain.last().unwrap_or(&param.span)))),
            );
            return None;
        }
        let ty_span = ty_span?;
        let element = self.a_type(ty_span);
        self.arrayed(element, ty_span, &arrays, &param.shape, param.shape_span)
    }

    /// Wrap an element type in the `arr` links written before it, with their sizes.
    ///
    /// The same three refusals a declaration gets, because it is the same question:
    /// two `arr` links, an element type that is not packed yet, and a shape on
    /// something that is not an array.
    fn arrayed(
        &mut self,
        element: Option<Ty>,
        element_span: Span,
        arrays: &[Span],
        shape: &[Span],
        shape_span: Option<Span>,
    ) -> Option<Ty> {
        let Some(arr) = arrays.first() else {
            if let Some(span) = shape_span {
                self.errors.push(
                    Diagnostic::new("E0425", "only an array has a shape.")
                        .primary(span, "here")
                        .rule("a shape says how many elements an array holds, and this is no array")
                        .fix("add `arr` to the chain, or remove the shape"),
                );
                return None;
            }
            return element;
        };
        let element = element?;
        let sizes = self.sizes(shape, shape_span, *arr)?;

        // One size per `arr` link, and the innermost takes whatever is left — so
        // `arr.i64 (2 3)` is one allocation of six and `arr.arr.i64 (2 3)` is two
        // allocations of three with an array of two handles over them. Every `arr` is
        // one allocation; how many of them there are is how many links were written.
        if sizes.len() < arrays.len() {
            self.errors.push(
                Diagnostic::new("E0423", format!(
                    "this says `arr` {} and gives {}.",
                    counted(arrays.len(), "time"),
                    counted(sizes.len(), "size")
                ))
                .primary(shape_span.unwrap_or(element_span), format!("{} here", counted(sizes.len(), "size")))
                .secondary(arrays[0].to(*arrays.last().expect("one at least")), format!("`arr` {}", counted(arrays.len(), "time")))
                .rule("every `arr` link is one allocation, and every allocation says how big it is")
                .tip("the innermost takes whatever sizes are left over, which is what makes `arr.i64 (2 3)` a rectangle.")
                .fix(format!("give at least {}", counted(arrays.len(), "size"))),
            );
            return None;
        }

        // Built inside out: the innermost allocation takes the sizes nothing else
        // claimed, and each link outside it wraps what is under it in one more.
        let mut ty = self.one_allocation(&sizes[arrays.len() - 1..], element, *arr)?;
        for (n, size) in sizes[..arrays.len() - 1].iter().enumerate().rev() {
            ty = self.one_allocation(std::slice::from_ref(size), ty, arrays[n])?;
        }
        Some(ty)
    }

    /// One `arr` link's worth of sizes, wrapped round what it holds.
    ///
    /// Only the first may say `grow`. Indexing is `(i - 1) x stride + j` and a stride is
    /// the sizes *under* a dimension, so the outermost is the one dimension whose size
    /// the arithmetic never asks for — and the only one that can be left unsaid.
    fn one_allocation(&mut self, sizes: &[Size], of: Ty, arr: Span) -> Option<Ty> {
        let length = match sizes.first() {
            Some(Size::Grows(_)) => Length::Grows,
            Some(Size::Unknown(_)) => Length::Unknown,
            _ => Length::Said,
        };
        let rest = if length.known() { sizes } else { &sizes[1..] };
        if let Some(bad) = rest.iter().find(|size| size.word().is_some()) {
            let (word, at) = (bad.word().expect("just found"), bad.at().expect("just found"));
            self.errors.push(
                Diagnostic::new("E0480", format!("only the first size of an allocation can say `{word}`."))
                    .primary(at, "here")
                    .secondary(arr, "this allocation")
                    .rule("finding an element is `(i - 1) x stride + j`, and a stride is the sizes under a dimension — so every size but the outermost has to be known")
                    .tip("`arr.arr.i64 (2 grow)` is two rows that each grow, which is what a growing inner dimension usually means.")
                    .fix(format!("`{word}` first, or a number here")),
            );
            return None;
        }
        let fixed: Vec<usize> = rest
            .iter()
            .map(|size| match size {
                Size::Fixed(n) => *n,
                _ => unreachable!("just refused above"),
            })
            .collect();
        Some(Ty::Arr { of: Box::new(of), shape: fixed, length })
    }

    /// A written value read as one of the whole-number types.
    ///
    /// The same digits are a different thing under each: `*200*` is a `u8` and is not
    /// an `i8`, which is not a mistake about the number but about which type was asked
    /// to hold it, and the message says which.
    fn a_whole_number(&mut self, digits: &str, ty: &Ty, at: Span) -> Option<Value> {
        let Ty::Int { bits, signed } = ty else {
            unreachable!("only a whole-number type reads one");
        };
        let (bits, signed) = (*bits, *signed);
        let (low, high) = whole_range(bits, signed);
        // The same reader `call as.i64['…']` uses, so that what an `i64` may be written
        // with and what one may be read from are one answer rather than two.
        match quench_num::read_whole(digits, bits, signed) {
            Whole::Read(n) => Some(Value::Number { value: n, bits, signed }),
            Whole::Outside => {
                let holds = if signed {
                    format!("`{}` holds {low} to {high}", ty.name())
                } else {
                    format!("`{}` holds 0 to {}", ty.name(), high as u64)
                };
                self.errors.push(
                    Diagnostic::new("E0489", format!("`{digits}` does not fit in {} `{}`.", ty.article(), ty.name()))
                        .primary(at, "here")
                        .rule(holds)
                        .tip(if !signed && digits.starts_with('-') {
                            "an unsigned type holds no negative number at all."
                        } else {
                            "a written value is read by the type it is given to, and this one has an edge."
                        })
                        .fix("write a number in that range, or declare it a wider type"),
                );
                None
            }
            Whole::NotOne => {
                self.errors.push(
                    Diagnostic::new("E0407", format!("`{digits}` is not {} `{}`.", ty.article(), ty.name()))
                        .primary(at, "here")
                        .rule(format!("a written value is read by the type it is given to, and `{}` reads whole numbers", ty.name()))
                        .tip(if signed { "a negative one wears its minus inside the marks." } else { "an unsigned type holds no negative number at all." })
                        .fix("write a whole number"),
                );
                None
            }
        }
    }

    /// A written value read as one of the three binary floats.
    ///
    /// `b16` is rounded to the nearest binary16 here as well as after every operation:
    /// a literal that binary16 cannot hold is the nearest one it can, which is what
    /// writing it in that type asked for.
    fn a_float(&mut self, written: &str, ty: &Ty, at: Span) -> Option<Value> {
        let width = match ty {
            Ty::F16 => 16u8,
            Ty::F32 => 32,
            _ => 64,
        };
        match quench_num::read_float(written, width) {
            Some(bits) => Some(Value::Float { bits, width }),
            None => {
                self.errors.push(
                    Diagnostic::new("E0486", format!("`{written}` is not a `{}`.", ty.name()))
                        .primary(at, "here")
                        .rule("a binary float is written as a number with or without a point: `*1.5*`, `*-0.25*`, `*3*`")
                        .tip("`infinity` and `not-a-number` are answers a program can reach, and not things it can write.")
                        .fix("write a number"),
                );
                None
            }
        }
    }

    /// A decimal float, read to however many digits its type keeps.
    ///
    /// Reading happens twice: here, to refuse what is not a number at all, and again at
    /// runtime, because what it reads to is a coefficient and an exponent rather than
    /// something the IR can carry.
    fn a_decimal(&mut self, written: &str, digits: u32, at: Span) -> Option<Value> {
        let format = if digits == 7 { quench_num::D32 } else { quench_num::D64 };
        match quench_num::Decimal::parse(written, format) {
            Some(_) => Some(Value::Decimal { written: written.to_string(), digits }),
            None => {
                let name = if digits == 7 { "d32" } else { "d64" };
                self.errors.push(
                    Diagnostic::new("E0489", format!("`{written}` is not a `{name}`."))
                        .primary(at, "here")
                        .rule("a decimal float is written as a number with or without a point: `*1.5*`, `*-0.25*`, `*3*`")
                        .tip("a ratio is not one of them — `*1/3*` is an `e`, and a third is what a decimal cannot hold.")
                        .fix("write a number"),
                );
                None
            }
        }
    }

    /// One type link, understood or honestly refused.
    fn a_type(&mut self, link: Span) -> Option<Ty> {
        let word = self.text(link);
        if let Some(ty) = Ty::simple(word) {
            return Some(ty);
        }
        if let Some(asked) = Hole::from_word(word) {
            return self.a_hole(asked, link);
        }
        {
            self.errors.push(
                Diagnostic::new("E0402", format!("`{word}` is not a type."))
                    .primary(link, "here")
                    .rule("a chain says the type of what it is describing")
                    .tip("the types are the numbers, `e`, `bool` and `str`, and `nothing` where a function gives none back.")
                    .fix("check the spelling"),
            );
        }
        None
    }

    /// `any` or `number` where a type would go.
    ///
    /// Three answers, and which one depends on where it was written. Opening a signature
    /// introduces the hole; writing it again anywhere in that function refers to the same
    /// one; writing it anywhere else is a mistake, because a hole belongs to a function
    /// and there is no function here to fill it.
    fn a_hole(&mut self, asked: Hole, link: Span) -> Option<Ty> {
        match self.hole {
            // The first one in a signature is what opens it.
            None if self.opening => {
                self.hole = Some(asked);
                Some(Ty::Hole(asked))
            }
            Some(already) if already == asked => Some(Ty::Hole(asked)),
            // A second, different hole word. Refused rather than given a second hole,
            // because `any` and `number` promise different things and a function with
            // two holes needs names for them — which is the decision maps will force and
            // this one does not.
            Some(already) => {
                self.errors.push(
                    Diagnostic::new("E0500", format!("this function opened `{}`, and this says `{}`.", already.word(), asked.word()))
                        .primary(link, format!("`{}` here", asked.word()))
                        .rule("a function has one hole, and every mention of it says the same word")
                        .tip("`any` and `number` are different holes: one takes every type and forbids `<`, and the other takes the numbers and allows it.")
                        .fix(format!("say `{}` here too", already.word())),
                );
                None
            }
            None => {
                self.errors.push(
                    Diagnostic::new("E0501", format!("`{}` is a hole, and there is no function here to fill it.", asked.word()))
                        .primary(link, "here")
                        .rule("a hole is opened by a function's signature and filled by whoever calls it")
                        .tip("inside such a function it may be written again, and it means that same hole. Nowhere else can.")
                        .fix("name a type"),
                );
                None
            }
        }
    }

    /// A whole function: its parameters put in scope, then its body.
    fn function(&mut self, func: &ast::Func) -> Option<Func> {
        let name = Checker::qualified(&self.at_file, &self.at_module, &self.named(func.name));
        let which = *self.named.get(&name)?;
        let signature = &self.signatures[which as usize];
        let (visibility, returns) = (signature.visibility, signature.returns.clone());
        let takes: Vec<Ty> = signature.takes.clone();
        // Known now rather than discovered, so a hole word written in the body refers to
        // the one the signature opened and cannot open a second.
        let hole = signature.hole;
        self.hole = hole;

        let params: Vec<(Span, Span, Ty)> = func
            .params
            .iter()
            .zip(&takes)
            .map(|(p, ty)| (p.name, p.span, ty.clone()))
            .collect();
        let body = self.in_a_function(returns.clone(), &params, &func.body);

        // A function that answers with something has to answer on every way out. There
        // is no value to hand back otherwise, and no honest thing to invent.
        if returns.is_some() && !gives(&body) {
            let ty = returns.clone().expect("just checked");
            self.errors.push(
                Diagnostic::new("E0466", format!("this function says it gives back {} `{}`, and does not always.", ty.article(), ty.name()))
                    .primary(func.name, "here")
                    .rule("every way out of a function that answers with something ends in a `give`")
                    .tip("an `if` counts only when it has an `else` and every arm gives -- otherwise there is a way through with no answer.")
                    .fix("`give [ … ];` at the end"),
            );
        }

        self.hole = None;
        Some(Func {
            name,
            visibility,
            returns,
            takes: takes.len(),
            locals: std::mem::take(&mut self.locals),
            body,
            at: func.name,
            hole,
        })
    }

    /// Check a body with a scope, a set of parameters and a return type of its own.
    fn in_a_function(
        &mut self,
        returns: Option<Ty>,
        params: &[(Span, Span, Ty)],
        body: &[ast::Stmt],
    ) -> Vec<Stmt> {
        self.locals = Vec::new();
        self.scope = vec![HashMap::new()];
        self.depth = 0;
        self.returns = returns;

        for (name, chain, ty) in params {
            let text = self.named(*name);
            let id = LocalId(self.locals.len() as u32);
            self.locals.push(Local {
                counter: false,
                name: text.clone(),
                ty: ty.clone(),
                // Every parameter is written to say so, and `mut` on one changes only
                // this function's copy -- nothing here is a reference yet.
                mutable: self.text(*chain).starts_with("mut."),
                at: *name,
                chain: *chain,
            });
            if self.scope[0].insert(text.clone(), id).is_some() {
                self.errors.push(
                    Diagnostic::new("E0201", format!("`'{text}'` is declared twice."))
                        .primary(*name, "and declared again here")
                        .rule("one name means one thing")
                        .fix("rename one of them"),
                );
            }
        }

        self.statements(body)
    }

    // --- declarations ---------------------------------------------------------------

    fn declare(&mut self, var: &ast::Var) {
        let Some(chain) = self.chain(var) else { return };

        for (n, name_span) in var.names.iter().enumerate() {
            let name = self.named(*name_span);

            // Checked before the type is, so that declaring `'x'` twice is reported as
            // declaring it twice even when the second one also names a type that is not
            // built. The name is what collided.
            if let Some(before) = self.seen(&name) {
                let first = &self.locals[before.0 as usize];
                self.errors.push(
                    Diagnostic::new("E0201", format!("`'{name}'` is declared twice."))
                        .secondary(
                            first.at,
                            format!("declared here first, as `{}`", first.ty.name()),
                        )
                        .primary(
                            *name_span,
                            format!("and declared again here, as `{}`", self.text(chain.ty_span)),
                        )
                        .rule("a name is declared once, and keeps the type it was declared with")
                        .tip("a declaration always makes a new name. It never replaces one.")
                        .fix("rename one of them"),
                );
                continue;
            }

            let Some(ty) = chain.ty.clone() else { continue };
            let Some(value) = var.values.get(n) else { continue };
            let Some(value) = self.value(value, &ty, chain.ty_span) else { continue };

            let local = LocalId(self.locals.len() as u32);
            let chain_span = var.chain[0].to(*var.chain.last().expect("a chain has links"));
            self.locals.push(Local {
                counter: false,
                name: name.clone(),
                ty,
                mutable: chain.mutable,
                at: *name_span,
                chain: chain_span,
            });
            self.scope.last_mut().expect("a scope is always open").insert(name, local);
            self.body.push(Stmt::Declare { local, value });
        }
    }

    /// `var` . `mut`? . `arr`* . type, then a shape in brackets.
    fn chain(&mut self, var: &ast::Var) -> Option<Chain> {
        // Not a `bool` with a default. Silence is a third answer, and it is refused.
        let mut mutable: Option<(bool, Span)> = None;
        let mut ty_span = None;
        let mut arrays: Vec<Span> = Vec::new();

        for link in var.chain.iter().skip(1) {
            match self.text(*link) {
                // Who can see it was read before this, by whoever is at the top of the
                // file. Here it is simply not the type.
                word if self.at_top && Visibility::from_word(word).is_some() => {}
                word @ ("mut" | "immut") if self.at_top => self.errors.push(
                    Diagnostic::new("E0473", format!("a constant never changes, so `{word}` says nothing."))
                        .primary(*link, "here")
                        .rule("`const` is the answer to whether it changes, which is why it is a different word from `var`")
                        .tip("a variable that never changes is `var.immut`, inside a function.")
                        .fix("remove it"),
                ),
                word @ ("mut" | "immut") if mutable.is_none() && ty_span.is_none() => {
                    mutable = Some((word == "mut", *link));
                }
                word @ ("mut" | "immut") if ty_span.is_some() => {
                    self.errors.push(
                        Diagnostic::new("E0401", format!("`{word}` comes before the type."))
                            .primary(*link, "here")
                            .rule("a chain reads `var`, then whether it changes, then what it is")
                            .fix(format!("`var.{word}.<type>`")),
                    );
                }
                word @ ("mut" | "immut") => {
                    let (_, first) = mutable.expect("something was said already");
                    self.errors.push(
                        Diagnostic::new("E0443", "this says twice whether it can change.")
                            .secondary(first, format!("`{}` here", self.text(first)))
                            .primary(*link, format!("and `{word}` here"))
                            .rule("a declaration says `mut` or `immut`, once")
                            .fix("keep the one that was meant"),
                    );
                }
                "arr" if ty_span.is_none() => arrays.push(*link),
                word if ty_span.is_none() => {
                    // A hole is a type inside the function that opened it, which is
                    // what makes `var.mut.number ['best']` the way a body holds one of
                    // whatever it was handed.
                    if Ty::simple(word).is_none() && Hole::from_word(word).is_none() {
                        self.errors.push(
                            Diagnostic::new("E0402", format!("`{word}` is not a type."))
                                .primary(*link, "here")
                                .rule("a declaration's chain ends with the type of what it declares")
                                .tip("the types are the numbers, `e`, `bool` and `str`.")
                                .fix("check the spelling"),
                        );
                        return None;
                    }
                    ty_span = Some(*link);
                }
                word => {
                    self.errors.push(
                        Diagnostic::new("E0403", format!("`{word}` comes after the type."))
                            .primary(*link, "here")
                            .rule("the type is the last link in a declaration's chain")
                            .fix("move it before the type, or remove it"),
                    );
                }
            }
        }

        // A constant is never mutable, and does not have to say so.
        if self.at_top {
            mutable = mutable.or(Some((false, var.chain[0])));
        }
        let Some((mutable, _)) = mutable else {
            self.errors.push(
                Diagnostic::new("E0444", "this declaration does not say whether it can change.")
                    .primary(var.chain[0], "here")
                    .rule("a declaration says `mut` or `immut`, and silence is not one of them")
                    .tip("it goes between `var` and the type, where visibility goes on the things that have it.")
                    .fix("`var.immut.<type>` if it never changes, `var.mut.<type>` if it does"),
            );
            return None;
        };

        let ty_span = ty_span.or_else(|| {
            self.errors.push(
                Diagnostic::new("E0404", "this declaration does not say what it is declaring.")
                    .primary(var.chain[0], "the chain ends here")
                    .rule("a declaration always says its type, because a written value means nothing without one")
                    .fix("`var.str [...]`, or whichever type was meant"),
            );
            None
        })?;

        // `a_type` rather than `Ty::simple`, because a hole is a type here too -- and
        // saying so in one place is what makes writing it outside a function that opened
        // one a refusal with a reason rather than "not a type".
        let mut ty = self.a_type(ty_span);

        // The same three questions a parameter and a return type get, asked by the same
        // code: how many `arr` links, what they hold, and what shape.
        if !arrays.is_empty() || var.shape_span.is_some() {
            ty = Some(self.arrayed(ty, ty_span, &arrays, &var.shape, var.shape_span)?);
        }

        Some(Chain { mutable, ty, ty_span })
    }

    /// `(5)` or `(2 3)` — the sizes, which an array must have and must not be empty.
    fn sizes(
        &mut self,
        shape: &[Span],
        shape_span: Option<Span>,
        arr: Span,
    ) -> Option<Vec<Size>> {
        let Some(span) = shape_span else {
            self.errors.push(
                Diagnostic::new("E0426", "this array does not say how big it is.")
                    .primary(arr, "here")
                    .rule("an array says its size in brackets after the chain, because the size is part of the type")
                    .tip("`grow` is a size too: it says there is no number yet, which is different from not saying.")
                    .fix("`arr.i64 (5)`, or `arr.i64 (grow)` if it is to be filled afterwards"),
            );
            return None;
        };
        if shape.is_empty() {
            self.errors.push(
                Diagnostic::new("E0427", "this shape is empty.")
                    .primary(span, "here")
                    .rule("a shape is one size for each dimension, and there is always at least one")
                    .fix("`(5)`, or whichever size was meant"),
            );
            return None;
        }
        let mut sizes = Vec::new();
        for size in shape {
            // The two sizes that are not numbers. `grow` says there is no number yet
            // and this may be added to; `any` says the number was never told to whoever
            // wrote this, which is a different thing and grants less.
            match self.text(*size) {
                "grow" => {
                    sizes.push(Size::Grows(*size));
                    continue;
                }
                "any" => {
                    sizes.push(Size::Unknown(*size));
                    continue;
                }
                _ => {}
            }
            match self.text(*size).parse::<usize>() {
                Ok(0) => {
                    self.errors.push(
                        Diagnostic::new("E0428", "an array of nothing holds nothing.")
                            .primary(*size, "here")
                            .rule("every size in a shape is at least one")
                            .fix("give it a size, or do not declare it"),
                    );
                    return None;
                }
                Ok(n) => sizes.push(Size::Fixed(n)),
                Err(_) => {
                    self.errors.push(
                        Diagnostic::new("E0429", format!("`{}` is not a size.", self.text(*size)))
                            .primary(*size, "here")
                            .rule("a size is a whole number written without marks, or `grow` where there is no number yet, or `any` where the number was never said")
                            .fix("write a whole number, `grow`, or `any`"),
                    );
                    return None;
                }
            }
        }
        Some(sizes)
    }

    // --- values ---------------------------------------------------------------------

    fn value(&mut self, value: &ast::Value, ty: &Ty, ty_span: Span) -> Option<Value> {
        // A value that is one call is that call's answer, whatever the type -- the same
        // way a value that is one name is that variable's. Neither is a list of pieces,
        // and reading either as one would ask the wrong question about it.
        if let [term @ (ast::Term::Call(_)
        | ast::Term::Piece(ast::Piece::Call(_))
        | ast::Term::Not { .. }
        | ast::Term::Handed { .. })] = value.terms.as_slice()
        {
            let built = self.term(term)?;
            let found = self.type_of(&built, value.span)?;
            if !ty.accepts(&found) {
                self.errors.push(
                    Diagnostic::new("E0406", format!("this works out to {} `{}`, and it is being given to {} `{}`.", found.article(), found.name(), ty.article(), ty.name()))
                        .primary(value.span, format!("{} `{}`", found.article(), found.name()))
                        .secondary(ty_span, format!("declared `{}` here", ty.name()))
                        .rule("nothing converts on its own — two types meet only where something says they should")
                        .fix("declare it the same type"),
                );
                return None;
            }
            return Some(built);
        }

        // No operators written: the pieces sit side by side, which builds text.
        if !value.has_operators() {
            return self.juxtaposed(value, ty, ty_span);
        }

        // Operators were written, so this is arithmetic and juxtaposition has no meaning
        // in it. Saying which of the two was probably meant is more use than "wrong".
        if value.between.iter().any(Option::is_none) {
            self.errors.push(
                Diagnostic::new("E0414", "some of these are joined and some are added.")
                    .primary(value.span, "here")
                    .rule("pieces side by side build text; an operator between them works something out")
                    .tip("a value does one or the other, not both.")
                    .fix("put an operator between all of them, or none of them"),
            );
            return None;
        }

        // The chain says what its numbers are, which is the same rule that makes
        // `*1000*` a number under `i64` and four characters under `str`.
        let outer = std::mem::replace(&mut self.reading, match ty {
            Ty::Exact => Ty::Exact,
            Ty::Decimal { digits } => Ty::Decimal { digits: *digits },
            Ty::F64 => Ty::F64,
            Ty::F32 => Ty::F32,
            Ty::F16 => Ty::F16,
            Ty::Int { bits, signed } => Ty::Int { bits: *bits, signed: *signed },
            _ => Ty::Int { bits: 64, signed: true },
        });
        let built = self.tree(value);
        self.reading = outer;
        let built = built?;
        let found = self.type_of(&built, value.span)?;
        if &found != ty {
            self.errors.push(
                Diagnostic::new("E0406", format!("this works out to {} `{}`, and it is being given to {} `{}`.", found.article(), found.name(), ty.article(), ty.name()))
                    .primary(value.span, format!("{} `{}`", found.article(), found.name()))
                    .secondary(ty_span, format!("declared `{}` here", ty.name()))
                    .rule("nothing converts on its own — two types meet only where something says they should")
                    .fix("declare it the same type"),
            );
            return None;
        }
        Some(built)
    }

    /// Pieces side by side, which is how text is built.
    fn juxtaposed(&mut self, value: &ast::Value, ty: &Ty, ty_span: Span) -> Option<Value> {
        // A value that says its own type is read by that type and then checked against
        // the one that was wanted. Where the chain also says it this is a repetition
        // rather than a mistake -- and it is not always a repetition, since a call's
        // argument may be going into a hole, which says nothing at all.
        if let [one @ ast::Term::Piece(ast::Piece::Written { ty: Some(_), .. })] =
            value.terms.as_slice()
        {
            let built = self.term(one)?;
            let found = self.type_of(&built, value.span)?;
            if !ty.accepts(&found) {
                self.errors.push(
                    Diagnostic::new("E0406", format!("this says it is {} `{}`, and it is being given to {} `{}`.", found.article(), found.name(), ty.article(), ty.name()))
                        .primary(value.span, format!("{} `{}`", found.article(), found.name()))
                        .secondary(ty_span, format!("declared `{}` here", ty.name()))
                        .rule("nothing converts on its own — two types meet only where something says they should")
                        .fix(format!("say `{}`, or leave the type off and let the chain say it", ty.name())),
                );
                return None;
            }
            return Some(built);
        }

        // A value that is one name is that variable's value, whatever the type.
        if let [ast::Term::Piece(ast::Piece::Name(span))] = value.terms.as_slice() {
            let (built, held) = self.named_value(*span)?;

            // An array is the only thing a second name can reach, so it is the only
            // thing that has to say which of the two was meant. Sharing costs nothing
            // and lets a change here be seen there; copying costs an allocation and the
            // whole of the array. Neither should be paid by an omission.
            if matches!(held, Ty::Arr { .. }) {
                let name = self.named(*span);
                self.errors.push(
                    Diagnostic::new("E0478", format!("this does not say whether it shares `'{name}'` or copies it."))
                        .primary(*span, "here")
                        .rule("naming an array in a value says `share` or `copy`, and silence is not one of them")
                        .tip("`share` makes a second name for one array, so a change through either is seen through both. `copy` makes a second array, and pays for it.")
                        .fix(format!("`[share '{name}']`, or `[copy '{name}']`")),
                );
                return None;
            }

            if &held != ty {
                self.errors.push(
                    Diagnostic::new("E0406", format!("this is {} `{}`, and it is being given to {} `{}`.", held.article(), held.name(), ty.article(), ty.name()))
                        .primary(*span, format!("{} `{}`", held.article(), held.name()))
                        .secondary(ty_span, format!("declared `{}` here", ty.name()))
                        .rule("nothing converts on its own — two types meet only where something says they should")
                        .fix("declare it the same type, or write the value out"),
                );
                return None;
            }
            return Some(built);
        }
        // A single bracketed value is just that value.
        if let [ast::Term::Group { value: inner, .. }] = value.terms.as_slice() {
            return self.value(inner, ty, ty_span);
        }

        // An array: its elements, juxtaposed, flat however many dimensions it has.
        if let Ty::Arr { of, shape, length } = ty {
            return self.array(value, of, shape, *length, ty_span);
        }

        // Exactly one thing that is not a written value -- an index, a call, brackets.
        // Every arm below reads a *written* value under the chain's type, and none of
        // these is written: whatever it is, it already knows what it is, so the only
        // question left is whether that agrees with the chain.
        if !matches!(ty, Ty::Str | Ty::Arr { .. })
            && let [one] = value.terms.as_slice()
            && !matches!(one, ast::Term::Piece(ast::Piece::Written { .. }))
        {
            let built = self.term(one)?;
            let found = self.type_of(&built, one.span())?;
            if &found != ty {
                self.errors.push(
                    Diagnostic::new("E0406", format!("this is {} `{}`, and it is being given to {} `{}`.", found.article(), found.name(), ty.article(), ty.name()))
                        .primary(one.span(), format!("{} `{}`", found.article(), found.name()))
                        .secondary(ty_span, format!("declared `{}` here", ty.name()))
                        .rule("nothing converts on its own — two types meet only where something says they should")
                        .fix("declare it the same type"),
                );
                return None;
            }
            return Some(built);
        }

        match ty {
            Ty::Arr { .. } => unreachable!("handled above"),
            // A hole holds whatever the caller put in it, so nothing may be *written*
            // at one -- `*1*` is a number under `i64` and four characters under `str`,
            // and a hole has not said which it is. What does work is naming something
            // already of that type, which is every use a body has for one.
            Ty::Hole(_) => match value.terms.as_slice() {
                [] => None,
                [one] => {
                    let built = self.term(one)?;
                    let found = self.type_of(&built, one.span())?;
                    if !ty.accepts(&found) {
                        self.errors.push(
                            Diagnostic::new("E0406", format!("this is {} `{}`, and it is being given to {} `{}`.", found.article(), found.name(), ty.article(), ty.name()))
                                .primary(one.span(), format!("{} `{}`", found.article(), found.name()))
                                .secondary(ty_span, format!("declared `{}` here", ty.name()))
                                .rule("a hole is one type, the same one everywhere in a signature — and the caller decides which")
                                .fix(format!("name something the function was given, or declare it `{}`", found.name())),
                        );
                        return None;
                    }
                    Some(built)
                }
                terms => {
                    let all = terms[0].span().to(terms[terms.len() - 1].span());
                    self.errors.push(
                        Diagnostic::new("E0408", format!("{} `{}` is one value, not several.", ty.article(), ty.name()))
                            .primary(all, format!("{} pieces here", terms.len()))
                            .secondary(ty_span, format!("declared `{}` here", ty.name()))
                            .rule("pieces side by side build text, and a hole is not known to be text")
                            .fix("write it as one value"),
                    );
                    None
                }
            },
            // `*1.5*`, `*-0.25*`, `*3*`. A decimal point here is the nearest value of
            // this width to what was written, which is what a binary float is and why
            // `e` exists.
            Ty::F64 | Ty::F32 | Ty::F16 => match value.terms.as_slice() {
                [ast::Term::Piece(ast::Piece::Written { ty: None, mark })] => {
                    self.a_float(&unmarked(self.text(*mark)), ty, *mark)
                }
                _ => {
                    self.errors.push(
                        Diagnostic::new("E0487", format!("a `{}` is one written value, not several.", ty.name()))
                            .primary(value.span, "here")
                            .secondary(ty_span, format!("declared `{}` here", ty.name()))
                            .rule("pieces side by side build text; a number is written once")
                            .fix("write it as one value, or put an operator between them"),
                    );
                    None
                }
            },
            // `*1.5*`, `*0.1*`, `*3*`. A decimal point is exact *in this many digits*:
            // one tenth is one tenth, and a third is not a third.
            Ty::Decimal { digits } => match value.terms.as_slice() {
                [ast::Term::Piece(ast::Piece::Written { ty: None, mark })] => {
                    self.a_decimal(&unmarked(self.text(*mark)), *digits, *mark)
                }
                _ => {
                    self.errors.push(
                        Diagnostic::new("E0487", format!("a `{}` is one written value, not several.", ty.name()))
                            .primary(value.span, "here")
                            .secondary(ty_span, format!("declared `{}` here", ty.name()))
                            .rule("pieces side by side build text; a number is written once")
                            .fix("write it as one value, or put an operator between them"),
                    );
                    None
                }
            },
            // `*12*`, `*-3/4*`, `*0.1*`. A decimal point is exact here, which is the
            // whole reason to write one: `0.1` is one tenth, not the `b64` nearest it.
            Ty::Exact => match value.terms.as_slice() {
                [ast::Term::Piece(ast::Piece::Written { ty: None, mark })] => {
                    let written = unmarked(self.text(*mark));
                    match quench_num::Exact::parse(&written) {
                        Some(_) => Some(Value::Exact(written)),
                        None => {
                            self.errors.push(
                                Diagnostic::new("E0474", format!("`{written}` is not an exact number."))
                                    .primary(*mark, "here")
                                    .rule("an `e` is written `*12*`, `*-3/4*` or `*0.1*`, and all three are exact")
                                    .tip("`*0.1*` is one tenth here, rather than the nearest a `b64` gets to it.")
                                    .fix("write a whole number, a ratio, or a decimal"),
                            );
                            None
                        }
                    }
                }
                _ => {
                    self.errors.push(
                        Diagnostic::new("E0475", "an `e` is one written value, not several.")
                            .primary(value.span, "here")
                            .secondary(ty_span, "declared `e` here")
                            .rule("pieces side by side build text; a number is written once")
                            .fix("write it as one value, or put an operator between them"),
                    );
                    None
                }
            },
            Ty::Str => {
                // Pieces side by side, which is what juxtaposition means everywhere in
                // the language. A run of them that is entirely known here stays one
                // piece of text; a name among them makes it something joined while the
                // program runs, which is the same thing said at a different time.
                let mut pieces: Vec<Value> = Vec::new();
                let mut so_far = String::new();
                for term in &value.terms {
                    if let ast::Term::Piece(ast::Piece::Name(_))
                    | ast::Term::Piece(ast::Piece::Path(_))
                    | ast::Term::Piece(ast::Piece::At { .. })
                    | ast::Term::Piece(ast::Piece::Call(_))
                    | ast::Term::At { .. }
                    | ast::Term::Call(_) = term
                    {
                        let built = self.term(term)?;
                        let found = self.type_of(&built, term.span())?;
                        if found != Ty::Str {
                            self.errors.push(
                                Diagnostic::new("E0411", format!("this is {} `{}`, and text is made of text.", found.article(), found.name()))
                                    .primary(term.span(), format!("{} `{}`", found.article(), found.name()))
                                    .secondary(ty_span, "declared `str` here")
                                    .rule("pieces side by side join, and nothing converts on its own")
                                    .tip("a `print` shows any type because showing is not joining -- it writes one piece after another and builds nothing.")
                                    .fix("declare it `str`, or print the pieces separately"),
                            );
                            return None;
                        }
                        if !so_far.is_empty() {
                            pieces.push(Value::Text(std::mem::take(&mut so_far)));
                        }
                        pieces.push(built);
                        continue;
                    }
                    let ast::Term::Piece(piece) = term else {
                        self.errors.push(
                            Diagnostic::new("E0415", "brackets group something to work out, and text is not worked out.")
                                .primary(term.span(), "here")
                                .rule("text is a list of pieces, written side by side")
                                .fix("remove the brackets"),
                        );
                        return None;
                    };
                    so_far.push_str(&self.literal(piece)?);
                }
                if pieces.is_empty() {
                    return Some(Value::Text(so_far));
                }
                if !so_far.is_empty() {
                    pieces.push(Value::Text(so_far));
                }
                Some(Value::Join(pieces))
            }
            Ty::Bool => match value.terms.as_slice() {
                [ast::Term::Piece(ast::Piece::Written { ty: None, mark })] => {
                    match unmarked(self.text(*mark)).as_str() {
                        "true" => Some(Value::Bool(true)),
                        "false" => Some(Value::Bool(false)),
                        other => {
                            self.errors.push(
                                Diagnostic::new("E0416", format!("`{other}` is not true or false."))
                                    .primary(*mark, "here")
                                    .rule("a `bool` is written `*true*` or `*false*`, and nothing is truthy")
                                    .fix("`*true*` or `*false*`, or a comparison"),
                            );
                            None
                        }
                    }
                }
                _ => {
                    self.errors.push(
                        Diagnostic::new("E0417", "a `bool` is one thing, not several.")
                            .primary(value.span, "here")
                            .rule("`*true*`, `*false*`, or something that compares two values")
                            .fix("write one of those"),
                    );
                    None
                }
            },
            Ty::Int { .. } => match value.terms.as_slice() {
                [ast::Term::Piece(ast::Piece::Written { ty: None, mark })] => {
                    self.a_whole_number(&unmarked(self.text(*mark)), ty, *mark)
                }
                [] => None,
                // Exactly one thing that is not a written value -- a bare number, an
                // index, brackets. Whatever it is, `term` knows what is wrong with it
                // specifically, and "not several" would be a poor description of one.
                [one] => {
                    let built = self.term(one)?;
                    let found = self.type_of(&built, one.span())?;
                    if !matches!(found, Ty::Int { .. }) {
                        self.errors.push(
                            Diagnostic::new("E0406", format!("this is {} `{}`, and it is being given to an `i64`.", found.article(), found.name()))
                                .primary(one.span(), format!("{} `{}`", found.article(), found.name()))
                                .secondary(ty_span, "declared `i64` here")
                                .rule("nothing converts on its own — two types meet only where something says they should")
                                .fix("declare it the same type"),
                        );
                        return None;
                    }
                    Some(built)
                }
                terms => {
                    let all = terms[0].span().to(terms[terms.len() - 1].span());
                    self.errors.push(
                        Diagnostic::new("E0408", "a number is one written value, not several.")
                            .primary(all, format!("{} pieces here", terms.len()))
                            .secondary(ty_span, "declared `i64` here")
                            .rule("pieces side by side build text; a number is written once")
                            .tip("`str` is the type where a value is a list of pieces.")
                            .fix("write it as one value, or put `+` between them"),
                    );
                    None
                }
            },
        }
    }

    /// The elements of an array: `[[*1* *2* *3*]]`.
    ///
    /// Written flat however many dimensions the shape has, because the type already said
    /// the shape and writing it twice would only be a chance to disagree.
    fn array(
        &mut self,
        value: &ast::Value,
        of: &Ty,
        shape: &[usize],
        length: Length,
        ty_span: Span,
    ) -> Option<Value> {
        // A length nobody here knows is a length nobody here can write elements for.
        // An `(any)` array is one that arrived; `grow` is the word for one that starts
        // empty and is filled. Asked of what lies under it as well, because `[[]]` on an
        // `arr.arr.i64 (2 any)` would otherwise make two arrays nothing can ever fill.
        if length == Length::Unknown || unsaid(of) {
            self.errors.push(
                Diagnostic::new("E0510", "an array whose length is `any` is one that arrived, not one written here.")
                    .primary(value.span, "here")
                    .secondary(ty_span, "declared `any` here")
                    .rule("`any` says the number was never said, so nothing here knows how many elements to expect")
                    .tip("`grow` is the word for an array that starts empty and is filled, and it may be added to.")
                    .fix("say a number, or `grow`, or name an array that was handed in"),
            );
            return None;
        }

        // A group cut out of a longer run is already the elements; only the whole
        // value wears the brackets that say so.
        let cut: Vec<ast::Term>;
        let (written, open, close) = match value.terms.as_slice() {
            [ast::Term::Elements { of, open, close }] => (of.as_slice(), *open, *close),
            terms if terms.len() > 1 || matches!(terms.first(), Some(ast::Term::Piece(_))) => {
                cut = terms.to_vec();
                (cut.as_slice(), value.span, value.span)
            }
            _ => (&[][..], value.span, value.span),
        };
        let (open, close) = (&open, &close);
        if written.is_empty() && !matches!(value.terms.as_slice(), [ast::Term::Elements { .. }]) {
            self.errors.push(
                Diagnostic::new("E0430", "an array is written between brackets.")
                    .primary(value.span, "here")
                    .rule("the elements go in a list of their own, inside the value")
                    .fix("`[[*1* *2* *3*]]`"),
            );
            return None;
        }

        // Flat, however many allocations deep it is, since the type already gave the
        // shape. `arr.arr.i64 (2 3)` is six numbers, grouped three and three.
        // What one element takes when it is written out, and what the whole allocation
        // takes: a fixed one wants exactly that many, a growing one wants a multiple.
        let each = of.count();
        let all = each * shape.iter().product::<usize>();

        // Nothing under this said how big it is, so nothing says where one row of it
        // ends. Such an array is written empty and filled afterwards -- and a fixed
        // number of growing rows starts as that many empty ones.
        if !of.settled() {
            if !written.is_empty() {
                self.errors.push(
                    Diagnostic::new("E0484", "nothing here says where one row ends.")
                        .primary(open.to(*close), "here")
                        .secondary(ty_span, "what this holds says `grow`")
                        .rule("elements are written flat and cut into rows, and a row of something that grows has no length to cut at")
                        .tip("`add` is how one of these is filled, one array at a time.")
                        .fix("write it empty, as `[[]]`"),
                );
                return None;
            }
            let rows = if length.known() { shape.iter().product::<usize>() } else { 0 };
            let Ty::Arr { of: under, .. } = of else {
                unreachable!("only an array holds something that is not settled")
            };
            let empty = Value::Array { of: under.clone(), elements: Vec::new() };
            return Some(Value::Array { of: Box::new(of.clone()), elements: vec![empty; rows] });
        }

        // A growing allocation takes however many were written, so long as they fill
        // whole rows of whatever lies under it. A fixed one takes exactly its size.
        if !length.known() {
            if all == 0 || written.len() % all != 0 {
                self.errors.push(
                    Diagnostic::new("E0481", format!(
                        "this grows in rows of {}, and {} were written.",
                        counted(all, "element"),
                        counted(written.len(), "element")
                    ))
                    .primary(open.to(*close), format!("{} here", counted(written.len(), "element")))
                    .secondary(ty_span, "declared `grow`")
                    .rule("a growing array holds whole rows of whatever is under it, however many rows it has")
                    .tip("`(grow grow)` can only be written empty, because nothing says where one row ends.")
                    .fix(format!("write a multiple of {}", all)),
                );
                return None;
            }
        } else if written.len() != all {
            let wanted = all;
            let sizes: Vec<String> = shape.iter().map(usize::to_string).collect();
            self.errors.push(
                Diagnostic::new("E0431", format!(
                    "this holds {} element(s), and {} were written.",
                    wanted,
                    written.len()
                ))
                .primary(open.to(*close), format!("{} here", written.len()))
                .secondary(ty_span, format!("declared ({})", sizes.join(" ")))
                .rule("a shaped array is written flat, row by row, since the type already gave the shape")
                .fix(if written.len() < wanted { "write the missing ones" } else { "remove the extra ones" }),
            );
            return None;
        }

        // An array of arrays is built out of arrays, so the flat run is cut into
        // groups of what one inner allocation holds and each group becomes one.
        if let Ty::Arr { of: inner, shape: inner_shape, length: inner_length } = of {
            let mut rows = Vec::with_capacity(written.len() / each);
            for group in written.chunks(each) {
                let piece = ast::Value {
                    terms: group.to_vec(),
                    between: vec![None; group.len().saturating_sub(1)],
                    span: group
                        .first()
                        .map(|t| t.span().to(group.last().expect("not empty").span()))
                        .unwrap_or(ty_span),
                };
                let built = self.array(&piece, inner, inner_shape, *inner_length, ty_span)?;
                rows.push(built);
            }
            return Some(Value::Array { of: Box::new(of.clone()), elements: rows });
        }

        let mut elements = Vec::with_capacity(written.len());
        for term in written {
            // Each element is read *under* the element type, which is the same rule
            // that makes `*0.1*` one tenth under `e` and a mistake under `i64`. The
            // array's chain is what says which, exactly as a declaration's does.
            let outer = std::mem::replace(&mut self.reading, of.clone());
            let built = match (of, term) {
                (Ty::Str, ast::Term::Piece(piece)) => {
                    self.literal(piece).map(Value::Text)
                }
                _ => self.term(term),
            };
            self.reading = outer;
            let built = built?;
            let found = self.type_of(&built, term.span())?;
            if &found != of {
                self.errors.push(
                    Diagnostic::new("E0432", format!("this is {} `{}`, and the array holds {} `{}`.", found.article(), found.name(), of.article(), of.name()))
                        .primary(term.span(), format!("{} `{}`", found.article(), found.name()))
                        .secondary(ty_span, format!("declared `arr.{}` here", of.name()))
                        .rule("every element of an array is the type the array said, and nothing converts on its own")
                        .fix(format!("write a `{}`", of.name())),
                );
                return None;
            }
            elements.push(built);
        }
        Some(Value::Array { of: Box::new(of.clone()), elements })
    }

    /// `'xs'[…]` — one element.
    /// `'xs'[…]` — an index, always. A call says `call`.
    ///
    /// The parser hands the bracket's contents over as a comma-separated list, because
    /// that is the one shape that covers both this and a call's arguments. An index
    /// writes its dimensions side by side instead, so there is exactly one value here
    /// however many dimensions it has, and the commas are what this refuses.
    fn reached(&mut self, name: Span, given: &[ast::Value], close: Span) -> Option<Value> {
        let [one] = given else {
            if given.is_empty() {
                return self.at(name, &[], close);
            }
            let all = given[0].span.to(given[given.len() - 1].span);
            self.errors.push(
                Diagnostic::new("E0491", "an index writes its dimensions side by side.")
                    .primary(all, "commas here")
                    .secondary(name, "indexing this")
                    .rule("a shape is written `(2 3)` and an index into it is written the same way")
                    .tip("commas separate the arguments of a call, and this names no function.")
                    .fix("take the commas out"),
            );
            return None;
        };
        if one.between.iter().any(Option::is_some) {
            self.errors.push(
                Diagnostic::new("E0491", "an index is a number, not something to work out.")
                    .primary(one.span, "here")
                    .secondary(name, "indexing this")
                    .rule("a shape is written `(2 3)` and an index into it is written the same way")
                    .tip("brackets make one value out of a sum, and an index takes one value per dimension.")
                    .fix("put the sum in brackets of its own"),
            );
            return None;
        }
        self.at(name, &one.terms, close)
    }

    /// `call exp['x']` — the maths IEEE 754 only recommends, worked out here.
    ///
    /// A `b64` and nothing narrower. Working the answer out for a `b64` and then rounding
    /// that down to a `b32` rounds twice, and a value that sits just the wrong side of a
    /// `b32` boundary comes out one step off — rarely, silently, and in both engines at
    /// once. Doing it properly means rounding straight from the wide value to the narrow
    /// format, which is a thing to build rather than a thing to assume.
    fn slowly(&mut self, call: &ast::Call, name: &str, wants: usize, which: u8) -> Option<Value> {
        if call.args.len() != wants {
            self.errors.push(
                Diagnostic::new("E0494", format!("`{name}` takes {}.", counted(wants, "number")))
                    .primary(call.word.to(call.close), format!("given {}", counted(call.args.len(), "thing")))
                    .rule("a call brings one value for each thing it takes, in the same order")
                    .fix(format!("give it {}", counted(wants, "number"))),
            );
            return None;
        }
        let mut built = Vec::with_capacity(wants);
        for given in &call.args {
            let outer = std::mem::replace(&mut self.reading, Ty::F64);
            let value = self.tree(given);
            self.reading = outer;
            let value = value?;
            match self.type_of(&value, given.span)? {
                Ty::F64 => built.push(value),
                other => {
                    let narrow = matches!(other, Ty::F32 | Ty::F16);
                    self.errors.push(
                        Diagnostic::new("E0495", format!("`{name}` works on a `b64`, and this is {} `{}`.", other.article(), other.name()))
                            .primary(given.span, format!("{} `{}`", other.article(), other.name()))
                            .rule("IEEE 754 only recommends this one, so Quench works it out itself rather than asking a library — and it does that at one width")
                            .tip(if narrow {
                                "working it out for a `b64` and rounding that down would round twice, which is once too many."
                            } else {
                                "`+`, `-`, `x`, `/`, `sqrt` and the rest work on every width, because the standard settles those."
                            })
                            .fix("use a `b64`"),
                    );
                    return None;
                }
            }
        }
        Some(Value::Slowly { which, of: built })
    }

    /// `call sqrt['x']` — the maths IEEE 754 requires, on whatever width it was given.
    ///
    /// Every argument is a float and they are all the same width, which is also the
    /// width of the answer. A `b16` goes through as the `f32` it is carried in and is
    /// put back afterwards, exactly as `+` on one is.
    /// `call is.i64['line']` and `call as.i64['line']`.
    ///
    /// The two are one function asked two ways, which is deliberate and is the whole of
    /// how Quench fails. There is no value that is either an answer or a reason — that
    /// wants a type the language has not got — so instead `is` is the question and `as`
    /// is the answer, and a program that asks the first before the second never reaches
    /// a trap. A program that does not ask is a program with a mistake in it, and it
    /// stops, exactly like an index off the end of an array.
    ///
    /// What each type accepts is what a written value of that type accepts, because
    /// both go through [`quench_num::read`]. `*12*` is an `i64` in a source file and
    /// `12` is an `i64` here, and there is one implementation deciding it.
    fn reads(&mut self, call: &ast::Call, sure: bool) -> Option<Value> {
        let word = if sure { "as" } else { "is" };

        let [link] = call.chain.as_slice() else {
            let saying = if call.chain.is_empty() {
                format!("`{word}` says which type it is about.")
            } else {
                format!("`{word}` is about one type, and this names {}.", call.chain.len())
            };
            self.errors.push(
                Diagnostic::new("E0496", saying)
                    .primary(call.word.to(call.close), "here")
                    .rule(format!("`{word}` carries the type on its chain, one link and no more"))
                    .tip("text says nothing about what it holds — `12` is an `i64`, a `b64` and an `e` — so the type is asked for rather than worked out.")
                    .fix(format!("`call {word}.i64['line']`")),
            );
            return None;
        };

        let named = self.text(*link).to_string();
        let Some(ty) = Ty::simple(&named) else {
            let all: Vec<String> = Ty::NAMES.iter().map(|name| format!("`{name}`")).collect();
            self.errors.push(
                Diagnostic::new("E0496", format!("there is no type called `{named}`."))
                    .primary(*link, "here")
                    .rule(format!("the link after `{word}` names one of the types that is one word"))
                    .tip(format!("they are {}.", all.join(", ")))
                    .fix(format!("`call {word}.i64['line']`")),
            );
            return None;
        };
        if ty == Ty::Str {
            self.errors.push(
                Diagnostic::new("E0497", format!("`{word}.str` reads text out of text."))
                    .primary(*link, "here")
                    .rule("these read something that is not text, out of text")
                    .tip("`stitch` goes the other way, and every type may go through it.")
                    .fix("name a number type or `bool`"),
            );
            return None;
        }

        let [one] = call.args.as_slice() else {
            self.errors.push(
                Diagnostic::new("E0498", format!("`{word}` reads one piece of text."))
                    .primary(call.word.to(call.close), format!("given {}", counted(call.args.len(), "thing")))
                    .rule(format!("`{word}` takes one `str`, and nothing else"))
                    .fix(format!("`call {word}.{named}['line']`")),
            );
            return None;
        };

        // Built as a `str` is built anywhere else, which is what makes pieces side by
        // side join here too: `call as.i64['first' 'second']` reads one number out of
        // both, and a written value needs no `str:` in front of it because the only
        // thing this takes is text.
        let text = Box::new(self.value(one, &Ty::Str, call.name)?);
        Some(if sure { Value::Read { ty, text } } else { Value::CanRead { ty, text } })
    }

    /// `call text.slice['s', *2*, *4*]` and its four neighbours.
    ///
    /// Every one of them takes text and known types, so the arguments go through the
    /// ordinary builder rather than a reading of their own — which is what lets a
    /// written value stand where a `str` is wanted without saying `str:` first.
    /// `call input.line[]` and its three neighbours.
    ///
    /// `[]` is written even though there is nothing to write in it, the way a function
    /// that takes nothing writes it: *takes nothing* is a thing to say, and saying it by
    /// leaving the brackets off would be saying it with an absence.
    fn given(&mut self, call: &ast::Call, name: &str, which: u8) -> Option<Value> {
        if !call.args.is_empty() {
            self.errors.push(
                Diagnostic::new("E0521", format!("`input.{name}` takes nothing."))
                    .primary(call.name.to(call.close), format!("given {}", counted(call.args.len(), "thing")))
                    .rule("what arrived from outside arrived before the program did, so there is nothing to ask it for")
                    .fix(format!("`call input.{name}[]`")),
            );
            return None;
        }
        Some(Value::Given { which })
    }

    fn pieces(&mut self, call: &ast::Call, name: &str, which: u8) -> Option<Value> {
        let whole = Ty::Int { bits: 64, signed: true };
        let wants: Vec<Ty> = match which {
            0 => vec![Ty::Str, whole.clone(), whole],
            4 => vec![Ty::Str],
            _ => vec![Ty::Str, Ty::Str],
        };
        let shape = match which {
            0 => format!("`call text.{name}['s', *2*, *4*]`"),
            4 => format!("`call text.{name}['s']`"),
            _ => format!("`call text.{name}['s', 'in it']`"),
        };
        if call.args.len() != wants.len() {
            self.errors.push(
                Diagnostic::new("E0521", format!("`text.{name}` takes {}.", counted(wants.len(), "thing")))
                    .primary(call.name.to(call.close), format!("given {}", counted(call.args.len(), "thing")))
                    .rule("a call brings one value for each thing it takes, in the same order")
                    .fix(shape),
            );
            return None;
        }

        let mut built = Vec::with_capacity(wants.len());
        for (given, ty) in call.args.iter().zip(&wants) {
            built.push(self.value(given, ty, call.name)?);
        }
        Some(Value::Pieces { which, of: built })
    }

    fn maths(&mut self, call: &ast::Call, name: &str, wants: usize, which: u8) -> Option<Value> {
        if call.args.len() != wants {
            self.errors.push(
                Diagnostic::new("E0494", format!("`{name}` takes {}.", counted(wants, "number")))
                    .primary(call.word.to(call.close), format!("given {}", counted(call.args.len(), "thing")))
                    .rule("a call brings one value for each thing it takes, in the same order")
                    .fix(format!("give it {}", counted(wants, "number"))),
            );
            return None;
        }

        let mut built = Vec::with_capacity(wants);
        let mut width: Option<u8> = None;
        for given in &call.args {
            let outer = std::mem::replace(&mut self.reading, Ty::F64);
            let value = self.tree(given);
            self.reading = outer;
            let value = value?;
            let found = self.type_of(&value, given.span)?;
            let each = match found {
                Ty::F64 => 64u8,
                Ty::F32 => 32,
                Ty::F16 => 16,
                other => {
                    self.errors.push(
                        Diagnostic::new("E0494", format!("`{name}` works on binary floats, and this is {} `{}`.", other.article(), other.name()))
                            .primary(given.span, format!("{} `{}`", other.article(), other.name()))
                            .rule("the maths IEEE 754 settles is about `b16`, `b32` and `b64`, which are the types it settles")
                            .tip("`e` never rounds, so it has no need of these, and a decimal's are a different specification.")
                            .fix("use a `b64`, a `b32` or a `b16`"),
                    );
                    return None;
                }
            };
            match width {
                None => width = Some(each),
                Some(first) if first != each => {
                    self.errors.push(
                        Diagnostic::new("E0494", format!("`{name}` takes one width, and was given two."))
                            .primary(given.span, format!("{} `{}`", found.article(), found.name()))
                            .rule("nothing converts on its own, so two float widths never meet")
                            .fix("make them the same width"),
                    );
                    return None;
                }
                Some(_) => {}
            }
            built.push(value);
        }
        Some(Value::Maths { which, of: built, width: width.expect("at least one argument") })
    }

    /// `call stitch[*n is * 'n']` — the text of all of it, joined.
    ///
    /// The list is a `print`'s: pieces side by side, of whatever types. What it adds
    /// over plain juxtaposition is the one conversion Quench has — text beside a number
    /// is refused, because nothing converts on its own, and this is how a program says
    /// *do it anyway*. The word being written is what makes that a request rather than
    /// a guess, and it is the whole reason there is a word at all.
    fn stitched(&mut self, call: &ast::Call) -> Option<Value> {
        let [list] = call.args.as_slice() else {
            self.errors.push(
                Diagnostic::new("E0493", "`stitch` takes one list, written side by side.")
                    .primary(call.word.to(call.close), format!("{} here", counted(call.args.len(), "list")))
                    .rule("pieces beside each other build one value, so commas have nothing to separate")
                    .tip("it is the list a `print` takes, and it is read the same way.")
                    .fix("take the commas out"),
            );
            return None;
        };
        if list.between.iter().any(Option::is_some) {
            self.errors.push(
                Diagnostic::new("E0493", "`stitch` joins its pieces rather than working them out.")
                    .primary(list.span, "an operator here")
                    .rule("pieces beside each other build one value, and an operator between two of them is arithmetic")
                    .fix("put the sum in brackets of its own"),
            );
            return None;
        }

        // Read under `str`, so a written value with no type in front of it is the
        // characters it was written with -- which is what it is in a `print` too.
        let outer = std::mem::replace(&mut self.reading, Ty::Str);
        let built = self.stitching(list);
        self.reading = outer;
        built
    }

    fn stitching(&mut self, list: &ast::Value) -> Option<Value> {
        let mut pieces: Vec<Value> = Vec::new();
        let mut so_far = String::new();
        for term in &list.terms {
            // A written value with no type, or an escape, is known here and now, so a
            // run of them stays one piece rather than becoming a join while it runs.
            if let ast::Term::Piece(
                piece @ (ast::Piece::Written { ty: None, .. } | ast::Piece::Escape(_)),
            ) = term
            {
                so_far.push_str(&self.literal(piece)?);
                continue;
            }
            let value = self.term(term)?;
            let found = self.type_of(&value, term.span())?;
            if !so_far.is_empty() {
                pieces.push(Value::Text(std::mem::take(&mut so_far)));
            }
            pieces.push(match found {
                Ty::Str => value,
                ty => Value::Said { of: Box::new(value), ty },
            });
        }
        if pieces.is_empty() {
            return Some(Value::Text(so_far));
        }
        if !so_far.is_empty() {
            pieces.push(Value::Text(so_far));
        }
        Some(Value::Join(pieces))
    }

    /// A call to a function the writer declared.
    fn called(&mut self, which: u32, call: &ast::Call) -> Option<Value> {
        let (args, fill) = self.arguments(which, call.name, &call.args, call.close)?;
        if self.signatures[which as usize].returns.is_none() {
            let said = self.signatures[which as usize].said.clone();
            let at = self.signatures[which as usize].at;
            self.errors.push(
                Diagnostic::new("E0471", format!("`'{said}'` gives `nothing` back, and this wants a value."))
                    .secondary(at, "declared `nothing` here")
                    .primary(call.word.to(call.close), "and its answer is wanted here")
                    .rule("`nothing` means there is no answer, and there is no value to stand in for one")
                    .fix("call it on its own line, or have it give something back"),
            );
            return None;
        }
        Some(Value::Call { func: which, args, fill })
    }

    fn at(&mut self, name: Span, indices: &[ast::Term], close: Span) -> Option<Value> {
        // A constant array has somewhere it lives — the module's tables — so it is
        // indexed like any other. What it does not have is a way to be changed, and
        // that is `set`'s business rather than this one's.
        let (base, held) = self.named_value(name)?;
        let declared = match &base {
            Value::Copy(local) => self.locals[local.0 as usize].at,
            Value::Const(which) => self.constants[*which as usize].at,
            _ => name,
        };
        if !matches!(held, Ty::Arr { .. }) {
            self.errors.push(
                Diagnostic::new("E0433", format!("`{}` is not an array.", held.name()))
                    .primary(name, format!("{} `{}`", held.article(), held.name()))
                    .rule("only an array has elements to index")
                    .fix("index an array, or use the value on its own"),
            );
            return None;
        }

        // Every `arr` link is one allocation, so the indices are spent on them in
        // order: the outer link takes its dimensions, then whatever is left goes to
        // what it holds. Stopping at an allocation boundary hands back the array
        // that lives there, which is the whole reason to write two links.
        let mut levels = Vec::new();
        let mut walking = held.clone();
        let mut spent = 0;
        while let Ty::Arr { of, shape, length } = walking {
            if spent == indices.len() {
                break;
            }
            let mut dimensions = shape.clone();
            let unsaid = !length.known();
            if unsaid {
                // The one whose size was not said is outermost and takes an index like
                // any other; what it does not do is take part in a stride.
                dimensions.insert(0, 0);
            }
            spent += dimensions.len();
            levels.push((dimensions, unsaid));
            walking = *of;
        }

        if spent != indices.len() {
            let stops: Vec<String> = boundaries(&held).iter().map(usize::to_string).collect();
            self.errors.push(
                Diagnostic::new("E0434", format!(
                    "this takes {} index(es), and {} were given.",
                    stops.join(" or "),
                    indices.len()
                ))
                .primary(name.to(close), format!("{} here", indices.len()))
                .secondary(declared, format!("declared `{}`", held.name()))
                .rule("an index gives one number for each dimension, and may stop where one allocation ends")
                .tip("stopping early hands back the array that lives there, rather than an element of it.")
                .fix(format!("give {} of them", stops.join(" or "))),
            );
            return None;
        }

        // An index is counted, so it is read as a whole number wherever it is written.
        // Without this it is read as whatever the chain around it said, and
        // `['xs'[*1*] + 'xs'[*2*]]` under a `b64` chain refuses its own `*1*`.
        let outer =
            std::mem::replace(&mut self.reading, Ty::Int { bits: 64, signed: true });
        let mut built = Vec::with_capacity(indices.len());
        for index in indices {
            let value = self.term(index);
            let Some(value) = value else {
                self.reading = outer;
                return None;
            };
            let Some(found) = self.type_of(&value, index.span()) else {
                self.reading = outer;
                return None;
            };
            if !matches!(found, Ty::Int { .. }) {
                self.errors.push(
                    Diagnostic::new("E0435", format!("an index is a number, and this is {} `{}`.", found.article(), found.name()))
                        .primary(index.span(), "here")
                        .rule("an element is found by counting, and counting is done with numbers")
                        .fix("use a whole number"),
                );
                self.reading = outer;
                return None;
            }
            built.push(value);
        }
        self.reading = outer;

        // One `At` per allocation walked through, which is one `array-get` each. The
        // lowering needs nothing new: it already follows a handle and indexes it.
        let mut reached = base;
        let mut taken = 0;
        for (dimensions, _) in levels {
            let how_many = dimensions.len();
            reached = Value::At {
                array: Box::new(reached),
                indices: built[taken..taken + how_many].to_vec(),
                // The growing dimension is the outermost and stands in the shape as a
                // nought: it is never a stride, so its size is never asked for.
                shape: dimensions,
            };
            taken += how_many;
        }
        Some(reached)
    }

    /// Fold a flat run into a tree, or refuse and say why.
    ///
    /// This is where the rule lives: the precedence mathematics settled is applied, and
    /// anything mathematics left open is not guessed at.
    fn tree(&mut self, value: &ast::Value) -> Option<Value> {
        let ops: Vec<ast::Operator> = value.between.iter().flatten().copied().collect();

        // Anything unsettled, next to anything else at all, is two readings and no rule.
        if ops.len() > 1 {
            let unsettled = ops.iter().find(|o| tier(o.kind).is_none());
            let comparisons = ops.iter().filter(|o| is_comparison(o.kind)).count();
            if let Some(bad) = unsettled {
                let other = ops.iter().find(|o| o.span != bad.span).expect("more than one");
                self.ambiguous(value, *bad, *other);
                return None;
            }
            if comparisons > 1 {
                let mut two = ops.iter().filter(|o| is_comparison(o.kind));
                let (a, b) = (*two.next().expect("two"), *two.next().expect("two"));
                self.ambiguous(value, a, b);
                return None;
            }
        }

        // Every operator here has a settled tier. Fold the loosest first, rightmost, so
        // that what is left binds tighter and equal tiers end up left-associative.
        self.fold(&value.terms, &value.between)
    }

    fn fold(&mut self, terms: &[ast::Term], between: &[Option<ast::Operator>]) -> Option<Value> {
        if terms.len() == 1 {
            return self.term(&terms[0]);
        }
        let mut split = 0;
        let mut loosest = 0u8;
        for (i, op) in between.iter().enumerate() {
            // An unsettled operator counts as the loosest there is. It only ever gets
            // here alone -- two of anything with one of these among them was refused
            // above -- so where it would sit against something else never comes up.
            let t = tier(op.expect("juxtaposition was refused above").kind).unwrap_or(u8::MAX);
            if t >= loosest {
                loosest = t;
                split = i;
            }
        }
        let op = between[split].expect("juxtaposition was refused above");
        let lhs = self.fold(&terms[..=split], &between[..split])?;
        let rhs = self.fold(&terms[split + 1..], &between[split + 1..])?;
        Some(Value::Binary { op: op.kind, lhs: Box::new(lhs), rhs: Box::new(rhs) })
    }

    /// `call count['xs']` — how many elements an array holds, all told.
    ///
    /// The answer is known here, because a shape is written down in the declaration and
    /// never changes. So this is a number by the time anything runs, and a loop bounded
    /// by it costs nothing at all.
    fn call(&mut self, call: &ast::Call) -> Option<Value> {
        // Marks say who made it: the writer's own function is a name between them, and
        // what the language provides is a bare word. Which is the whole of the
        // difference between `call count['xs']` and `call 'count'['xs']`, and why both
        // may be written in one program.
        if call.marked {
            let which = self.function_named(call)?;
            return self.called(which, call);
        }
        let word = self.text(call.name).to_string();

        // A bare word is either something the language provides or a module of them.
        // The top level is tried first, because `is` and `as` are up there *and* carry
        // a chain -- so `is.i64` is a function and its type, while `maths.sqrt` is a
        // module and a function, and the two shapes are told apart by which name it is.
        let top = PROVIDED.iter().find(|(module, said, _)| module.is_empty() && *said == word);
        let (name, provides) = match top {
            Some((_, said, provides)) => ((*said).to_string(), provides),
            None if MODULES.contains(&word.as_str()) => {
                let used = self
                    .libraries
                    .get(&self.at_file)
                    .is_some_and(|all| all.iter().any(|module| *module == word));
                if !used {
                    let always: Vec<&str> = PROVIDED
                        .iter()
                        .filter(|(module, _, _)| module.is_empty())
                        .map(|(_, said, _)| *said)
                        .collect();
                    self.errors.push(
                        Diagnostic::new("E0523", format!("`{word}` is a module this file does not import."))
                            .primary(call.name, "here")
                            .rule("a library is imported before it is named, whether Quench wrote it or you did")
                            .tip(format!("what is not a library is always there: {} are the language itself.", listed(&always)))
                            .fix(format!("`import [{word}];` at the top of this file")),
                    );
                    return None;
                }
                let Some(link) = call.chain.first() else {
                    self.errors.push(
                        Diagnostic::new("E0519", format!("`{word}` is a module, and this names nothing in it."))
                            .primary(call.name.to(call.close), "here")
                            .rule("a module holds functions, and a call names one of them")
                            .tip(format!("they are {}.", provided_in(&word)))
                            .fix(format!("`call {word}.sqrt[…]`, or whichever was meant")),
                    );
                    return None;
                };
                let inside = self.text(*link).to_string();
                let found = PROVIDED
                    .iter()
                    .find(|(module, said, _)| *module == word && *said == inside);
                let Some((_, said, provides)) = found else {
                    self.errors.push(
                        Diagnostic::new("E0455", format!("`{word}` has nothing called `{inside}`."))
                            .primary(*link, "here")
                            .rule("a module holds the functions it holds, and this names none of them")
                            .tip(format!("`{word}` holds {}.", provided_in(&word)))
                            .fix("check the spelling"),
                    );
                    return None;
                };
                if call.chain.len() > 1 {
                    self.errors.push(
                        Diagnostic::new("E0499", format!("`{word}.{said}` carries no chain."))
                            .primary(call.chain[1], "here")
                            .rule("`is` and `as` are the only ones that say a type, because they are the only ones that cannot work it out")
                            .fix(format!("`call {word}.{said}[…]`")),
                    );
                    return None;
                }
                ((*said).to_string(), provides)
            }
            None => {
                // The one that will be written by habit for a long while: a maths name
                // on its own, from before the maths went behind a namespace.
                if let Some((module, said, _)) =
                    PROVIDED.iter().find(|(module, said, _)| !module.is_empty() && *said == word)
                {
                    self.errors.push(
                        Diagnostic::new("E0520", format!("`{said}` is in `{module}`."))
                            .primary(call.name, "here")
                            .rule(format!("`{module}` is a module the language provides, and its functions are named through it"))
                            .tip(format!("`{module}` holds {}, which is most of what the language provides — so they are behind a name rather than in front of everything.", provided_in(module)))
                            .fix(format!("`call {module}.{said}[…]`")),
                    );
                    return None;
                }
                let all: Vec<String> = PROVIDED
                    .iter()
                    .filter(|(module, _, _)| module.is_empty())
                    .map(|(_, said, _)| format!("`{said}`"))
                    .chain(MODULES.iter().map(|module| format!("`{module}`, which is a module")))
                    .collect();
                self.errors.push(
                    Diagnostic::new("E0455", format!("there is nothing called `{word}`."))
                        .primary(call.name, "here")
                        .rule("a bare word after `call` is something the language provides, and this names none of them")
                        .tip(format!("they are {}.", all.join(", ")))
                        .fix(format!("`call '{word}'[…]` if you declared it with `fn`")),
                );
                return None;
            }
        };

        // Only `is` and `as` say a second thing, and everything else is one word. Said
        // here rather than in each of them, because the ones that take no chain are the
        // ones nobody thinks to write the check in.
        if top.is_some() && !matches!(provides, Provides::Reads | Provides::Becomes) {
            if let Some(link) = call.chain.first() {
                self.errors.push(
                    Diagnostic::new("E0499", format!("`{name}` carries no chain."))
                        .primary(*link, "here")
                        .rule("`is` and `as` are the only ones that say a type, because they are the only ones that cannot work it out")
                        .tip(format!("what `{name}` gives back follows from what it is given."))
                        .fix(format!("`call {name}[…]`")),
                );
                return None;
            }
        }

        match provides {
            Provides::Stitch => return self.stitched(call),
            Provides::Reads => return self.reads(call, false),
            Provides::Becomes => return self.reads(call, true),
            Provides::Alone(which) => return self.maths(call, &name, 1, *which),
            Provides::Paired(which) => return self.maths(call, &name, 2, *which),
            Provides::Fused => return self.maths(call, &name, 3, 0),
            Provides::Slow(which) => return self.slowly(call, &name, 1, *which),
            Provides::Power(which) => return self.slowly(call, &name, 2, *which),
            Provides::Pieces(which) => return self.pieces(call, &name, *which),
            Provides::Given(which) => return self.given(call, &name, *which),
            Provides::Count => {}
        }

        let [one] = call.args.as_slice() else {
            self.errors.push(
                Diagnostic::new("E0456", "`count` counts one array.")
                    .primary(call.name.to(call.close), "here")
                    .rule("`count` takes one array, and nothing else")
                    .fix("`call count['xs']`"),
            );
            return None;
        };
        let [term] = one.terms.as_slice() else {
            self.errors.push(
                Diagnostic::new("E0456", "`count` counts one array.")
                    .primary(one.span, "here")
                    .rule("`count` takes one array, and nothing else")
                    .fix("`call count['xs']`"),
            );
            return None;
        };

        // Any array, not only one with a name: `call count['jagged'[*2*]]` is how long the
        // second row is, and a row of a jagged array is exactly the thing whose length
        // nothing else can tell you.
        let built = self.term(term)?;
        match self.type_of(&built, one.span)? {
            // Known here when the shape said a number, and asked while it runs when it
            // said `grow`. The first costs nothing and the second costs one call.
            Ty::Arr { shape, length: Length::Said, .. } => {
                Some(Value::Number {
                    value: shape.iter().product::<usize>() as i64,
                    bits: 64,
                    signed: true,
                })
            }
            Ty::Arr { .. } => Some(Value::Count(Box::new(built))),
            // A different question with the same word, because it is the same question:
            // how many things are in this. What one character *is* is
            // `[defaults] characters`, and it is the only place text has a setting.
            Ty::Str => Some(Value::CountText(Box::new(built))),
            other => {
                self.errors.push(
                    Diagnostic::new("E0457", format!("`count` was given {} `{}`.", other.article(), other.name()))
                        .primary(one.span, format!("{} `{}`", other.article(), other.name()))
                        .rule("only an array and a piece of text hold a number of things")
                        .tip("a number holds one thing, which is itself.")
                        .fix("name an array or a `str`"),
                );
                None
            }
        }
    }

    fn term(&mut self, term: &ast::Term) -> Option<Value> {
        match term {
            ast::Term::Number(span) => {
                self.errors.push(
                    Diagnostic::new("E0436", "a bare number is a size, not a value.")
                        .primary(*span, "here")
                        .rule("a value wears marks, so that a written thing is never read differently depending on where it sits")
                        .fix(format!("`*{}*`", self.text(*span))),
                );
                None
            }
            ast::Term::At { name, indices, close } => self.reached(*name, indices, *close),
            ast::Term::Call(call) | ast::Term::Piece(ast::Piece::Call(call)) => self.call(call),
            ast::Term::Elements { open, close, .. } => {
                self.errors.push(
                    Diagnostic::new("E0437", "this is a list of elements, and nothing here wants one.")
                        .primary(open.to(*close), "here")
                        .rule("elements are written for an array, and the type says which")
                        .fix("declare it `arr`, or remove the brackets"),
                );
                None
            }
            ast::Term::Group { value, .. } => self.tree_or_leaf(value),
            ast::Term::Handed { word, copies, of } => {
                let built = self.term(of)?;
                let found = self.type_of(&built, of.span())?;
                if !matches!(found, Ty::Arr { .. }) {
                    self.errors.push(
                        Diagnostic::new("E0479", format!("`{}` is for arrays, and this is {} `{}`.", self.text(*word), found.article(), found.name()))
                            .primary(of.span(), format!("{} `{}`", found.article(), found.name()))
                            .secondary(*word, "asked here")
                            .rule("an array is the only thing in Quench a second name can reach, so it is the only thing that has to say which was meant")
                            .tip("everything else is a value: naming it again is naming the value, and there is nothing to share.")
                            .fix("remove it"),
                    );
                    return None;
                }
                Some(if *copies { Value::Copied(Box::new(built)) } else { built })
            }
            ast::Term::Not { word, of } => {
                let built = self.term(of)?;
                let found = self.type_of(&built, of.span())?;
                if found != Ty::Bool {
                    self.errors.push(
                        Diagnostic::new("E0418", format!("`not` turns a `bool` round, and this is {} `{}`.", found.article(), found.name()))
                            .primary(of.span(), format!("{} `{}`", found.article(), found.name()))
                            .secondary(*word, "asked to turn round here")
                            .rule("nothing is truthy — `not` is for the type that is already true or false")
                            .fix("compare it against something first"),
                    );
                    return None;
                }
                Some(Value::Not(Box::new(built)))
            }
            ast::Term::Piece(ast::Piece::Name(span)) => {
                self.named_value(*span).map(|(value, _)| value)
            }
            ast::Term::Piece(ast::Piece::Path(path)) => {
                self.constant_at(path).map(|(value, _)| value)
            }
            ast::Term::Piece(ast::Piece::Written { ty: None, mark }) => {
                let digits = unmarked(self.text(*mark));
                // A hole is not a type yet, so there is nothing here to read the value.
                // Which is the ordinary rule -- `*1000*` is a number under one type and
                // four characters under another -- arriving in the one place where the
                // chain genuinely cannot say, so the value has to.
                if let Ty::Hole(hole) = self.reading {
                    self.errors.push(
                        Diagnostic::new("E0509", "this written value is what says which type the hole is, so it has to say.")
                            .primary(*mark, "no type in front of it")
                            .rule(format!("`{}` is filled by what the argument holds, and a written value holds nothing until a type reads it", hole.word()))
                            .tip("naming a variable says it instead, because a variable was declared with a type.")
                            .fix(format!("`str:{}`, or whichever type was meant", self.text(*mark))),
                    );
                    return None;
                }
                if matches!(self.reading, Ty::F64 | Ty::F32 | Ty::F16) {
                    let reading = self.reading.clone();
                    return self.a_float(&digits, &reading, *mark);
                }
                if let Ty::Decimal { digits: keep } = self.reading {
                    return self.a_decimal(&digits, keep, *mark);
                }
                if self.reading == Ty::Exact {
                    return match quench_num::Exact::parse(&digits) {
                        Some(_) => Some(Value::Exact(digits)),
                        None => {
                            self.errors.push(
                                Diagnostic::new("E0474", format!("`{digits}` is not an exact number."))
                                    .primary(*mark, "here")
                                    .rule("an `e` is written `*12*`, `*-3/4*` or `*0.1*`, and all three are exact")
                                    .fix("write a whole number, a ratio, or a decimal"),
                            );
                            None
                        }
                    };
                }
                // A bare number in a sum is read by whatever the chain said, which is
                // how `*200*` is a `u8` in one line and a mistake in the next.
                if let Ty::Int { .. } = self.reading {
                    let reading = self.reading.clone();
                    if let Some(value) = self.a_whole_number(&digits, &reading, *mark) {
                        return Some(value);
                    }
                    // Not a number at all may still be `*true*`, which the chain would
                    // have caught if it had said `bool`. Fall through and see.
                    if digits.parse::<i64>().is_ok() || digits.parse::<u64>().is_ok() {
                        return None;
                    }
                    self.errors.pop();
                }
                match digits.parse::<i64>() {
                    Ok(n) => Some(Value::Number { value: n, bits: 64, signed: true }),
                    Err(_) => match digits.as_str() {
                        "true" => Some(Value::Bool(true)),
                        "false" => Some(Value::Bool(false)),
                        _ => {
                            self.errors.push(
                                Diagnostic::new("E0407", format!("`{digits}` is not a whole number."))
                                    .primary(*mark, "here")
                                    .rule("a written value in a sum is read as a number")
                                    .tip("`e:*0.1*` and `b64:*0.1*` are how a value says its own type where the chain does not.")
                                    .fix("write a whole number, or say `e:` or `b64:` in front of it"),
                            );
                            None
                        }
                    },
                }
            }
            // `e:*0.1*` — where the chain does not say what a number is, the value can.
            // Which is the same thing a `print` list has always let a value do.
            ast::Term::Piece(ast::Piece::Written { ty: Some(span), mark }) => {
                let word = self.text(*span);
                let digits = unmarked(self.text(*mark));
                match Ty::simple(word) {
                    Some(Ty::Exact) => match quench_num::Exact::parse(&digits) {
                        Some(_) => Some(Value::Exact(digits)),
                        None => {
                            self.errors.push(
                                Diagnostic::new("E0474", format!("`{digits}` is not an exact number."))
                                    .primary(*mark, "here")
                                    .rule("an `e` is written `*12*`, `*-3/4*` or `*0.1*`, and all three are exact")
                                    .fix("write a whole number, a ratio, or a decimal"),
                            );
                            None
                        }
                    },
                    Some(ty @ Ty::Int { .. }) => self.a_whole_number(&digits, &ty, *mark),
                    Some(ty @ (Ty::F64 | Ty::F32 | Ty::F16)) => {
                        self.a_float(&digits, &ty, *mark)
                    }
                    Some(Ty::Decimal { digits: keep }) => {
                        self.a_decimal(&digits, keep, *mark)
                    }
                    Some(Ty::Str) => Some(Value::Text(digits)),
                    Some(Ty::Bool) => match digits.as_str() {
                        "true" => Some(Value::Bool(true)),
                        "false" => Some(Value::Bool(false)),
                        other => {
                            self.errors.push(
                                Diagnostic::new("E0416", format!("`{other}` is not true or false."))
                                    .primary(*mark, "here")
                                    .rule("a `bool` is written `*true*` or `*false*`, and nothing is truthy")
                                    .fix("`*true*` or `*false*`"),
                            );
                            None
                        }
                    },
                    _ => {
                        self.errors.push(
                            Diagnostic::new("E0409", format!("`{word}` has nothing to do in a sum."))
                                .primary(*span, "said here")
                                .rule("a value in a sum says one of the types that are built, and the chain says the rest")
                                .tip("this is for where the chain cannot say -- two things compared under a `bool` chain, most often.")
                                .fix("`e:`, `i64:`, `str:` or `bool:`, or nothing at all"),
                        );
                        None
                    }
                }
            }
            ast::Term::Piece(ast::Piece::At { name, indices, close }) => {
                self.reached(*name, indices, *close)
            }
            ast::Term::Piece(ast::Piece::Escape(span)) => {
                self.errors.push(
                    Diagnostic::new("E0419", "an escape is part of text, not of a sum.")
                        .primary(*span, "here")
                        .rule("escapes stand beside text, and there is nothing to escape in a number")
                        .fix("remove it"),
                );
                None
            }
        }
    }

    fn tree_or_leaf(&mut self, value: &ast::Value) -> Option<Value> {
        if value.has_operators() {
            self.tree(value)
        } else if let [one] = value.terms.as_slice() {
            self.term(one)
        } else {
            self.errors.push(
                Diagnostic::new("E0415", "brackets group something to work out, and this is not worked out.")
                    .primary(value.span, "here")
                    .rule("what is inside brackets is a sum, a comparison, or one value")
                    .fix("remove the brackets"),
            );
            None
        }
    }

    /// What a built tree comes out as.
    fn type_of(&mut self, value: &Value, span: Span) -> Option<Ty> {
        match value {
            Value::Text(_) => Some(Ty::Str),
            Value::Number { bits, signed, .. } => Some(Ty::Int { bits: *bits, signed: *signed }),
            Value::Exact(_) => Some(Ty::Exact),
            Value::Decimal { digits, .. } => Some(Ty::Decimal { digits: *digits }),
            Value::Float { width, .. } => Some(match width {
                16 => Ty::F16,
                32 => Ty::F32,
                _ => Ty::F64,
            }),
            Value::Bool(_) => Some(Ty::Bool),
            Value::Copy(local) => Some(self.locals[local.0 as usize].ty.clone()),
            Value::Array { .. } => None,
            Value::Join(_) | Value::Said { .. } => Some(Ty::Str),
            Value::Slowly { .. } => Some(Ty::F64),
            Value::Maths { width, .. } => Some(match width {
                16 => Ty::F16,
                32 => Ty::F32,
                _ => Ty::F64,
            }),
            Value::Not(_) => Some(Ty::Bool),
            Value::Count(_) | Value::CountText(_) => {
                Some(Ty::Int { bits: 64, signed: true })
            }
            Value::Pieces { which, .. } => Some(match which {
                0 | 4 => Ty::Str,
                1 => Ty::Bool,
                2 => Ty::Int { bits: 64, signed: true },
                // The pieces, in order, however many there turn out to be.
                _ => Ty::Arr {
                    of: Box::new(Ty::Str),
                    shape: Vec::new(),
                    length: Length::Grows,
                },
            }),
            Value::Given { which } => Some(match which {
                0 | 1 => Ty::Str,
                2 => Ty::Bool,
                _ => Ty::Arr {
                    of: Box::new(Ty::Str),
                    shape: Vec::new(),
                    length: Length::Grows,
                },
            }),
            Value::CanRead { .. } => Some(Ty::Bool),
            Value::Read { ty, .. } => Some(ty.clone()),
            Value::Copied(of) => self.type_of(of, span),
            Value::Const(which) => Some(self.constants[*which as usize].ty.clone()),
            Value::Call { func, fill, .. } => {
                let returns = self.signatures[*func as usize].returns.clone()?;
                Some(match fill {
                    Some(fill) => holes::filled(&returns, fill),
                    None => returns,
                })
            }
            Value::At { array, .. } => match self.type_of(array, span)? {
                Ty::Arr { of, .. } => Some(*of),
                other => Some(other),
            },
            Value::Binary { op, lhs, rhs } => {
                let (l, r) = (self.type_of(lhs, span)?, self.type_of(rhs, span)?);

                if matches!(op, OpKind::And | OpKind::Or) {
                    if l != Ty::Bool || r != Ty::Bool {
                        self.errors.push(
                            Diagnostic::new("E0422", format!("`{}` joins two things that are true or false.", op.written()))
                                .primary(span, format!("{} `{}` and {} `{}`", l.article(), l.name(), r.article(), r.name()))
                                .rule("nothing is truthy — `and` and `or` are for `bool`, and there is no second way to be one")
                                .tip("a comparison makes one, and so does a `bool` variable.")
                                .fix("compare each side against something"),
                        );
                        return None;
                    }
                    return Some(Ty::Bool);
                }

                // Two things are the same or they are not, whatever they are. Which of
                // two is *larger* only means something for numbers.
                let same_or_not = matches!(op, OpKind::Eq | OpKind::Ne);

                if same_or_not && l == r {
                    return Some(Ty::Bool);
                }
                if same_or_not {
                    self.errors.push(
                        Diagnostic::new("E0441", format!("`{}` compares two of the same thing.", op.written()))
                            .primary(span, format!("{} `{}` and {} `{}`", l.article(), l.name(), r.article(), r.name()))
                            .rule("nothing converts on its own, so two types are never equal — the question does not arise")
                            .fix("compare two things of the same type"),
                    );
                    return None;
                }

                // A `number` hole is every number type at once, so it has whatever
                // all of them have -- which is the four operations and the four
                // comparisons, and is the whole reason the word exists beside `any`.
                if l == r && l == Ty::Hole(Hole::Any) {
                    self.errors.push(
                        Diagnostic::new("E0502", format!("`{}` works on numbers, and `any` is not known to be one.", op.written()))
                            .primary(span, "here")
                            .rule("a hole written `any` may be filled with a `str`, a `bool` or an array, and none of those orders or adds")
                            .tip("`==` does work on an `any`, because every type answers that one.")
                            .fix("say `number` instead of `any`, and the caller may only bring a number"),
                    );
                    return None;
                }
                if l != r
                    || !matches!(
                        l,
                        Ty::Int { .. }
                            | Ty::Exact
                            | Ty::F64
                            | Ty::F32
                            | Ty::F16
                            | Ty::Decimal { .. }
                            | Ty::Hole(Hole::Number)
                    )
                {
                    self.errors.push(
                        Diagnostic::new("E0420", format!("`{}` works on numbers.", op.written()))
                            .primary(span, format!("{} `{}` and {} `{}`", l.article(), l.name(), r.article(), r.name()))
                            .rule("arithmetic and ordering are for numbers, and nothing converts on its own")
                            .tip("an `i64` and an `e` are both numbers and are not the same number, so neither becomes the other.")
                            .fix("use the same kind of number on both sides"),
                    );
                    return None;
                }

                // The two a `number` hole does not get. `mod` is refused on a float, a
                // decimal and an `e`, and `^` is refused on everything but a `b64` -- so
                // neither works on *every* number, and a hole has only what all of them
                // have.
                if l == Ty::Hole(Hole::Number) && matches!(op, OpKind::Mod | OpKind::Pow) {
                    self.errors.push(
                        Diagnostic::new("E0503", format!("`{}` does not work on every number, and `number` is every number.", op.written()))
                            .primary(span, "here")
                            .rule("a hole has what all the types filling it have, and this one is refused on some of them")
                            .tip(if *op == OpKind::Mod {
                                "`mod` asks what a division left over, and a float, a decimal and an `e` leave nothing."
                            } else {
                                "`^` is worked out rather than asked of a library, and that is built for `b64` alone."
                            })
                            .fix("say the type you meant, rather than a hole"),
                    );
                    return None;
                }

                // `^` on a `b64` is worked out here rather than asked of a library —
                // see `Provides::Power`. On the narrower two it is not, because working
                // the answer out for a `b64` and rounding that down rounds twice.
                if matches!(l, Ty::F32 | Ty::F16) && *op == OpKind::Pow {
                    self.errors.push(
                        Diagnostic::new("E0495", format!("`^` on a `{}` is not built yet.", l.name()))
                            .primary(span, "here")
                            .rule("IEEE 754 only recommends how a power rounds, so Quench works it out itself rather than asking a library — and it does that at one width")
                            .tip("working it out for a `b64` and rounding that down would round twice, which is once too many.")
                            .fix("use a `b64`"),
                    );
                    return None;
                }

                // A remainder is what a division left over, and a float division leaves
                // nothing: it answers with the nearest float and there is no remainder
                // to ask about. `call remainder['a', 'b']` is the question IEEE defines.
                if matches!(l, Ty::F64 | Ty::F32 | Ty::F16 | Ty::Decimal { .. })
                    && *op == OpKind::Mod
                {
                    self.errors.push(
                        Diagnostic::new("E0488", format!("`mod` asks what a division left over, and a `{}` division leaves nothing.", l.name()))
                            .primary(span, "here")
                            .rule("a float division answers with the nearest float there is, so nothing is left behind to ask about")
                            .tip("`call remainder['a', 'b']` is the question IEEE 754 defines for floats, and its answer is exact.")
                            .fix("`call remainder[…]`, or use `i64` for whole-number division"),
                    );
                    return None;
                }

                // `^` on a decimal is not built: the standard specifies decimal powers
                // and this does not have them yet.
                if matches!(l, Ty::Decimal { .. }) && *op == OpKind::Pow {
                    self.errors.push(
                        Diagnostic::new("E0495", format!("`^` on a `{}` is not built yet.", l.name()))
                            .primary(span, "here")
                            .rule("a decimal power is its own specification, and this has the binary one")
                            .fix("use a `b64`, or `x` a few times"),
                    );
                    return None;
                }

                // A remainder is what is left when a division does not go exactly, and
                // an exact division always goes exactly. There is nothing left over.
                if l == Ty::Exact && *op == OpKind::Mod {
                    self.errors.push(
                        Diagnostic::new("E0476", "`mod` asks what a division left over, and an `e` division leaves nothing.")
                            .primary(span, "here")
                            .rule("an `e` divided by an `e` is an `e`, exactly, so there is no remainder to ask about")
                            .tip("`mod` is for the number types that round, which is every one of them but this.")
                            .fix("use `i64` if you want whole-number division"),
                    );
                    return None;
                }

                Some(if is_comparison(*op) { Ty::Bool } else { l })
            }
        }
    }

    /// Two operators with no agreed order between them.
    fn ambiguous(&mut self, value: &ast::Value, first: ast::Operator, second: ast::Operator) {
        let (a, b) = if first.span.start <= second.span.start {
            (first, second)
        } else {
            (second, first)
        };
        self.errors.push(
            Diagnostic::new(
                "E0421",
                format!(
                    "`{}` and `{}` have no agreed order, so this could be read two ways.",
                    a.kind.written(),
                    b.kind.written()
                ),
            )
            .primary(value.span, "which of these first?")
            .secondary(a.span, "this one")
            .secondary(b.span, "or this one")
            .rule("Quench keeps the precedence mathematics settled and invents none of its own")
            .tip("`x`, `/`, `+`, `-` and comparison need no brackets. Everything else does.")
            .fix("put brackets round whichever should happen first"),
        );
    }

    /// One piece that has to be known now: text or an escape, but not a name.
    fn literal(&mut self, piece: &ast::Piece) -> Option<String> {
        match piece {
            // The parser already said this one: a value with no operators in it is a
            // value whose chain supplied the type, and it complains where it sees that.
            // Saying it twice would be one mistake becoming two.
            ast::Piece::Written { ty: Some(_), .. } => None,
            ast::Piece::Written { ty: None, mark } => Some(unmarked(self.text(*mark))),
            ast::Piece::Escape(span) => escape(self.text(*span)).map(str::to_string).or_else(|| {
                self.errors.push(
                    Diagnostic::new("E0410", format!("`{}` is not an escape.", self.text(*span)))
                        .primary(*span, "here")
                        .rule("the escapes are `\\n`, `\\t`, `\\r` and `\\\\`"),
                );
                None
            }),
            ast::Piece::Call(ast::Call { name, close, .. })
            | ast::Piece::At { name, close, .. } => {
                self.errors.push(
                    Diagnostic::new("E0411", "an element cannot be one piece of a longer value yet.")
                        .primary(name.to(*close), "here")
                        .rule("joining something to text builds a new value, and building one needs the collector")
                        .fix("print the pieces separately"),
                );
                None
            }
            ast::Piece::Path(path) => {
                let whole = path[0].to(*path.last().expect("a path has a last link"));
                self.errors.push(
                    Diagnostic::new("E0411", "a name cannot be one piece of a longer value yet.")
                        .primary(whole, "here")
                        .rule("joining a name to something else builds a new value, and building one needs the collector")
                        .tip("a value that is *only* a name works — that copies rather than builds.")
                        .fix("declare it on its own, and print the pieces separately"),
                );
                None
            }
            ast::Piece::Name(span) => {
                self.errors.push(
                    Diagnostic::new("E0411", "a name cannot be one piece of a longer value yet.")
                        .primary(*span, "here")
                        .rule("joining a name to something else builds a new value, and building one needs the collector")
                        .tip("a value that is *only* a name works — that copies rather than builds.")
                        .fix("declare it on its own, and print the pieces separately"),
                );
                None
            }
        }
    }

    // --- deciding ---------------------------------------------------------------------

    fn conditional(&mut self, conditional: &ast::If) {
        let live = self.locals.len() as u32;
        let mut arms = Vec::new();

        for arm in &conditional.arms {
            let Some(condition) = self.condition(&arm.condition, arm.word) else {
                // The body is still checked, because a wrong condition is no reason to
                // hide every mistake inside the arm as well.
                let _ = self.scoped(&arm.body);
                continue;
            };
            arms.push(Arm { condition, body: self.scoped(&arm.body) });
        }

        let otherwise = conditional.otherwise.as_ref().map(|body| self.scoped(body));
        self.body.push(Stmt::If { arms, otherwise, live });
    }

    /// What an arm asks. It is a `bool`, and nothing else is.
    fn condition(&mut self, value: &ast::Value, word: Span) -> Option<Value> {
        let built = if value.has_operators() {
            self.tree(value)?
        } else if let [one] = value.terms.as_slice() {
            self.term(one)?
        } else {
            self.errors.push(
                Diagnostic::new("E0439", "this is more than one thing, and a question is one.")
                    .primary(value.span, "here")
                    .rule("what follows `if` is a single thing that is true or false")
                    .fix("compare two things, or name one `bool`"),
            );
            return None;
        };

        let found = self.type_of(&built, value.span)?;
        if found != Ty::Bool {
            self.errors.push(
                Diagnostic::new(
                    "E0440",
                    format!("`{}` asks something true or false, and this is {} `{}`.",
                        self.text(word), found.article(), found.name()),
                )
                .primary(value.span, format!("{} `{}`", found.article(), found.name()))
                .rule("nothing is truthy — a condition is a `bool` and there is no second way to be one")
                .tip("a comparison makes one, and so does a `bool` variable.")
                .fix(match found {
                    Ty::Int { bits: 64, signed: true } => "compare it against something, such as `> *0*`",
                    _ => "compare it against something",
                }),
            );
            return None;
        }
        Some(built)
    }

    /// `add ['xs'] = [*7*];` — one more on the end.
    fn extend(&mut self, target: &ast::Place, value: &ast::Value, held: &Ty, word: Span) {
        // The array being added to, which may be one reached through others.
        let reached = match target {
            ast::Place::Name(name) => match self.named_value(*name) {
                Some((value, _)) => value,
                None => return,
            },
            ast::Place::At { name, indices, close } => match self.at(*name, indices, *close) {
                Some(value) => value,
                None => return,
            },
        };
        let Some(ty) = self.type_of(&reached, target.span()) else { return };
        let _ = held;

        let Ty::Arr { of, shape, length: Length::Grows } = ty else {
            self.errors.push(
                Diagnostic::new("E0482", format!("`{}` does not grow.", ty.name()))
                    .primary(target.span(), format!("{} `{}`", ty.article(), ty.name()))
                    .secondary(word, "asked to grow here")
                    .rule("only an allocation whose first size says `grow` can be made longer, because every other one said how long it is")
                    .tip("a shape is part of a type, and a type does not change while a program runs.")
                    .fix("declare it `(grow)`, or `set` an element that is already there"),
            );
            return;
        };

        // One element at a time, so the thing being added is a whole element. An
        // allocation with fixed dimensions under the growing one adds a *row*, which is
        // several things at once and is not built -- `arr.arr` says the same shape and
        // adds one inner array instead.
        if !shape.is_empty() {
            let sizes: Vec<String> = shape.iter().map(usize::to_string).collect();
            self.errors.push(
                Diagnostic::new("E0483", format!(
                    "this grows in rows of ({}), and `add` adds one thing.",
                    sizes.join(" ")
                ))
                .primary(target.span(), "here")
                .rule("`add` puts one element on the end, and a row is several")
                .tip(format!("`arr.arr.{} (grow {})` says the same shape and grows by whole arrays, which `add` can put on one at a time.", of.name(), sizes.join(" ")))
                .fix("use a second `arr` link"),
            );
            return;
        }

        let Some(built) = self.value(value, &of, target.span()) else { return };
        self.body.push(Stmt::Extend { array: reached, value: built });
    }

    /// Check a block with a scope of its own, and hand back what it means.
    fn scoped(&mut self, body: &[ast::Stmt]) -> Vec<Stmt> {
        self.scope.push(HashMap::new());
        let inner = self.statements(body);
        self.scope.pop();
        inner
    }

    /// Check a run of statements into a body of their own, in whatever scope is open.
    fn statements(&mut self, body: &[ast::Stmt]) -> Vec<Stmt> {
        let outer = std::mem::take(&mut self.body);
        for (n, stmt) in body.iter().enumerate() {
            // `break` and `give` both end the block they are in, so anything under one
            // never runs. Quench says so rather than quietly dropping it.
            let ended = match body.get(n.wrapping_sub(1)) {
                Some(ast::Stmt::Break(word)) => Some((*word, "break", "the loop is left here")),
                Some(ast::Stmt::Give(give)) => {
                    Some((give.word, "give", "the answer is given here"))
                }
                _ => None,
            };
            if let Some((word, which, said)) = ended {
                self.errors.push(
                    Diagnostic::new("E0445", "nothing here can run.")
                        .secondary(word, said)
                        .primary(stmt.span(), "and this is under it")
                        .rule(format!("`{which}` ends the block it is written in"))
                        .fix(format!("move it above the `{which}`, or into the `if` that guards it")),
                );
                break;
            }
            self.statement(stmt);
        }
        std::mem::replace(&mut self.body, outer)
    }

    // --- answering --------------------------------------------------------------------

    /// `give [ … ];`, checked against what the chain promised.
    fn answer(&mut self, give: &ast::Give) {
        match (self.returns.clone(), &give.value) {
            (Some(ty), Some(value)) => {
                let Some(built) = self.value(value, &ty, give.word) else { return };
                self.body.push(Stmt::Give(Some(built)));
            }
            (Some(ty), None) => self.errors.push(
                Diagnostic::new("E0467", format!("this gives nothing back, and the function said {} `{}`.", ty.article(), ty.name()))
                    .primary(give.span, "here")
                    .rule("a `give` with no value is for a function that answers with `nothing`")
                    .fix(format!("`give [ … ];` with {} `{}` in it", ty.article(), ty.name())),
            ),
            (None, Some(value)) if self.in_start => self.errors.push(
                Diagnostic::new("E0468", "`START` has nobody to give an answer to.")
                    .primary(value.span, "here")
                    .rule("`START` is where the program begins, not something anything calls")
                    .tip("`give;` on its own works here, and stops the program early.")
                    .fix("`give;`, or print it instead"),
            ),
            (None, Some(value)) => self.errors.push(
                Diagnostic::new("E0468", "this gives something back, and the function said `nothing`.")
                    .primary(value.span, "here")
                    .rule("`nothing` means there is no answer, so there is nowhere for this to go")
                    .tip("`give;` on its own is how you leave early.")
                    .fix("`give;`, or say what the function answers with"),
            ),
            (None, None) => self.body.push(Stmt::Give(None)),
        }
    }

    /// A call written for what it does rather than for its answer.
    fn perform(&mut self, call: &ast::Call) {
        if !call.marked && self.text(call.name) == "count" {
            self.errors.push(
                Diagnostic::new("E0469", "`count` answers a question and does nothing else.")
                    .primary(call.word.to(call.close), "here")
                    .rule("a call written on its own is written for what it does, and this does nothing")
                    .fix("use the answer, or remove the line"),
            );
            return;
        }
        if !call.marked {
            self.errors.push(
                Diagnostic::new("E0455", format!("there is nothing called `{}`.", self.text(call.name)))
                    .primary(call.name, "here")
                    .rule("a call written on its own names a function the writer declared, between marks")
                    .fix("check the spelling, or declare it with `fn`"),
            );
            return;
        }
        let Some(which) = self.function_named(call) else {
            return;
        };
        let Some((args, fill)) = self.arguments(which, call.name, &call.args, call.close)
        else {
            return;
        };
        self.body.push(Stmt::Do { func: which, args, fill });
    }

    /// Look a call up, and check what it was given against what it takes.
    fn arguments(
        &mut self,
        which: u32,
        at_name: Span,
        given: &[ast::Value],
        close: Span,
    ) -> Option<(Vec<Value>, Option<Ty>)> {
        let signature = &self.signatures[which as usize];
        let name = signature.said.clone();
        let (wanted, at, list) = (signature.takes.clone(), signature.at, signature.list);
        let hole = signature.hole;
        if given.len() != wanted.len() {
            self.errors.push(
                Diagnostic::new(
                    "E0470",
                    format!(
                        "`'{name}'` takes {}, and was given {}.",
                        counted(wanted.len(), "thing"),
                        counted(given.len(), "thing")
                    ),
                )
                .secondary(list, format!("takes {}", counted(wanted.len(), "thing")))
                .primary(at_name.to(close), format!("given {}", counted(given.len(), "thing")))
                .rule("a call brings one value for each parameter, in the same order")
                .fix("add what is missing, or take away what is spare"),
            );
            return None;
        }

        let Some(hole) = hole else {
            let mut args = Vec::new();
            for (value, ty) in given.iter().zip(&wanted) {
                args.push(self.value(value, ty, at)?);
            }
            return Some((args, None));
        };

        // The arguments are what say what the hole is. Nothing at the call site names a
        // type, for the same reason nothing names one anywhere else it can be worked
        // out — `is.i64` says which type because text cannot say, and an argument can.
        let mut fill: Option<(Ty, Span)> = None;
        let mut built: Vec<Option<(Value, Ty)>> = Vec::with_capacity(wanted.len());
        for (value, ty) in given.iter().zip(&wanted) {
            if !holes::mentioned(ty) {
                built.push(None);
                continue;
            }
            let outer = std::mem::replace(&mut self.reading, Ty::Hole(hole));
            let made = self.tree_or_leaf(value);
            self.reading = outer;
            let made = made?;
            let found = self.type_of(&made, value.span)?;
            let Some(said) = holes::solve(ty, &found) else {
                self.errors.push(
                    Diagnostic::new("E0505", format!("this is {} `{}`, and `'{name}'` takes {} `{}`.", found.article(), found.name(), ty.article(), ty.name()))
                        .primary(value.span, format!("{} `{}`", found.article(), found.name()))
                        .secondary(list, format!("takes {} `{}`", ty.article(), ty.name()))
                        .rule("a hole is filled by what the argument holds, and the shape around it still has to match")
                        .tip("`arr.any` takes an array of something. It does not take the something.")
                        .fix("pass an argument of that shape"),
                );
                return None;
            };
            match &fill {
                None => fill = Some((said, value.span)),
                Some((first, at_first)) if first != &said => {
                    let (first, at_first) = (first.clone(), *at_first);
                    self.errors.push(
                        Diagnostic::new("E0506", format!("`'{name}'` has one hole, and this call gives it two types."))
                            .secondary(at_first, format!("filled with `{}` here", first.name()))
                            .primary(value.span, format!("and with `{}` here", said.name()))
                            .rule("every mention of a hole in one signature is the same hole, so one call fills it once")
                            .tip("that is what makes `[immut.any 'a', immut.any 'b']` mean two of the same thing.")
                            .fix("pass two of the same type"),
                    );
                    return None;
                }
                Some(_) => {}
            }
            built.push(Some((made, found)));
        }

        let Some((fill, at_fill)) = fill else {
            self.errors.push(
                Diagnostic::new("E0507", format!("nothing here says what `'{name}'`'s `{}` is.", hole.word()))
                    .primary(at_name.to(close), "here")
                    .secondary(list, "and nothing it takes mentions the hole")
                    .rule("a hole is worked out from the arguments, so it has to appear in what the function takes")
                    .tip("a function whose hole is only in what it gives back has nothing to work it out from, and there is no way to say it at the call.")
                    .fix("take something of that type, or say a real type"),
            );
            return None;
        };
        if !hole.takes(&fill) {
            self.errors.push(
                Diagnostic::new("E0508", format!("`'{name}'` takes a `number`, and this is {} `{}`.", fill.article(), fill.name()))
                    .primary(at_fill, format!("{} `{}`", fill.article(), fill.name()))
                    .secondary(list, "`number` here")
                    .rule("a `number` hole is filled by a number type, because the body is allowed to order and add what goes in it")
                    .tip("`any` is the hole that takes everything, and in exchange its body may only hold, hand back and `==`.")
                    .fix("pass a number, or have it say `any`"),
            );
            return None;
        }

        // And now the ordinary check, against the type the hole turned out to be. So a
        // wrong argument to a generic function gets the same error a wrong argument to
        // any other one gets.
        let mut args = Vec::new();
        for ((value, ty), made) in given.iter().zip(&wanted).zip(built) {
            match made {
                Some((made, found)) => {
                    let asked = holes::filled(ty, &fill);
                    if !asked.accepts(&found) {
                        self.errors.push(
                            Diagnostic::new("E0406", format!("this is {} `{}`, and it is being given to {} `{}`.", found.article(), found.name(), asked.article(), asked.name()))
                                .primary(value.span, format!("{} `{}`", found.article(), found.name()))
                                .secondary(at_fill, format!("the hole was filled with `{}` here", fill.name()))
                                .rule("nothing converts on its own — two types meet only where something says they should")
                                .fix("pass the same type"),
                        );
                        return None;
                    }
                    args.push(made);
                }
                None => args.push(self.value(value, ty, at)?),
            }
        }
        Some((args, Some(fill)))
    }

    // --- looping ----------------------------------------------------------------------

    fn leave(&mut self, word: Span) {
        if self.depth == 0 {
            self.errors.push(
                Diagnostic::new("E0446", "`break` is written outside a loop.")
                    .primary(word, "here")
                    .rule("`break` leaves the loop it is written in, and there is none")
                    .tip("an `if` is not a loop. `break` looks past every one of them to the nearest `loop`.")
                    .fix("put it inside a `loop`"),
            );
            return;
        }
        self.body.push(Stmt::Break);
    }

    /// `loop.temp.range.i64 ['i'] = [*1*, *5*] { … }`, or `loop.while … { … }`.
    ///
    /// The chain is read the same way a declaration's is, because for a counting loop it
    /// is one: `temp`/`perm` is how long the counter lives, and the last link is what it
    /// is. A `while` has no counter and so says neither.
    fn repeat(&mut self, repeat: &ast::Loop) {
        let live = self.locals.len() as u32;
        let counting = matches!(repeat.kind, ast::LoopKind::Range { .. });

        let mut lives: Option<(bool, Span)> = None;
        let mut ty_span: Option<Span> = None;
        for link in &repeat.chain {
            match self.text(*link) {
                word @ ("temp" | "perm") => match lives {
                    None => lives = Some((word == "perm", *link)),
                    Some((_, first)) => self.errors.push(
                        Diagnostic::new("E0447", "this says twice how long the counter lives.")
                            .secondary(first, format!("`{}` here", self.text(first)))
                            .primary(*link, format!("and `{word}` here"))
                            .rule("a counting loop says `temp` or `perm`, once")
                            .fix("keep the one that was meant"),
                    ),
                },
                "range" | "while" => {}
                word if counting && ty_span.is_none() => {
                    if Ty::simple(word).is_none() {
                        self.errors.push(
                            Diagnostic::new("E0402", format!("`{word}` is not a type."))
                                .primary(*link, "here")
                                .rule("a counting loop's chain ends with the type of its counter")
                                .fix("check the spelling"),
                        );
                        return;
                    }
                    ty_span = Some(*link);
                }
                word => self.errors.push(
                    Diagnostic::new("E0448", format!("`{word}` has no place in a loop's chain."))
                        .primary(*link, "here")
                        .rule("a loop reads `loop.temp|perm.range.<type>`, or `loop.while`")
                        .fix("remove it"),
                ),
            }
        }

        match &repeat.kind {
            ast::LoopKind::While(condition) => self.asking(repeat, condition, lives, ty_span),
            ast::LoopKind::Range { name, from, to } => {
                self.counting(repeat, *name, from, to, live, lives, ty_span)
            }
        }
    }

    /// `loop.while <condition> { … }` — no counter, and so nothing to say about one.
    fn asking(
        &mut self,
        repeat: &ast::Loop,
        condition: &ast::Value,
        lives: Option<(bool, Span)>,
        ty_span: Option<Span>,
    ) {
        if let Some((_, at)) = lives {
            let word = self.text(at);
            self.errors.push(
                Diagnostic::new("E0449", format!("a `while` loop has no counter, so `{word}` has nothing to describe."))
                    .primary(at, "here")
                    .rule("`temp` and `perm` say how long a counter lives, and only `range` has one")
                    .tip("a `while` that wants a counter wants a `var` above it and a `set` inside it — which is what it would have been either way.")
                    .fix("`loop.while`"),
            );
        }
        if let Some(at) = ty_span {
            self.errors.push(
                Diagnostic::new("E0450", "a `while` loop declares nothing, so it has no type.")
                    .primary(at, "here")
                    .rule("the type at the end of a loop's chain is its counter's, and a `while` has no counter")
                    .fix("`loop.while`"),
            );
        }

        let live = self.locals.len() as u32;
        let Some(built) = self.condition(condition, repeat.word) else {
            let _ = self.scoped(&repeat.body);
            return;
        };
        self.depth += 1;
        let body = self.scoped(&repeat.body);
        self.depth -= 1;
        self.body.push(Stmt::Loop { flow: Flow::While(built), body, live });
    }

    /// `loop.temp.range.i64 ['i'] = [*1*, *5*] { … }` — both ends included.
    fn counting(
        &mut self,
        repeat: &ast::Loop,
        name: Span,
        from: &ast::Value,
        to: &ast::Value,
        live: u32,
        lives: Option<(bool, Span)>,
        ty_span: Option<Span>,
    ) {
        let Some((keeps, _)) = lives else {
            self.errors.push(
                Diagnostic::new("E0451", "this loop does not say how long its counter lives.")
                    .primary(repeat.word, "here")
                    .rule("a counting loop says `temp` or `perm`, and silence is not one of them")
                    .tip("`temp` is the usual one. `perm` keeps the counter afterwards, holding the last value it took, which is what you want after a `break`.")
                    .fix("`loop.temp.range.<type>`, or `loop.perm.range.<type>`"),
            );
            return;
        };
        let Some(ty_span) = ty_span else {
            self.errors.push(
                Diagnostic::new("E0452", "this loop does not say what its counter is.")
                    .primary(repeat.word, "the chain ends here")
                    .rule("a counting loop declares a variable, and a declaration always says its type")
                    .fix("`loop.temp.range.i64 [...]`"),
            );
            return;
        };
        let word = self.text(ty_span);
        if word != "i64" {
            self.errors.push(
                Diagnostic::new("E0453", format!("counting by `{word}` is not built yet."))
                    .primary(ty_span, "here")
                    .rule("Quench means to count by every number type, and counts by `i64` today")
                    .fix("`i64` for now"),
            );
            return;
        }

        // Both bounds before the first pass, and neither asked again. A loop whose end
        // moved under it would be a loop nobody could read.
        let Some(from) = self.value(from, &Ty::Int { bits: 64, signed: true }, name) else { return };
        let Some(to) = self.value(to, &Ty::Int { bits: 64, signed: true }, name) else { return };

        let counter = LocalId(self.locals.len() as u32);
        debug_assert_eq!(counter.0, live);
        let text = self.named(name);
        self.locals.push(Local {
            counter: true,
            name: text.clone(),
            ty: Ty::Int { bits: 64, signed: true },
            mutable: false,
            at: name,
            chain: repeat.word.to(*repeat.chain.last().unwrap_or(&repeat.word)),
        });

        self.scope.push(HashMap::new());
        self.scope.last_mut().expect("just pushed").insert(text.clone(), counter);
        self.depth += 1;
        let body = self.statements(&repeat.body);
        self.depth -= 1;
        self.scope.pop();

        // `perm` is the one thing in Quench that outlives the block it was written in,
        // and it is deliberate: after an early `break` the counter is the answer.
        if keeps {
            self.scope.last_mut().expect("a scope is always open").insert(text, counter);
        }

        self.body.push(Stmt::Loop { flow: Flow::Range { from, to, keeps }, body, live });
    }

    // --- changing ---------------------------------------------------------------------

    fn set(&mut self, set: &ast::Set, adding: bool) {
        for (n, target) in set.targets.iter().enumerate() {
            let Some(local) = self.lookup(target.name()) else { continue };
            let held = self.locals[local.0 as usize].ty.clone();

            // Not the `immut` error, because it is not the same mistake: the counter
            // does change, every pass, and what is wrong is who changes it.
            if self.locals[local.0 as usize].counter {
                let declared = self.locals[local.0 as usize].clone();
                self.errors.push(
                    Diagnostic::new(
                        "E0454",
                        format!("`'{}'` is a loop's counter, and the loop is what moves it.", declared.name),
                    )
                    .secondary(declared.at, "the loop counts this")
                    .primary(target.name(), "and this would move it too")
                    .rule("a counter belongs to its loop: the bounds say where it starts and stops, and nothing else may say otherwise")
                    .tip("`break` is how you leave early, and `perm` is how you keep where it stopped.")
                    .fix("count with a `var.mut` of your own, or use `break`"),
                );
                continue;
            }

            if !self.locals[local.0 as usize].mutable {
                let declared = self.locals[local.0 as usize].clone();
                // `var.immut.i64` becomes `var.mut.i64`, which is the line they wanted.
                // A replacement rather than an insertion, since every declaration now
                // says one or the other and inserting would say both.
                let with_mut = self.text(declared.chain).replacen("immut", "mut", 1);
                self.errors.push(
                    Diagnostic::new(
                        "E0438",
                        format!("`'{}'` cannot be changed, because its declaration never said it could.", declared.name),
                    )
                    .secondary(declared.chain, "declared `immut` here")
                    .primary(target.name(), "changed here")
                    .rule("a variable changes only if its declaration says `mut`")
                    .tip("`immut` and `mut` are the two answers, and this one gave the other.")
                    .fix(format!("`{with_mut}`")),
                );
                continue;
            }

            let Some(value) = set.values.get(n) else { continue };

            if adding {
                self.extend(target, value, &held, set.word);
                continue;
            }

            match target {
                ast::Place::Name(span) => {
                    let Some(built) = self.value(value, &held, *span) else { continue };
                    self.body.push(Stmt::Assign { to: Place::Local(local), value: built });
                }
                ast::Place::At { name, indices, close } => {
                    if !matches!(held, Ty::Arr { .. }) {
                        self.errors.push(
                            Diagnostic::new("E0433", format!("`{}` is not an array.", held.name()))
                                .primary(*name, format!("{} `{}`", held.article(), held.name()))
                                .rule("only an array has elements to change")
                                .fix("change the whole thing, or index an array"),
                        );
                        continue;
                    }
                    // Read the same way it would be read, then written to instead. The
                    // outermost step is the one that changes something; everything
                    // under it is the walk to get there.
                    let Some(reached) = self.at(*name, indices, *close) else { continue };
                    let Some(of) = self.type_of(&reached, name.to(*close)) else { continue };
                    let Value::At { array, indices: built, shape } = reached else {
                        continue;
                    };
                    let Some(value) = self.value(value, &of, *name) else { continue };
                    self.body.push(Stmt::Assign {
                        to: Place::Element { array: *array, indices: built, shape },
                        value,
                    });
                }
            }
        }
    }

    // --- printing ---------------------------------------------------------------------

    fn print(&mut self, print: &ast::Print) {
        let named = self.text(print.to);
        let Some(to) = Stream::from_name(named) else {
            let all: Vec<String> =
                Stream::ALL.iter().map(|s| format!("`{}`", s.name())).collect();
            self.errors.push(
                Diagnostic::new("E0442", format!("`{named}` is not somewhere to print."))
                    .primary(print.to, "here")
                    .rule("a `print` says where it goes, because a reader should not have to know a default")
                    .tip(format!("there is {}.", all.join(" and ")))
                    .fix(format!("{} for the ordinary one", all[0])),
            );
            return;
        };
        let mut pieces = Vec::new();
        for piece in &print.pieces {
            match piece {
                ast::Piece::Name(span) => {
                    let Some((value, ty)) = self.named_value(*span) else { continue };
                    pieces.push(Printed::Value { value, ty });
                }
                ast::Piece::Path(path) => {
                    let Some((value, ty)) = self.constant_at(path) else { continue };
                    pieces.push(Printed::Value { value, ty });
                }
                ast::Piece::Written { ty: None, mark } => {
                    self.errors.push(
                        Diagnostic::new("E0412", "this written value does not say what it is.")
                            .primary(*mark, "no type in front of it")
                            .rule("a written value means nothing until a type reads it: `*1000*` is a number under `i64` and four characters under `str`")
                            .tip("a declaration says the type in its chain, so only here does the value have to.")
                            .fix(format!("`str:{}` if it is text", self.text(*mark))),
                    );
                }
                ast::Piece::Written { ty: Some(span), mark } => {
                    let word = self.text(*span);
                    match Ty::simple(word) {
                        Some(Ty::Arr { .. }) => unreachable!("`arr` is not a simple type"),
                        Some(Ty::Hole(_)) => unreachable!("a hole word is not a simple type"),
                        Some(ty @ (Ty::F64 | Ty::F32 | Ty::F16)) => {
                            let written = unmarked(self.text(*mark));
                            if let Some(value) = self.a_float(&written, &ty, *mark) {
                                pieces.push(Printed::Value { value, ty });
                            }
                        }
                        Some(ty @ Ty::Decimal { digits }) => {
                            let written = unmarked(self.text(*mark));
                            if let Some(value) = self.a_decimal(&written, digits, *mark) {
                                pieces.push(Printed::Value { value, ty });
                            }
                        }
                        Some(Ty::Exact) => {
                            let written = unmarked(self.text(*mark));
                            match quench_num::Exact::parse(&written) {
                                Some(_) => {
                                    pieces.push(Printed::Value {
                                        value: Value::Exact(written),
                                        ty: Ty::Exact,
                                    })
                                }
                                None => self.errors.push(
                                    Diagnostic::new("E0474", format!("`{written}` is not an exact number."))
                                        .primary(*mark, "here")
                                        .rule("an `e` is written `*12*`, `*-3/4*` or `*0.1*`, and all three are exact")
                                        .fix("write a whole number, a ratio, or a decimal"),
                                ),
                            }
                        }
                        Some(Ty::Str) => pieces.push(Printed::Text(unmarked(self.text(*mark)))),
                        Some(Ty::Bool) => match unmarked(self.text(*mark)).as_str() {
                            "true" | "false" => {
                                pieces.push(Printed::Text(unmarked(self.text(*mark))))
                            }
                            other => self.errors.push(
                                Diagnostic::new("E0416", format!("`{other}` is not true or false."))
                                    .primary(*mark, "here")
                                    .rule("a `bool` is written `*true*` or `*false*`, and nothing is truthy")
                                    .fix("`*true*` or `*false*`"),
                            ),
                        },
                        Some(Ty::Int { .. }) => {
                            let digits = unmarked(self.text(*mark));
                            match digits.parse::<i64>() {
                                Ok(_) => pieces.push(Printed::Text(digits)),
                                Err(_) => self.errors.push(
                                    Diagnostic::new("E0407", format!("`{digits}` is not a whole number."))
                                        .primary(*mark, "here")
                                        .rule("a written value is read by the type it is given to, and `i64` reads whole numbers")
                                        .fix("write a whole number"),
                                ),
                            }
                        }
                        None => self.errors.push(
                            Diagnostic::new("E0402", format!("`{word}` is not a type."))
                                .primary(*span, "here")
                                .rule("a value that says its own type says one of the types there are")
                                .fix("check the spelling"),
                        ),
                    }
                }
                ast::Piece::At { name, indices, close } => {
                    let Some(value) = self.reached(*name, indices, *close) else { continue };
                    let Some(ty) = self.type_of(&value, name.to(*close)) else { continue };
                    pieces.push(Printed::Value { value, ty });
                }
                ast::Piece::Call(call) => {
                    let Some(value) = self.call(call) else { continue };
                    let at = call.name.to(call.close);
                    let Some(ty) = self.type_of(&value, at) else { continue };
                    pieces.push(Printed::Value { value, ty });
                }
                ast::Piece::Escape(span) => match escape(self.text(*span)) {
                    Some(text) => pieces.push(Printed::Text(text.to_string())),
                    None => self.errors.push(
                        Diagnostic::new("E0410", format!("`{}` is not an escape.", self.text(*span)))
                            .primary(*span, "here")
                            .rule("the escapes are `\\n`, `\\t`, `\\r` and `\\\\`"),
                    ),
                },
            }
        }
        self.body.push(Stmt::Print { to, pieces });
    }

    /// Whether this name is already taken, anywhere still in scope.
    fn seen(&self, name: &str) -> Option<LocalId> {
        self.scope.iter().rev().find_map(|scope| scope.get(name)).copied()
    }

    /// A name used as a value: what a variable holds, or a constant written in.
    /// `'text'.'MARK'` — a constant named through a module path.
    ///
    /// Only a constant is ever reached this way. A variable is local, so it has no
    /// module to be in; a function is reached with `call`, which has a path of its own.
    fn constant_at(&mut self, path: &[Span]) -> Option<(Value, Ty)> {
        let spelt: Vec<String> = path.iter().map(|link| self.named(*link)).collect();
        let key = spelt.join(".");
        let whole = path[0].to(*path.last().expect("a path has a last link"));

        let Some(which) = self.reachable_constant(&key) else {
            self.errors.push(
                Diagnostic::new("E0455", format!("there is nothing called `'{key}'`."))
                    .primary(whole, "here")
                    .rule("a path in a value names a constant in a module, and a name is looked for here and then outward")
                    .tip("a variable is inside a function, so it has no module to be in, and a function is reached with `call`.")
                    .fix("check the spelling, or declare it with `const`"),
            );
            return None;
        };

        let held = self.constants[which as usize].in_module.clone();
        let held_file = self.constants[which as usize].in_file.clone();
        if !self.modules_reach(&held_file, &held, whole) {
            return None;
        }
        let constant = &self.constants[which as usize];
        if !constant
            .visibility
            .reaches((&self.at_file, &self.at_module), (&constant.in_file, &constant.in_module))
        {
            let (said, at) = (constant.visibility.word(), constant.at);
            let here = format!(
                "`{}`",
                Checker::qualified(&self.at_file, &self.at_module, "").trim_end_matches('.')
            );
            self.errors.push(
                Diagnostic::new("E0511", format!("`'{key}'` says `{said}`, and this is written in {here}."))
                    .secondary(at, format!("`{said}` here"))
                    .primary(whole, "and named here")
                    .rule(format!("`{}` are the five, narrowest first, and each names a boundary a name may cross", listed(Visibility::ALL)))
                    .tip("the same ladder a function says, and the same walk outward to find it.")
                    .fix("widen what it says, or move this inside"),
            );
            return None;
        }
        Some((Value::Const(which), constant.ty.clone()))
    }

    fn named_value(&mut self, span: Span) -> Option<(Value, Ty)> {
        let name = self.named(span);
        if let Some(local) = self.seen(&name) {
            return Some((Value::Copy(local), self.locals[local.0 as usize].ty.clone()));
        }
        if let Some(which) = self.reachable_constant(&name) {
            return Some((Value::Const(which), self.constants[which as usize].ty.clone()));
        }
        self.lookup(span)
            .map(|local| (Value::Copy(local), self.locals[local.0 as usize].ty.clone()))
    }

    /// A name that has to be somewhere a value *lives* — indexed, counted or changed.
    fn lookup(&mut self, span: Span) -> Option<LocalId> {
        let name = self.named(span);
        if let Some(which) = self.reachable_constant(&name) {
            let at = self.constants[which as usize].at;
            self.errors.push(
                Diagnostic::new("E0472", format!("`'{name}'` is a constant."))
                    .secondary(at, "declared here")
                    .primary(span, "and changed here")
                    .rule("a constant is what a program was written with, and a program does not rewrite what it was written with")
                    .tip("an array constant lives in the module and can be read and indexed; what nothing can do is change it.")
                    .fix("`copy` it into a `var.mut` first, and change that"),
            );
            return None;
        }
        match self.seen(&name) {
            Some(local) => Some(local),
            None => {
                let near = self.nearest(&name);
                let mut diag =
                    Diagnostic::new("E0413", format!("`'{name}'` is not declared."))
                        .primary(span, "here")
                        .rule("a name means something only after a declaration says what it means");
                diag = match near {
                    Some(near) => diag.fix(format!("did you mean `'{near}'`?")),
                    None => diag.fix("declare it above, with `var`"),
                };
                self.errors.push(diag);
                None
            }
        }
    }

    /// The declared name closest to this one, when there is a close one.
    ///
    /// Only ever suggests something within one edit, because a suggestion that is not
    /// the answer is worse than no suggestion: it costs the reader a second look.
    fn nearest(&self, name: &str) -> Option<String> {
        if let Some(near) = self.known.keys().find(|known| within_one_edit(known, name)) {
            return Some(near.clone());
        }
        self.scope
            .iter()
            .rev()
            .flat_map(|scope| scope.keys())
            .find(|known| within_one_edit(known, name))
            .cloned()
    }
}

struct Chain {
    mutable: bool,
    ty: Option<Ty>,
    ty_span: Span,
}

/// What is between a pair of marks, with the mark's own escape undone.
fn unmarked(written: &str) -> String {
    if written.len() < 2 {
        return String::new();
    }
    let inside = &written[1..written.len() - 1];
    let mut out = String::with_capacity(inside.len());
    let mut chars = inside.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(next @ ('*' | '\\' | '\'')) => out.push(next),
                Some(next) => {
                    out.push('\\');
                    out.push(next);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape(written: &str) -> Option<&'static str> {
    match written {
        "\\n" => Some("\n"),
        "\\t" => Some("\t"),
        "\\r" => Some("\r"),
        "\\\\" => Some("\\"),
        _ => None,
    }
}

/// Whether two names differ by at most one insertion, deletion or substitution.
fn within_one_edit(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if a.len().abs_diff(b.len()) > 1 {
        return false;
    }
    let (long, short) = if a.len() >= b.len() { (&a, &b) } else { (&b, &a) };
    let mut skipped = false;
    let (mut i, mut j) = (0, 0);
    while i < long.len() && j < short.len() {
        if long[i] == short[j] {
            i += 1;
            j += 1;
        } else if skipped {
            return false;
        } else {
            skipped = true;
            i += 1;
            if long.len() == short.len() {
                j += 1;
            }
        }
    }
    true
}
