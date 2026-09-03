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
use quench_parse::{ast, Parsed};
use std::collections::HashMap;

/// A type, as far as the checker is concerned.
///
/// Quench's type list is longer than this. These are the ones that exist all the way
/// down — anything else is read, understood, and then honestly refused as not built.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ty {
    I64,
    Str,
    Bool,
    /// `arr.i64 (2 3)` — one allocation, laid out row by row.
    ///
    /// One `arr` link is one allocation however many dimensions it has, which is what
    /// makes indexing arithmetic. Two `arr` links would be an array of handles, and is
    /// a different type that is not built yet.
    Arr { of: Box<Ty>, shape: Vec<usize> },
}

impl Ty {
    pub fn name(&self) -> String {
        match self {
            Ty::I64 => "i64".to_string(),
            Ty::Str => "str".to_string(),
            Ty::Bool => "bool".to_string(),
            Ty::Arr { of, shape } => {
                let sizes: Vec<String> = shape.iter().map(usize::to_string).collect();
                format!("arr.{} ({})", of.name(), sizes.join(" "))
            }
        }
    }

    /// `a` or `an`, for the name of this type.
    ///
    /// Small, and worth having: a language whose selling point is its errors cannot
    /// write "a i64" in them.
    pub fn article(&self) -> &'static str {
        match self {
            Ty::I64 | Ty::Arr { .. } => "an",
            Ty::Str | Ty::Bool => "a",
        }
    }

    /// How many elements one of these holds, all told.
    pub fn count(&self) -> usize {
        match self {
            Ty::Arr { shape, .. } => shape.iter().product(),
            _ => 1,
        }
    }

    fn simple(word: &str) -> Option<Ty> {
        match word {
            "i64" => Some(Ty::I64),
            "str" => Some(Ty::Str),
            "bool" => Some(Ty::Bool),
            _ => None,
        }
    }
}

/// Every type Quench means to have, whether or not it is built.
///
/// Kept so that `b16` gets "not built yet" and `b17` gets "there is no such type",
/// which are different things and a reader deserves to know which happened.
const INTENDED: [&str; 17] = [
    "b16", "b32", "b64", "d32", "d64", "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "e",
    "bool", "str", "text",
];

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
}

/// Which variable. An index into [`Checked::locals`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LocalId(pub u32);

/// A value, with the syntax taken off it and the tree built.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Value {
    /// Text, with every piece already joined and every escape already meant.
    Text(String),
    Number(i64),
    Bool(bool),
    /// The value another variable holds.
    Copy(LocalId),
    Binary { op: OpKind, lhs: Box<Value>, rhs: Box<Value> },
    /// The elements of an array, flat and in order however many dimensions it has.
    Array(Vec<Value>),
    /// One element. The shape is carried so the lowering can work out where it is
    /// without going back to the type.
    At { array: Box<Value>, indices: Vec<Value>, shape: Vec<usize> },
}

pub use quench_parse::OpKind;
pub use quench_qir::Stream;

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
    /// `set` — changing something that already exists.
    Assign { to: Place, value: Value },
    Print { to: Stream, pieces: Vec<Printed> },
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
    Element { local: LocalId, indices: Vec<Value>, shape: Vec<usize> },
}

/// What a program means.
pub struct Checked {
    pub locals: Vec<Local>,
    pub body: Vec<Stmt>,
    pub errors: Vec<Diagnostic>,
    /// False when there was no `START` at all.
    pub has_start: bool,
}

impl Checked {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Read a whole file, parse it, and work out what it means.
pub fn check(source: &str) -> Checked {
    let Parsed { program, errors } = quench_parse::parse(source);
    let mut checker =
        Checker { source, locals: Vec::new(), scope: vec![HashMap::new()], body: Vec::new(), errors };

    let has_start = program.start.is_some();
    if let Some(start) = &program.start {
        for stmt in &start.body {
            checker.statement(stmt);
        }
    }

    Checked {
        locals: checker.locals,
        body: checker.body,
        errors: checker.errors,
        has_start,
    }
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
            ast::Stmt::Set(set) => self.set(set),
            ast::Stmt::If(conditional) => self.conditional(conditional),
        }
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
                    if !INTENDED.contains(&word) {
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
                Diagnostic::new("E0405", format!("`{word}` is not built yet."))
                    .primary(ty_span, "here")
                    .rule("Quench means to have this type, and does not have it today")
                    .tip("`i64`, `str` and `bool` are the ones that work all the way down.")
                    .fix("one of those for now"),
            );
        }

        if arrays.len() > 1 {
            self.errors.push(
                Diagnostic::new("E0423", "an array of arrays is not built yet.")
                    .primary(arrays[1], "this second `arr`")
                    .rule("one `arr` is one allocation, however many dimensions it has; two would be an array of handles")
                    .tip("a rectangular array does most of what nesting is wanted for: `arr.i64 (2 3)` is two rows of three.")
                    .fix("use one `arr` with more than one size"),
            );
            return None;
        }

        if let Some(arr) = arrays.first() {
            let Some(element) = ty else { return None };
            if !matches!(element, Ty::I64) {
                self.errors.push(
                    Diagnostic::new("E0424", format!("an array of `{}` is not built yet.", element.name()))
                        .primary(ty_span, "here")
                        .rule("the elements of an array are packed by width, and only `i64` is built that way today")
                        .fix("`arr.i64` for now"),
                );
                return None;
            }
            let shape = self.shape(var, *arr)?;
            ty = Some(Ty::Arr { of: Box::new(element), shape });
        } else if var.shape_span.is_some() {
            self.errors.push(
                Diagnostic::new("E0425", "only an array has a shape.")
                    .primary(var.shape_span.expect("just checked"), "here")
                    .rule("a shape says how many elements an array holds, and this declares no array")
                    .fix("add `arr` to the chain, or remove the shape"),
            );
            return None;
        }

        Some(Chain { mutable, ty, ty_span })
    }

    /// `(5)` or `(2 3)` — the sizes, which an array must have and must not be empty.
    fn shape(&mut self, var: &ast::Var, arr: Span) -> Option<Vec<usize>> {
        let Some(span) = var.shape_span else {
            self.errors.push(
                Diagnostic::new("E0426", "this array does not say how big it is.")
                    .primary(arr, "here")
                    .rule("an array says its size in brackets after the chain, because the size is part of the type")
                    .tip("a growing array is not built yet, so every one has to say.")
                    .fix("`arr.i64 (5)`, or whichever size was meant"),
            );
            return None;
        };
        if var.shape.is_empty() {
            self.errors.push(
                Diagnostic::new("E0427", "this shape is empty.")
                    .primary(span, "here")
                    .rule("a shape is one size for each dimension, and there is always at least one")
                    .fix("`(5)`, or whichever size was meant"),
            );
            return None;
        }
        let mut sizes = Vec::new();
        for size in &var.shape {
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
                Ok(n) => sizes.push(n),
                Err(_) => {
                    self.errors.push(
                        Diagnostic::new("E0429", format!("`{}` is not a size.", self.text(*size)))
                            .primary(*size, "here")
                            .rule("a size is a whole number, written without marks, because it is part of a type")
                            .fix("write a whole number"),
                    );
                    return None;
                }
            }
        }
        Some(sizes)
    }

    // --- values ---------------------------------------------------------------------

    fn value(&mut self, value: &ast::Value, ty: &Ty, ty_span: Span) -> Option<Value> {
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

        let built = self.tree(value)?;
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
            let local = self.lookup(*span)?;
            let held = self.locals[local.0 as usize].ty.clone();
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
            return Some(Value::Copy(local));
        }
        // A single bracketed value is just that value.
        if let [ast::Term::Group { value: inner, .. }] = value.terms.as_slice() {
            return self.value(inner, ty, ty_span);
        }

        // An array: its elements, juxtaposed, flat however many dimensions it has.
        if let Ty::Arr { of, shape } = ty {
            return self.array(value, of, shape, ty_span);
        }

        match ty {
            Ty::Arr { .. } => unreachable!("handled above"),
            Ty::Str => {
                let mut out = String::new();
                for term in &value.terms {
                    let ast::Term::Piece(piece) = term else {
                        self.errors.push(
                            Diagnostic::new("E0415", "brackets group something to work out, and text is not worked out.")
                                .primary(term.span(), "here")
                                .rule("text is a list of pieces, written side by side")
                                .fix("remove the brackets"),
                        );
                        return None;
                    };
                    out.push_str(&self.literal(piece)?);
                }
                Some(Value::Text(out))
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
            Ty::I64 => match value.terms.as_slice() {
                [ast::Term::Piece(ast::Piece::Written { ty: None, mark })] => {
                    let digits = unmarked(self.text(*mark));
                    match digits.parse::<i64>() {
                        Ok(n) => Some(Value::Number(n)),
                        Err(_) => {
                            self.errors.push(
                                Diagnostic::new("E0407", format!("`{digits}` is not a whole number."))
                                    .primary(*mark, "here")
                                    .rule("a written value is read by the type it is given to, and `i64` reads whole numbers")
                                    .tip("`i64` holds -9223372036854775808 to 9223372036854775807.")
                                    .fix("write a whole number, or declare it a type that fits this"),
                            );
                            None
                        }
                    }
                }
                [] => None,
                // Exactly one thing that is not a written value -- a bare number, an
                // index, brackets. Whatever it is, `term` knows what is wrong with it
                // specifically, and "not several" would be a poor description of one.
                [one] => {
                    let built = self.term(one)?;
                    let found = self.type_of(&built, one.span())?;
                    if found != Ty::I64 {
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
        ty_span: Span,
    ) -> Option<Value> {
        let [ast::Term::Elements { of: written, open, close }] = value.terms.as_slice() else {
            self.errors.push(
                Diagnostic::new("E0430", "an array is written between brackets.")
                    .primary(value.span, "here")
                    .rule("the elements go in a list of their own, inside the value")
                    .fix("`[[*1* *2* *3*]]`"),
            );
            return None;
        };

        let wanted: usize = shape.iter().product();
        if written.len() != wanted {
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

        let mut elements = Vec::with_capacity(written.len());
        for term in written {
            let built = self.term(term)?;
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
        Some(Value::Array(elements))
    }

    /// `'xs'[…]` — one element.
    fn at(&mut self, name: Span, indices: &[ast::Term], close: Span) -> Option<Value> {
        let local = self.lookup(name)?;
        let held = self.locals[local.0 as usize].ty.clone();
        let Ty::Arr { of: _, shape } = held else {
            self.errors.push(
                Diagnostic::new("E0433", format!("`{}` is not an array.", held.name()))
                    .primary(name, format!("a `{}`", held.name()))
                    .rule("only an array has elements to index")
                    .fix("index an array, or use the value on its own"),
            );
            return None;
        };

        if indices.len() != shape.len() {
            let sizes: Vec<String> = shape.iter().map(usize::to_string).collect();
            self.errors.push(
                Diagnostic::new("E0434", format!(
                    "this array has {} dimension(s), and {} index(es) were given.",
                    shape.len(),
                    indices.len()
                ))
                .primary(name.to(close), format!("{} here", indices.len()))
                .secondary(self.locals[local.0 as usize].at, format!("declared ({})", sizes.join(" ")))
                .rule("an index gives one number for each dimension, in the order the shape wrote them")
                .fix(format!("give {} of them", shape.len())),
            );
            return None;
        }

        let mut built = Vec::with_capacity(indices.len());
        for index in indices {
            let value = self.term(index)?;
            let found = self.type_of(&value, index.span())?;
            if found != Ty::I64 {
                self.errors.push(
                    Diagnostic::new("E0435", format!("an index is a number, and this is {} `{}`.", found.article(), found.name()))
                        .primary(index.span(), "here")
                        .rule("an element is found by counting, and counting is done with numbers")
                        .fix("use a whole number"),
                );
                return None;
            }
            built.push(value);
        }

        Some(Value::At { array: Box::new(Value::Copy(local)), indices: built, shape })
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
            ast::Term::At { name, indices, close } => self.at(*name, indices, *close),
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
            ast::Term::Not { word, .. } => {
                self.errors.push(
                    Diagnostic::new("E0418", "`not` is not built yet.")
                        .primary(*word, "here")
                        .rule("the parts of Quench arrive one at a time, and this one has not")
                        .fix("compare with `==` or `!=` for now"),
                );
                None
            }
            ast::Term::Piece(ast::Piece::Name(span)) => {
                let local = self.lookup(*span)?;
                Some(Value::Copy(local))
            }
            ast::Term::Piece(ast::Piece::Written { ty: None, mark }) => {
                let digits = unmarked(self.text(*mark));
                match digits.parse::<i64>() {
                    Ok(n) => Some(Value::Number(n)),
                    Err(_) => match digits.as_str() {
                        "true" => Some(Value::Bool(true)),
                        "false" => Some(Value::Bool(false)),
                        _ => {
                            self.errors.push(
                                Diagnostic::new("E0407", format!("`{digits}` is not a whole number."))
                                    .primary(*mark, "here")
                                    .rule("a written value in a sum is read as a number")
                                    .fix("write a whole number"),
                            );
                            None
                        }
                    },
                }
            }
            ast::Term::Piece(ast::Piece::Written { ty: Some(span), .. }) => {
                self.errors.push(
                    Diagnostic::new("E0409", "this value says its type twice.")
                        .primary(*span, "said here")
                        .rule("a declaration's chain already says the type, so its values do not repeat it")
                        .fix("remove it"),
                );
                None
            }
            ast::Term::Piece(ast::Piece::At { name, indices, close }) => {
                self.at(*name, indices, *close)
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
            Value::Number(_) => Some(Ty::I64),
            Value::Bool(_) => Some(Ty::Bool),
            Value::Copy(local) => Some(self.locals[local.0 as usize].ty.clone()),
            Value::Array(_) => None,
            Value::At { shape, array, .. } => {
                let Value::Copy(local) = **array else { return None };
                let _ = shape;
                match self.locals[local.0 as usize].ty.clone() {
                    Ty::Arr { of, .. } => Some(*of),
                    other => Some(other),
                }
            }
            Value::Binary { op, lhs, rhs } => {
                if matches!(op, OpKind::Pow | OpKind::And | OpKind::Or) {
                    self.errors.push(
                        Diagnostic::new("E0422", format!("`{}` is not built yet.", op.written()))
                            .primary(span, "here")
                            .rule("the parts of Quench arrive one at a time, and this one has not")
                            .tip("`+`, `-`, `x`, `/`, `mod` and the comparisons work.")
                            .fix("use one of those for now"),
                    );
                    return None;
                }
                let (l, r) = (self.type_of(lhs, span)?, self.type_of(rhs, span)?);

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

                if l != Ty::I64 || r != Ty::I64 {
                    self.errors.push(
                        Diagnostic::new("E0420", format!("`{}` works on numbers.", op.written()))
                            .primary(span, format!("{} `{}` and {} `{}`", l.article(), l.name(), r.article(), r.name()))
                            .rule("arithmetic and ordering are for numbers, and nothing converts on its own")
                            .fix("use numbers on both sides"),
                    );
                    return None;
                }
                Some(if is_comparison(*op) { Ty::Bool } else { Ty::I64 })
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
            ast::Piece::Written { ty: Some(span), .. } => {
                self.errors.push(
                    Diagnostic::new("E0409", "this value says its type twice.")
                        .primary(*span, "said here")
                        .rule("a declaration's chain already says the type, so its values do not repeat it")
                        .fix("remove it"),
                );
                None
            }
            ast::Piece::Written { ty: None, mark } => Some(unmarked(self.text(*mark))),
            ast::Piece::Escape(span) => escape(self.text(*span)).map(str::to_string).or_else(|| {
                self.errors.push(
                    Diagnostic::new("E0410", format!("`{}` is not an escape.", self.text(*span)))
                        .primary(*span, "here")
                        .rule("the escapes are `\\n`, `\\t`, `\\r` and `\\\\`"),
                );
                None
            }),
            ast::Piece::At { name, close, .. } => {
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
                    Ty::I64 => "compare it against something, such as `> *0*`",
                    _ => "compare it against something",
                }),
            );
            return None;
        }
        Some(built)
    }

    /// Check a block with a scope of its own, and hand back what it means.
    fn scoped(&mut self, body: &[ast::Stmt]) -> Vec<Stmt> {
        self.scope.push(HashMap::new());
        let outer = std::mem::take(&mut self.body);
        for stmt in body {
            self.statement(stmt);
        }
        let inner = std::mem::replace(&mut self.body, outer);
        self.scope.pop();
        inner
    }

    // --- changing ---------------------------------------------------------------------

    fn set(&mut self, set: &ast::Set) {
        for (n, target) in set.targets.iter().enumerate() {
            let Some(local) = self.lookup(target.name()) else { continue };
            let held = self.locals[local.0 as usize].ty.clone();

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

            match target {
                ast::Place::Name(span) => {
                    let Some(built) = self.value(value, &held, *span) else { continue };
                    self.body.push(Stmt::Assign { to: Place::Local(local), value: built });
                }
                ast::Place::At { name, indices, close } => {
                    let Ty::Arr { of, shape } = held else {
                        self.errors.push(
                            Diagnostic::new("E0433", format!("`{}` is not an array.", held.name()))
                                .primary(*name, format!("a `{}`", held.name()))
                                .rule("only an array has elements to change")
                                .fix("change the whole thing, or index an array"),
                        );
                        continue;
                    };
                    let Some(Value::At { indices: built, .. }) =
                        self.at(*name, indices, *close)
                    else {
                        continue;
                    };
                    let Some(value) = self.value(value, &of, *name) else { continue };
                    self.body.push(Stmt::Assign {
                        to: Place::Element { local, indices: built, shape },
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
                    let Some(local) = self.lookup(*span) else { continue };
                    let ty = self.locals[local.0 as usize].ty.clone();
                    pieces.push(Printed::Value { value: Value::Copy(local), ty });
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
                        Some(Ty::I64) => {
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
                            Diagnostic::new("E0405", format!("`{word}` is not built yet."))
                                .primary(*span, "here")
                                .rule("Quench means to have this type, and does not have it today")
                                .fix("`i64` or `str` for now"),
                        ),
                    }
                }
                ast::Piece::At { name, indices, close } => {
                    let Some(value) = self.at(*name, indices, *close) else { continue };
                    let Some(ty) = self.type_of(&value, name.to(*close)) else { continue };
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

    fn lookup(&mut self, span: Span) -> Option<LocalId> {
        let name = self.named(span);
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
