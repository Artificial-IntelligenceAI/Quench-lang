//! The shape of a Quench program, as written.
//!
//! Everything here carries a [`Span`] and nothing here carries a `String`: the source is
//! still around, so a piece of the tree is a range into it rather than a copy of it. That
//! keeps the tree small, and it means every node can point a diagnostic at exactly the
//! characters somebody typed rather than at a reconstruction of them.
//!
//! This is the tree of the **syntax**, not of the meaning. A declaration of three names
//! is one node here, and becomes three of something else later; nothing is desugared
//! while an error might still need to quote it.

use quench_diag::Span;

/// A whole file.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Program {
    /// Constants and functions, in the order they were written — which matters, because
    /// a constant may be built out of the ones above it.
    pub items: Vec<Item>,
    /// Where the program begins, if it says.
    pub start: Option<Start>,
}

/// Something at the top of a file. Not `START`, which is neither and is kept apart.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Item {
    /// `const.export.i64 ['LIMIT'] = [*100*];`
    ///
    /// The same syntax as a declaration, because it is one — only the keyword and where
    /// it is written differ. The chain says who can see it and what it is, and there is
    /// no link for whether it changes: a constant never does, and a link that only ever
    /// says one thing is noise rather than explicitness.
    Const(Var),
    Func(Func),
}

impl Item {
    pub fn span(&self) -> Span {
        match self {
            Item::Const(c) => c.span,
            Item::Func(f) => f.span,
        }
    }
}

/// `fn.export.i64 ['add'] [immut.i64 'a', immut.i64 'b'] { … }`
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Func {
    /// `fn`, then everything dotted after it — who can see it, and what it gives back.
    pub chain: Vec<Span>,
    /// `(2 3)` — the shape of what it gives back, when that is an array.
    pub shape: Vec<Span>,
    pub shape_span: Option<Span>,
    pub name: Span,
    /// Where the parameter list was written, empty or not, for pointing at when a call
    /// gives the wrong number of things.
    pub takes: Span,
    pub params: Vec<Param>,
    pub body: Vec<Stmt>,
    pub span: Span,
}

/// One parameter: a declaration's chain with `var` taken off, and a name.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Param {
    /// `immut.i64` — the same links `var` would carry, said the same way.
    pub chain: Vec<Span>,
    /// `(2 3)` — one size per `arr` link, where there is one. Empty when there is not.
    pub shape: Vec<Span>,
    pub shape_span: Option<Span>,
    pub name: Span,
    pub span: Span,
}

/// `START`, and everything after it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Start {
    /// The word itself, for pointing at.
    pub word: Span,
    pub body: Vec<Stmt>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Stmt {
    Print(Print),
    Var(Var),
    Set(Set),
    /// `add ['xs'] = [*7*];` — one more on the end of a growing array.
    Add(Set),
    If(If),
    Loop(Loop),
    /// `break;` — leave the innermost loop.
    Break(Span),
    /// `give ['a' + 'b'];` — the answer, and the end of the function.
    Give(Give),
    /// `greet[*Tankun*];` — a call written for what it does rather than for its answer.
    Do(Call),
}

/// `give [ … ];`, or `give;` from a function that gives nothing back.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Give {
    pub word: Span,
    pub value: Option<Value>,
    pub span: Span,
}

/// `add[*1*, *2*]` — a bare word before a bracket.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Call {
    pub name: Span,
    /// One value per argument, separated by commas.
    pub args: Vec<Value>,
    pub close: Span,
}

/// `loop.temp.range.i64 ['i'] = [*1*, *5*] { … }` or `loop.while … { … }`.
///
/// The chain reads like a declaration's, because a counting loop *is* one: how long the
/// counter lives, what kind of loop, what type the counter is.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Loop {
    /// `loop`, for pointing at.
    pub word: Span,
    /// Everything dotted after `loop`.
    pub chain: Vec<Span>,
    pub kind: LoopKind,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LoopKind {
    /// `['i'] = [*1*, *5*]` — a counter and two inclusive bounds.
    Range { name: Span, from: Value, to: Value },
    /// A condition, asked again before every pass.
    While(Value),
}

/// `if … { } else-if … { } else { }`
///
/// The condition wears no brackets, because `[ ]` holds a *list* everywhere else and a
/// condition is never a list. It runs until the `{`, which is unambiguous: `{` only ever
/// opens a block, so nothing inside an expression can be mistaken for the end of one.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct If {
    /// `if` and every `else-if`, asked in order.
    pub arms: Vec<Arm>,
    /// The `else`, if there is one. There is only ever one, and it comes last.
    pub otherwise: Option<Vec<Stmt>>,
    pub span: Span,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Arm {
    /// `if` or `else-if`, for pointing at.
    pub word: Span,
    pub condition: Value,
    pub body: Vec<Stmt>,
}

/// `set ['x'] = [*5*];` — changing something that already exists.
///
/// The same shape as a declaration minus the chain, because the variable already has a
/// type and saying it again would only be a chance to disagree.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Set {
    pub word: Span,
    pub targets: Vec<Place>,
    pub values: Vec<Value>,
    pub span: Span,
}

/// Somewhere a value can be put.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Place {
    Name(Span),
    At { name: Span, indices: Vec<Term>, close: Span },
}

impl Place {
    pub fn span(&self) -> Span {
        match self {
            Place::Name(s) => *s,
            Place::At { name, close, .. } => name.to(*close),
        }
    }

    /// The name being changed, without whatever follows it.
    pub fn name(&self) -> Span {
        match self {
            Place::Name(s) => *s,
            Place::At { name, .. } => *name,
        }
    }
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Print(p) => p.span,
            Stmt::Var(v) => v.span,
            Stmt::Set(s) => s.span,
            Stmt::Add(s) => s.span,
            Stmt::If(i) => i.span,
            Stmt::Loop(l) => l.span,
            Stmt::Break(s) => *s,
            Stmt::Give(g) => g.span,
            Stmt::Do(c) => c.name.to(c.close),
        }
    }
}

/// `print.stdout[str:*Hello* 'name' \n];`
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Print {
    pub word: Span,
    /// `stdout` or `stderr` — where it goes, said rather than assumed.
    pub to: Span,
    pub pieces: Vec<Piece>,
    pub span: Span,
}

/// `var.mut.b16 ['x', 'y'] = [*1*, *2*];`
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Var {
    /// `var`, then everything dotted after it — `mut`, the type, whatever else arrives.
    /// Kept as written so a diagnostic can point at one link rather than the whole line.
    pub chain: Vec<Span>,
    /// `(5 2)` — one size per `arr` link, outside in, and the innermost link takes
    /// whatever is left over. Empty when none was written.
    pub shape: Vec<Span>,
    /// Where the shape was written, for pointing at when it does not match the chain.
    pub shape_span: Option<Span>,
    pub names: Vec<Span>,
    /// One list of pieces per name, in the order the names were given.
    pub values: Vec<Value>,
    pub span: Span,
}

/// One value: operands, and whatever sits between them.
///
/// Deliberately **flat**. A tree would mean deciding what binds to what, and that is a
/// question about meaning rather than about what somebody typed — Quench keeps the
/// precedence mathematics settled and refuses the rest, and *refusing* is something only
/// a checker can do, since it has to say what the two readings were. So the syntax tree
/// records the sequence and the checker builds the tree, or declines to.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Value {
    /// The operands, in order. Always one longer than `between`, unless empty.
    pub terms: Vec<Term>,
    /// What sits between each pair of terms. `None` is juxtaposition — nothing written,
    /// which is how a list of pieces builds text.
    pub between: Vec<Option<Operator>>,
    pub span: Span,
}

impl Value {
    /// Whether any operator was written at all.
    pub fn has_operators(&self) -> bool {
        self.between.iter().any(Option::is_some)
    }
}

/// One operand.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Term {
    Piece(Piece),
    /// `[…]` inside a value — the elements of an array, juxtaposed.
    Elements { open: Span, of: Vec<Term>, close: Span },
    /// `'xs'[…]` or `'double'[…]` — a name and a bracketed list, which is an index
    /// when the name is a variable's and a call when it is a function's.
    ///
    /// The parser cannot tell those apart and does not try: both are a name between
    /// marks and a list of values, and which one it is depends on what the name was
    /// declared as. So the list is parsed the way a call's arguments are -- commas
    /// between values -- and an index unpacks the one value it gets, whose terms are
    /// its dimensions written side by side.
    At { name: Span, indices: Vec<Value>, close: Span },
    /// A bare number, which is only ever part of a shape.
    Number(Span),
    /// `count['xs']` — a bare word before a bracket is a call, where a quoted name
    /// before one is an index. That distinction was already in the language.
    Call(Call),
    /// `( … )` — which is how anything mathematics did not settle gets said.
    Group { open: Span, value: Box<Value>, close: Span },
    /// `not x`
    Not { word: Span, of: Box<Term> },
    /// `share 'xs'` or `copy 'xs'` — which of the two things binding an array to a
    /// second name could mean. Required, because they cost different things and
    /// neither cost should be paid by an omission.
    Handed { word: Span, copies: bool, of: Box<Term> },
}

impl Term {
    pub fn span(&self) -> Span {
        match self {
            Term::Piece(p) => p.span(),
            Term::Elements { open, close, .. } => open.to(*close),
            Term::At { name, close, .. } => name.to(*close),
            Term::Number(span) => *span,
            Term::Call(c) => c.name.to(c.close),
            Term::Group { open, close, .. } => open.to(*close),
            Term::Not { word, of } => word.to(of.span()),
            Term::Handed { word, of, .. } => word.to(of.span()),
        }
    }
}

/// An operator, and where it was written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Operator {
    pub kind: OpKind,
    pub span: Span,
}

/// What an operator does. Which of these bind tighter is [`quench_check`]'s business.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OpKind {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
    /// Remainder. Mathematics never settled where this sits, so it never binds against
    /// anything without brackets.
    Mod,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
    /// Invented by language designers rather than derived, so the same applies.
    And,
    Or,
}

impl OpKind {
    pub fn written(self) -> &'static str {
        match self {
            OpKind::Add => "+",
            OpKind::Sub => "-",
            OpKind::Mul => "x",
            OpKind::Div => "/",
            OpKind::Pow => "^",
            OpKind::Mod => "mod",
            OpKind::Lt => "<",
            OpKind::Gt => ">",
            OpKind::Le => "<==",
            OpKind::Ge => ">==",
            OpKind::Eq => "==",
            OpKind::Ne => "!==",
            OpKind::And => "and",
            OpKind::Or => "or",
        }
    }
}

/// One item in a list: something written, a name, or an escape.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Piece {
    /// `*1000*`, or `str:*1000*` where no chain supplies the type.
    Written { ty: Option<Span>, mark: Span },
    /// `'name'`
    Name(Span),
    /// `\n`
    Escape(Span),
    /// `'xs'[…]` — an index or a call, told apart by what the name was declared as.
    At { name: Span, indices: Vec<Value>, close: Span },
    /// `count['xs']` — the same call a value can hold, in a list.
    Call(Call),
}

impl Piece {
    pub fn span(&self) -> Span {
        match self {
            Piece::Written { ty: Some(ty), mark } => ty.to(*mark),
            Piece::Written { ty: None, mark } => *mark,
            Piece::Name(s) | Piece::Escape(s) => *s,
            Piece::At { name, close, .. } => name.to(*close),
            Piece::Call(c) => c.name.to(c.close),
        }
    }
}
