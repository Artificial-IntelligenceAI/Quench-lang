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
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Print(p) => p.span,
            Stmt::Var(v) => v.span,
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
    pub names: Vec<Span>,
    /// One list of pieces per name, in the order the names were given.
    pub values: Vec<Value>,
    pub span: Span,
}

/// One value: as many pieces as it likes, juxtaposed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Value {
    pub pieces: Vec<Piece>,
    pub span: Span,
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
}

impl Piece {
    pub fn span(&self) -> Span {
        match self {
            Piece::Written { ty: Some(ty), mark } => ty.to(*mark),
            Piece::Written { ty: None, mark } => *mark,
            Piece::Name(s) | Piece::Escape(s) => *s,
        }
    }
}
