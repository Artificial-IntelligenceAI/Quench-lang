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
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ty {
    I64,
    Str,
}

impl Ty {
    pub fn name(self) -> &'static str {
        match self {
            Ty::I64 => "i64",
            Ty::Str => "str",
        }
    }

    fn of(word: &str) -> Option<Ty> {
        match word {
            "i64" => Some(Ty::I64),
            "str" => Some(Ty::Str),
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
}

/// Which variable. An index into [`Checked::locals`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LocalId(pub u32);

/// A value, with the syntax taken off it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Value {
    /// Text, with every piece already joined and every escape already meant.
    Text(String),
    Number(i64),
    /// The value another variable holds.
    Copy(LocalId),
}

/// One thing to print.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Printed {
    Text(String),
    Local { local: LocalId, ty: Ty },
}

/// A statement, resolved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Stmt {
    Declare { local: LocalId, value: Value },
    Print(Vec<Printed>),
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
        Checker { source, locals: Vec::new(), scope: HashMap::new(), body: Vec::new(), errors };

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
    /// Name to declaration. One scope for now, because there is only one block.
    scope: HashMap<String, LocalId>,
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
            if let Some(before) = self.scope.get(&name) {
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

            let Some(ty) = chain.ty else { continue };
            let Some(value) = var.values.get(n) else { continue };
            let Some(value) = self.value(value, ty, chain.ty_span) else { continue };

            let local = LocalId(self.locals.len() as u32);
            self.locals.push(Local {
                name: name.clone(),
                ty,
                mutable: chain.mutable,
                at: *name_span,
            });
            self.scope.insert(name, local);
            self.body.push(Stmt::Declare { local, value });
        }
    }

    /// `var` . `mut`? . type — and nothing else, yet.
    fn chain(&mut self, var: &ast::Var) -> Option<Chain> {
        let mut mutable = false;
        let mut ty_span = None;

        for link in var.chain.iter().skip(1) {
            match self.text(*link) {
                "mut" if !mutable && ty_span.is_none() => mutable = true,
                "mut" => {
                    self.errors.push(
                        Diagnostic::new("E0401", "`mut` comes before the type.")
                            .primary(*link, "here")
                            .rule("a chain reads `var`, then whether it changes, then what it is")
                            .fix("`var.mut.<type>`"),
                    );
                }
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
        let ty = Ty::of(word);
        if ty.is_none() {
            self.errors.push(
                Diagnostic::new("E0405", format!("`{word}` is not built yet."))
                    .primary(ty_span, "here")
                    .rule("Quench means to have this type, and does not have it today")
                    .tip("`i64` and `str` are the two that work all the way down.")
                    .fix("`i64` or `str` for now"),
            );
        }
        Some(Chain { mutable, ty, ty_span })
    }

    // --- values ---------------------------------------------------------------------

    fn value(&mut self, value: &ast::Value, ty: Ty, ty_span: Span) -> Option<Value> {
        // A value that is one name is that variable's value, whatever the type.
        if let [ast::Piece::Name(span)] = value.pieces.as_slice() {
            let local = self.lookup(*span)?;
            let held = self.locals[local.0 as usize].ty;
            if held != ty {
                self.errors.push(
                    Diagnostic::new("E0406", format!("this is `{}`, and it is being given to a `{}`.", held.name(), ty.name()))
                        .primary(*span, format!("a `{}`", held.name()))
                        .secondary(ty_span, format!("declared `{}` here", ty.name()))
                        .rule("nothing converts on its own — two types meet only where something says they should")
                        .fix("declare it the same type, or write the value out"),
                );
                return None;
            }
            return Some(Value::Copy(local));
        }

        match ty {
            // Text is a list: every piece in order, joined now because they are all
            // known now. Joining later would mean allocating at run time to build
            // something that was never going to change.
            Ty::Str => {
                let mut out = String::new();
                for piece in &value.pieces {
                    out.push_str(&self.literal(piece)?);
                }
                Some(Value::Text(out))
            }
            Ty::I64 => match value.pieces.as_slice() {
                [ast::Piece::Written { ty: None, mark }] => {
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
                pieces => {
                    // Juxtaposition builds text. It has no meaning for a number, and
                    // saying so is more use than a general "wrong shape".
                    let all = pieces[0].span().to(pieces[pieces.len() - 1].span());
                    self.errors.push(
                        Diagnostic::new("E0408", "a number is one written value, not several.")
                            .primary(all, format!("{} pieces here", pieces.len()))
                            .secondary(ty_span, "declared `i64` here")
                            .rule("pieces side by side build text; a number is written once")
                            .tip("`str` is the type where a value is a list of pieces.")
                            .fix("write it as one value, or add them with `+` once arithmetic is built"),
                    );
                    None
                }
            },
        }
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

    // --- printing ---------------------------------------------------------------------

    fn print(&mut self, print: &ast::Print) {
        let mut pieces = Vec::new();
        for piece in &print.pieces {
            match piece {
                ast::Piece::Name(span) => {
                    let Some(local) = self.lookup(*span) else { continue };
                    let ty = self.locals[local.0 as usize].ty;
                    pieces.push(Printed::Local { local, ty });
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
                    match Ty::of(word) {
                        Some(Ty::Str) => pieces.push(Printed::Text(unmarked(self.text(*mark)))),
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
        self.body.push(Stmt::Print(pieces));
    }

    fn lookup(&mut self, span: Span) -> Option<LocalId> {
        let name = self.named(span);
        match self.scope.get(&name) {
            Some(local) => Some(*local),
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
            .keys()
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
