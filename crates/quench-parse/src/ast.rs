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
    /// Where the program begins, if it says.
    pub start: Option<Start>,
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
        }
    }
}

/// `print[str:*Hello* 'name' \n];`
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Print {
    pub word: Span,
    pub pieces: Vec<Piece>,
    pub span: Span,
}

/// `var.mut.b16 ['x', 'y'] = [*1*, *2*];`
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Var {
    /// `var`, then everything dotted after it — `mut`, the type, whatever else arrives.
    /// Kept as written so a diagnostic can point at one link rather than the whole line.
    pub chain: Vec<Span>,
    /// `(5 2)` — one size per `arr` link, outside in. Empty when none was written.
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
    /// `'xs'[…]` — one element of an array. The indices are juxtaposed, matching the
    /// shape they index into.
    At { name: Span, indices: Vec<Term>, close: Span },
    /// A bare number, which is only ever part of a shape.
    Number(Span),
    /// `( … )` — which is how anything mathematics did not settle gets said.
    Group { open: Span, value: Box<Value>, close: Span },
    /// `not x`
    Not { word: Span, of: Box<Term> },
}

impl Term {
    pub fn span(&self) -> Span {
        match self {
            Term::Piece(p) => p.span(),
            Term::Elements { open, close, .. } => open.to(*close),
            Term::At { name, close, .. } => name.to(*close),
            Term::Number(span) => *span,
            Term::Group { open, close, .. } => open.to(*close),
            Term::Not { word, of } => word.to(of.span()),
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
            OpKind::Le => "</=",
            OpKind::Ge => ">/=",
            OpKind::Eq => "==",
            OpKind::Ne => "!=",
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
    /// `'xs'[…]` — one element of an array.
    At { name: Span, indices: Vec<Term>, close: Span },
}

impl Piece {
    pub fn span(&self) -> Span {
        match self {
            Piece::Written { ty: Some(ty), mark } => ty.to(*mark),
            Piece::Written { ty: None, mark } => *mark,
            Piece::Name(s) | Piece::Escape(s) => *s,
            Piece::At { name, close, .. } => name.to(*close),
        }
    }
}
