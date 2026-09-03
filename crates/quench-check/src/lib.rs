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

use quench_diag::{Diagnostic, Span};
use quench_parse::{ast, counted, Parsed};
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
    /// `arr.i64 (2 3)` — one allocation, laid out row by row.
    ///
    /// One `arr` link is one allocation however many dimensions it has, which is what
    /// makes indexing arithmetic. Two `arr` links are two allocations, with handles in
    /// the outer one.
    ///
    /// `grows` is the first size having said `grow` rather than a number, and only the
    /// first may: indexing is `(i - 1) x stride + j`, and a stride is the sizes *under*
    /// a dimension. The outermost has nothing above it to be a stride for, so it is the
    /// one dimension whose size the arithmetic never needs.
    Arr { of: Box<Ty>, shape: Vec<usize>, grows: bool },
}

impl Ty {
    pub fn name(&self) -> String {
        match self {
            Ty::Int { bits, signed } => {
                format!("{}{bits}", if *signed { "i" } else { "u" })
            }
            Ty::Str => "str".to_string(),
            Ty::Bool => "bool".to_string(),
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
                while let Ty::Arr { of, shape, grows } = walking {
                    links += 1;
                    if *grows {
                        sizes.push("grow".to_string());
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
            Ty::Str | Ty::Bool | Ty::F64 | Ty::F32 | Ty::F16 | Ty::Decimal { .. } => "a",
        }
    }

    /// Whether every allocation in this, from the top down, said how big it is.
    ///
    /// When one did not, nothing can say where one row of the thing above it ends —
    /// which is why such an array can only be written empty and filled afterwards.
    pub fn settled(&self) -> bool {
        match self {
            Ty::Arr { of, grows, .. } => !grows && of.settled(),
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
    File,
    Program,
    Export,
}

impl Visibility {
    fn from_word(word: &str) -> Option<Visibility> {
        match word {
            "file" => Some(Visibility::File),
            "program" => Some(Visibility::Program),
            "export" => Some(Visibility::Export),
            _ => None,
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
    /// `not 'ready'` — the opposite of a `bool`.
    Not(Box<Value>),
    /// `copy 'xs'` — a new array holding the same things. `share 'xs'` needs no node of
    /// its own: it is the handle, which is what naming a variable already gives.
    Copied(Box<Value>),
    /// How many an array holds, asked while it runs — which is what `count` becomes on
    /// an array that grows, and what it never becomes on one that does not.
    Count(Box<Value>),
    /// A top-level constant, written in where it was named.
    Const(u32),
    /// `add[*1*, *2*]` — the answer a function gave back.
    Call { func: u32, args: Vec<Value> },
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
    while let Ty::Arr { of, shape, grows } = walking {
        at += shape.len() + usize::from(*grows);
        stops.push(at);
        walking = of;
    }
    stops
}

/// The lowest and highest a whole-number type holds.
///
/// The high end of a `u64` does not fit in an `i64`, and is carried as the bits of one
/// — which is what the whole type does, so nothing is lost by saying it here too.
fn whole_range(bits: u8, signed: bool) -> (i64, i64) {
    if signed {
        let high = if bits >= 64 { i64::MAX } else { (1i64 << (bits - 1)) - 1 };
        (-high - 1, high)
    } else {
        let high = if bits >= 64 { -1i64 } else { (1i64 << bits) - 1 };
        (0, high)
    }
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
    Do { func: u32, args: Vec<Value> },
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
    name: String,
    visibility: Option<Visibility>,
    returns: Option<Ty>,
    /// One per parameter, in order.
    takes: Vec<Ty>,
    /// The name, for pointing at.
    at: Span,
    /// The parameter list, for pointing at when a call brings the wrong number.
    list: Span,
}

/// Read a whole file, parse it, and work out what it means.
pub fn check(source: &str) -> Checked {
    let Parsed { program, errors } = quench_parse::parse(source);
    let mut checker = Checker {
        source,
        locals: Vec::new(),
        scope: vec![HashMap::new()],
        depth: 0,
        returns: None,
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

    // Constants first, in the order written: one may be built out of those above it,
    // and out of nothing else, because there is nothing else yet worked out.
    for item in &program.items {
        if let ast::Item::Const(declaration) = item {
            checker.constant(declaration);
        }
    }

    // Then every signature, before any body. Which is what lets `even` call `odd` when
    // `odd` is written underneath it.
    for item in &program.items {
        if let ast::Item::Func(func) = item {
            checker.signature(func);
        }
    }

    let mut funcs = Vec::new();
    for item in &program.items {
        if let ast::Item::Func(func) = item {
            if let Some(checked) = checker.function(func) {
                funcs.push(checked);
            }
        }
    }

    let start = program.start.as_ref().map(|start| {
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
        });
        funcs.len() - 1
    });

    Checked { funcs, constants: checker.constants, start, errors: checker.errors }
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
                        .rule("a top-level declaration says `file`, `program` or `export`, once")
                        .fix("keep the one that was meant"),
                ),
            }
        }
        match found {
            Some((visibility, _)) => Some(visibility),
            None => {
                self.errors.push(
                    Diagnostic::new("E0459", format!("this {what} does not say who can see it."))
                        .primary(word, "here")
                        .rule("everything at the top of a file says `file`, `program` or `export`, and silence is not one of them")
                        .tip("`file` is the careful answer: nothing outside this file can name it.")
                        .fix("`file` here, `program` across the program, `export` for anything outside it"),
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
            self.known.insert(name.clone(), at_index);
            self.constants.push(Constant {
                name,
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
        let name = self.named(func.name);
        let word = func.chain[0];
        let visibility = self.seen_by(&func.chain, word, "function");

        if let Some(&first) = self.named.get(&name) {
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

        self.named.insert(name.clone(), self.signatures.len() as u32);
        self.signatures.push(Signature {
            name,
            visibility,
            returns,
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
        let grows = matches!(sizes.first(), Some(Size::Grows(_)));
        let rest = if grows { &sizes[1..] } else { sizes };
        if let Some(Size::Grows(at)) =
            rest.iter().find(|size| matches!(size, Size::Grows(_)))
        {
            let at = *at;
            self.errors.push(
                Diagnostic::new("E0480", "only the first size of an allocation can grow.")
                    .primary(at, "here")
                    .secondary(arr, "this allocation")
                    .rule("finding an element is `(i - 1) x stride + j`, and a stride is the sizes under a dimension — so every size but the outermost has to be known")
                    .tip("`arr.arr.i64 (2 grow)` is two rows that each grow, which is what a growing inner dimension usually means.")
                    .fix("`grow` first, or a number here"),
            );
            return None;
        }
        let fixed: Vec<usize> = rest
            .iter()
            .map(|size| match size {
                Size::Fixed(n) => *n,
                Size::Grows(_) => unreachable!("just refused above"),
            })
            .collect();
        Some(Ty::Arr { of: Box::new(of), shape: fixed, grows })
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
        let read = if signed {
            digits.parse::<i64>().ok()
        } else {
            digits.parse::<u64>().ok().map(|n| n as i64)
        };
        match read {
            Some(n) if signed && (n < low || n > high) => {
                self.errors.push(
                    Diagnostic::new("E0489", format!("`{digits}` does not fit in {} `{}`.", ty.article(), ty.name()))
                        .primary(at, "here")
                        .rule(format!("`{}` holds {low} to {high}", ty.name()))
                        .tip("a written value is read by the type it is given to, and this one has an edge.")
                        .fix("write a number in that range, or declare it a wider type"),
                );
                None
            }
            Some(n) if !signed && ((n as u64) > high as u64) => {
                self.errors.push(
                    Diagnostic::new("E0489", format!("`{digits}` does not fit in {} `{}`.", ty.article(), ty.name()))
                        .primary(at, "here")
                        .rule(format!("`{}` holds 0 to {}", ty.name(), high as u64))
                        .tip("a written value is read by the type it is given to, and this one has an edge.")
                        .fix("write a number in that range, or declare it a wider type"),
                );
                None
            }
            Some(n) => Some(Value::Number { value: n, bits, signed }),
            None => {
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
        let read = match width {
            64 => written.parse::<f64>().ok().filter(|x| x.is_finite()).map(f64::to_bits),
            32 => written
                .parse::<f32>()
                .ok()
                .filter(|x| x.is_finite())
                .map(|x| u64::from(x.to_bits())),
            _ => written
                .parse::<f32>()
                .ok()
                .filter(|x| x.is_finite())
                .map(|x| u64::from(quench_num::to_b16(x).to_bits())),
        };
        match read {
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

    /// A whole function: its parameters put in scope, then its body.
    fn function(&mut self, func: &ast::Func) -> Option<Func> {
        let name = self.named(func.name);
        let which = *self.named.get(&name)?;
        let signature = &self.signatures[which as usize];
        let (visibility, returns) = (signature.visibility, signature.returns.clone());
        let takes: Vec<Ty> = signature.takes.clone();

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

        Some(Func {
            name,
            visibility,
            returns,
            takes: takes.len(),
            locals: std::mem::take(&mut self.locals),
            body,
            at: func.name,
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
                    if Ty::simple(word).is_none() {
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

        let word = self.text(ty_span);
        let mut ty = Ty::simple(word);
        if ty.is_none() {
            self.errors.push(
                Diagnostic::new("E0402", format!("`{word}` is not a type."))
                    .primary(ty_span, "here")
                    .rule("a chain says the type of what it is describing")
                    .tip("the types are the numbers, `e`, `bool` and `str`.")
                    .fix("check the spelling"),
            );
        }

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
            // The one size that is not a number: it says there is no number, because
            // nobody knows one yet.
            if self.text(*size) == "grow" {
                sizes.push(Size::Grows(*size));
                continue;
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
                            .rule("a size is a whole number written without marks, or `grow` where there is no number yet")
                            .fix("write a whole number, or `grow`"),
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
                    .primary(value.span, format!("a `{}`", found.name()))
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
                        .primary(*span, format!("a `{}`", held.name()))
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
        if let Ty::Arr { of, shape, grows } = ty {
            return self.array(value, of, shape, *grows, ty_span);
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
                                .primary(one.span(), format!("a `{}`", found.name()))
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
        grows: bool,
        ty_span: Span,
    ) -> Option<Value> {
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
            let rows = if grows { 0 } else { shape.iter().product::<usize>() };
            let Ty::Arr { of: under, .. } = of else {
                unreachable!("only an array holds something that is not settled")
            };
            let empty = Value::Array { of: under.clone(), elements: Vec::new() };
            return Some(Value::Array { of: Box::new(of.clone()), elements: vec![empty; rows] });
        }

        // A growing allocation takes however many were written, so long as they fill
        // whole rows of whatever lies under it. A fixed one takes exactly its size.
        if grows {
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
        if let Ty::Arr { of: inner, shape: inner_shape, grows: inner_grows } = of {
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
                let built = self.array(&piece, inner, inner_shape, *inner_grows, ty_span)?;
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
                        .primary(term.span(), format!("a `{}`", found.name()))
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

    /// A call to a function the writer declared.
    fn called(&mut self, which: u32, call: &ast::Call) -> Option<Value> {
        let args = self.arguments(which, call.name, &call.args, call.close)?;
        if self.signatures[which as usize].returns.is_none() {
            let said = self.signatures[which as usize].name.clone();
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
        Some(Value::Call { func: which, args })
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
                    .primary(name, format!("a `{}`", held.name()))
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
        while let Ty::Arr { of, shape, grows } = walking {
            if spent == indices.len() {
                break;
            }
            let mut dimensions = shape.clone();
            if grows {
                // The growing one is outermost and takes an index like any other; what
                // it does not do is take part in a stride.
                dimensions.insert(0, 0);
            }
            spent += dimensions.len();
            levels.push((dimensions, grows));
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
            let name = self.named(call.name);
            let Some(&which) = self.named.get(&name) else {
                self.errors.push(
                    Diagnostic::new("E0455", format!("there is nothing called `'{name}'`."))
                        .primary(call.name, "here")
                        .rule("a name between marks after `call` is a function the writer declared")
                        .tip("a bare word there is one of Quench's own, which is how a reader tells them apart.")
                        .fix("check the spelling, or declare it with `fn`"),
                );
                return None;
            };
            return self.called(which, call);
        }
        if self.text(call.name) != "count" {
            let name = self.text(call.name).to_string();
            self.errors.push(
                Diagnostic::new("E0455", format!("there is nothing called `{name}`."))
                    .primary(call.name, "here")
                    .rule("a bare word after `call` is something the language provides, and this names none of them")
                    .tip("`count` is the one that comes with the language.")
                    .fix(format!("`call '{name}'[…]` if you declared it with `fn`")),
            );
            return None;
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
            Ty::Arr { shape, grows: false, .. } => {
                Some(Value::Number {
                    value: shape.iter().product::<usize>() as i64,
                    bits: 64,
                    signed: true,
                })
            }
            Ty::Arr { .. } => Some(Value::Count(Box::new(built))),
            other => {
                self.errors.push(
                    Diagnostic::new("E0457", format!("`count` was given {} `{}`.", other.article(), other.name()))
                        .primary(one.span, format!("{} `{}`", other.article(), other.name()))
                        .rule("only an array holds a number of things")
                        .tip("counting the characters of a `str` is a different question, and is not built yet.")
                        .fix("name an array"),
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
            ast::Term::Piece(ast::Piece::Written { ty: None, mark }) => {
                let digits = unmarked(self.text(*mark));
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
            Value::Join(_) => Some(Ty::Str),
            Value::Not(_) => Some(Ty::Bool),
            Value::Count(_) => Some(Ty::Int { bits: 64, signed: true }),
            Value::Copied(of) => self.type_of(of, span),
            Value::Const(which) => Some(self.constants[*which as usize].ty.clone()),
            Value::Call { func, .. } => self.signatures[*func as usize].returns.clone(),
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

                if l != r
                    || !matches!(
                        l,
                        Ty::Int { .. }
                            | Ty::Exact
                            | Ty::F64
                            | Ty::F32
                            | Ty::F16
                            | Ty::Decimal { .. }
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

                // `^` on a float is `pow`, which no standard requires to be rounded
                // the same way twice — so it is one of the two things a differential
                // oracle actually has to worry about, and it waits for the answer.
                if matches!(l, Ty::F64 | Ty::F32 | Ty::F16 | Ty::Decimal { .. })
                    && matches!(op, OpKind::Pow | OpKind::Mod)
                {
                    self.errors.push(
                        Diagnostic::new("E0488", format!("`{}` on a `{}` is not built yet.", op.written(), l.name()))
                            .primary(span, "here")
                            .rule("`+`, `-`, `x`, `/` and the comparisons are settled by IEEE 754 and give the same bits everywhere; nothing else about floats is")
                            .tip("that is why they are what `b64` has: an answer every engine must agree on has to be one somebody specified.")
                            .fix("use `+`, `-`, `x`, `/` or a comparison"),
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

        let Ty::Arr { of, shape, grows: true } = ty else {
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
        let name = if call.marked {
            self.named(call.name)
        } else {
            self.text(call.name).to_string()
        };
        let Some(&which) = self.named.get(&name).filter(|_| call.marked) else {
            self.errors.push(
                Diagnostic::new("E0455", format!("there is nothing called `{}`.", self.text(call.name)))
                    .primary(call.name, "here")
                    .rule("a call written on its own names a function the writer declared, between marks")
                    .fix("check the spelling, or declare it with `fn`"),
            );
            return;
        };
        let Some(args) = self.arguments(which, call.name, &call.args, call.close) else {
            return;
        };
        self.body.push(Stmt::Do { func: which, args });
    }

    /// Look a call up, and check what it was given against what it takes.
    fn arguments(
        &mut self,
        which: u32,
        at_name: Span,
        given: &[ast::Value],
        close: Span,
    ) -> Option<Vec<Value>> {
        let signature = &self.signatures[which as usize];
        let name = signature.name.clone();
        let (wanted, at, list) = (signature.takes.clone(), signature.at, signature.list);
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

        let mut args = Vec::new();
        for (value, ty) in given.iter().zip(&wanted) {
            args.push(self.value(value, ty, at)?);
        }
        Some(args)
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
                                .primary(*name, format!("a `{}`", held.name()))
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
    fn named_value(&mut self, span: Span) -> Option<(Value, Ty)> {
        let name = self.named(span);
        if let Some(local) = self.seen(&name) {
            return Some((Value::Copy(local), self.locals[local.0 as usize].ty.clone()));
        }
        if let Some(&which) = self.known.get(&name) {
            return Some((Value::Const(which), self.constants[which as usize].ty.clone()));
        }
        self.lookup(span)
            .map(|local| (Value::Copy(local), self.locals[local.0 as usize].ty.clone()))
    }

    /// A name that has to be somewhere a value *lives* — indexed, counted or changed.
    fn lookup(&mut self, span: Span) -> Option<LocalId> {
        let name = self.named(span);
        if let Some(&which) = self.known.get(&name) {
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
