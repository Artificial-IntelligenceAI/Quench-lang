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
    I64,
    Str,
    Bool,
    /// `e` — a number held exactly, however large it grows.
    ///
    /// Not a size. Every other number type in Quench says how many bits it has, and
    /// this one says instead that it never rounds and never overflows: `*1*` divided by
    /// `*3*` is a third, and a third times three is one. What that costs is that an `e`
    /// lives on the heap and a `b64` lives in a register.
    Exact,
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
            Ty::Exact => "e".to_string(),
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
            Ty::I64 | Ty::Arr { .. } | Ty::Exact => "an",
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
            "e" => Some(Ty::Exact),
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
    Number(i64),
    Bool(bool),
    /// An `e`, kept as the text it was written with. Reading it is the runtime's job,
    /// because the answer does not fit in anything the IR can carry.
    Exact(String),
    /// The value another variable holds.
    Copy(LocalId),
    Binary { op: OpKind, lhs: Box<Value>, rhs: Box<Value> },
    /// The elements of an array, flat and in order however many dimensions it has.
    Array(Vec<Value>),
    /// One element. The shape is carried so the lowering can work out where it is
    /// without going back to the type.
    At { array: Box<Value>, indices: Vec<Value>, shape: Vec<usize> },
    /// `not 'ready'` — the opposite of a `bool`.
    Not(Box<Value>),
    /// A top-level constant, written in where it was named.
    Const(u32),
    /// `add[*1*, *2*]` — the answer a function gave back.
    Call { func: u32, args: Vec<Value> },
}

pub use quench_parse::OpKind;
pub use quench_qir::Stream;

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
    Element { local: LocalId, indices: Vec<Value>, shape: Vec<usize> },
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
        reading: Ty::I64,
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
            ast::Stmt::Set(set) => self.set(set),
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

            if matches!(ty, Ty::Arr { .. }) {
                self.errors.push(
                    Diagnostic::new("E0460", "a constant array is not built yet.")
                        .primary(at, "here")
                        .rule("a constant is written in wherever it is named, and an array is a thing rather than a value")
                        .tip("an array wants somewhere to live, and a constant has nowhere.")
                        .fix("declare it inside `START` with `var` for now"),
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
        if name == "count" {
            self.errors.push(
                Diagnostic::new("E0462", "`count` is already something.")
                    .primary(func.name, "here")
                    .rule("`count` answers how many elements an array holds, and is not yours to redefine")
                    .fix("pick another name"),
            );
            return;
        }

        // The type is whatever the chain says that is not `fn` and not a visibility.
        let mut returns = None;
        let mut said = false;
        for link in func.chain.iter().skip(1) {
            let word = self.text(*link);
            if Visibility::from_word(word).is_some() {
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
            if word == "nothing" {
                continue;
            }
            match self.a_type(*link) {
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
        for link in &param.chain {
            match self.text(*link) {
                word @ ("mut" | "immut") if ty_span.is_none() => mutable = Some(word == "mut"),
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
        self.a_type(ty_span)
    }

    /// One type link, understood or honestly refused.
    fn a_type(&mut self, link: Span) -> Option<Ty> {
        let word = self.text(link);
        if let Some(ty) = Ty::simple(word) {
            return Some(ty);
        }
        if INTENDED.contains(&word) {
            self.errors.push(
                Diagnostic::new("E0405", format!("`{word}` is not built yet."))
                    .primary(link, "here")
                    .rule("Quench means to have this type, and does not have it today")
                    .tip("`i64`, `str` and `bool` are the ones that work all the way down.")
                    .fix("one of those for now"),
            );
        } else {
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
        // A value that is one call is that call's answer, whatever the type -- the same
        // way a value that is one name is that variable's. Neither is a list of pieces,
        // and reading either as one would ask the wrong question about it.
        if let [term @ (ast::Term::Call(_)
        | ast::Term::Piece(ast::Piece::Call(_))
        | ast::Term::Not { .. })] = value.terms.as_slice()
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
            _ => Ty::I64,
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
        if let Ty::Arr { of, shape } = ty {
            return self.array(value, of, shape, ty_span);
        }

        match ty {
            Ty::Arr { .. } => unreachable!("handled above"),
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

    /// `count['xs']` — how many elements an array holds, all told.
    ///
    /// The answer is known here, because a shape is written down in the declaration and
    /// never changes. So this is a number by the time anything runs, and a loop bounded
    /// by it costs nothing at all.
    fn call(&mut self, call: &ast::Call) -> Option<Value> {
        if self.text(call.name) != "count" {
            let (which, args) = self.invocation(call)?;
            if self.signatures[which as usize].returns.is_none() {
                let name = self.signatures[which as usize].name.clone();
                let at = self.signatures[which as usize].at;
                self.errors.push(
                    Diagnostic::new("E0471", format!("`'{name}'` gives `nothing` back, and this wants a value."))
                        .secondary(at, "declared `nothing` here")
                        .primary(call.name.to(call.close), "and its answer is wanted here")
                        .rule("`nothing` means there is no answer, and there is no value to stand in for one")
                        .fix("call it on its own line, or have it give something back"),
                );
                return None;
            }
            return Some(Value::Call { func: which, args });
        }

        let [one] = call.args.as_slice() else {
            self.errors.push(
                Diagnostic::new("E0456", "`count` counts one array.")
                    .primary(call.name.to(call.close), "here")
                    .rule("`count` takes the name of an array, and nothing else")
                    .fix("`count['xs']`"),
            );
            return None;
        };
        let [ast::Term::Piece(ast::Piece::Name(of))] = one.terms.as_slice() else {
            self.errors.push(
                Diagnostic::new("E0456", "`count` counts one array.")
                    .primary(one.span, "here")
                    .rule("`count` takes the name of an array, and nothing else")
                    .fix("`count['xs']`"),
            );
            return None;
        };

        let local = self.lookup(*of)?;
        match &self.locals[local.0 as usize].ty {
            Ty::Arr { shape, .. } => Some(Value::Number(shape.iter().product::<usize>() as i64)),
            other => {
                let other = other.clone();
                self.errors.push(
                    Diagnostic::new("E0457", format!("`count` was given {} `{}`.", other.article(), other.name()))
                        .primary(*of, format!("{} `{}`", other.article(), other.name()))
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
            ast::Term::At { name, indices, close } => self.at(*name, indices, *close),
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
                                    .tip("`e:*0.1*` is how a value says its own type where the chain does not.")
                                    .fix("write a whole number, or say `e:` in front of it"),
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
                    Some(Ty::I64) => match digits.parse::<i64>() {
                        Ok(n) => Some(Value::Number(n)),
                        Err(_) => {
                            self.errors.push(
                                Diagnostic::new("E0407", format!("`{digits}` is not a whole number."))
                                    .primary(*mark, "here")
                                    .rule("a written value in a sum is read as a number")
                                    .fix("write a whole number"),
                            );
                            None
                        }
                    },
                    _ => {
                        self.errors.push(
                            Diagnostic::new("E0409", format!("`{word}` has nothing to do in a sum."))
                                .primary(*span, "said here")
                                .rule("a value in a sum says a number type or says nothing, and the chain says the rest")
                                .fix("`e:` for an exact number, or nothing at all"),
                        );
                        None
                    }
                }
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
            Value::Exact(_) => Some(Ty::Exact),
            Value::Bool(_) => Some(Ty::Bool),
            Value::Copy(local) => Some(self.locals[local.0 as usize].ty.clone()),
            Value::Array(_) => None,
            Value::Not(_) => Some(Ty::Bool),
            Value::Const(which) => Some(self.constants[*which as usize].ty.clone()),
            Value::Call { func, .. } => self.signatures[*func as usize].returns.clone(),
            Value::At { shape, array, .. } => {
                let Value::Copy(local) = **array else { return None };
                let _ = shape;
                match self.locals[local.0 as usize].ty.clone() {
                    Ty::Arr { of, .. } => Some(*of),
                    other => Some(other),
                }
            }
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

                if l != r || !matches!(l, Ty::I64 | Ty::Exact) {
                    self.errors.push(
                        Diagnostic::new("E0420", format!("`{}` works on numbers.", op.written()))
                            .primary(span, format!("{} `{}` and {} `{}`", l.article(), l.name(), r.article(), r.name()))
                            .rule("arithmetic and ordering are for numbers, and nothing converts on its own")
                            .tip("an `i64` and an `e` are both numbers and are not the same number, so neither becomes the other.")
                            .fix("use the same kind of number on both sides"),
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
        let name = self.text(call.name);
        if name == "count" {
            self.errors.push(
                Diagnostic::new("E0469", "`count` answers a question and does nothing else.")
                    .primary(call.name.to(call.close), "here")
                    .rule("a call written on its own is written for what it does, and this does nothing")
                    .fix("use the answer, or remove the line"),
            );
            return;
        }
        let Some((which, args)) = self.invocation(call) else { return };
        self.body.push(Stmt::Do { func: which, args });
    }

    /// Look a call up, and check what it was given against what it takes.
    fn invocation(&mut self, call: &ast::Call) -> Option<(u32, Vec<Value>)> {
        let name = self.text(call.name).to_string();
        let Some(&which) = self.named.get(&name) else {
            self.errors.push(
                Diagnostic::new("E0455", format!("there is nothing called `{name}`."))
                    .primary(call.name, "here")
                    .rule("a bare word before a bracket is a call, and this names no function")
                    .tip("`count` is the one that comes with the language.")
                    .fix("check the spelling, or declare it with `fn`"),
            );
            return None;
        };

        let signature = &self.signatures[which as usize];
        let (wanted, at, list) = (signature.takes.clone(), signature.at, signature.list);
        if call.args.len() != wanted.len() {
            self.errors.push(
                Diagnostic::new(
                    "E0470",
                    format!(
                        "`'{name}'` takes {}, and was given {}.",
                        counted(wanted.len(), "thing"),
                        counted(call.args.len(), "thing")
                    ),
                )
                .secondary(list, format!("takes {}", counted(wanted.len(), "thing")))
                .primary(call.name.to(call.close), format!("given {}", counted(call.args.len(), "thing")))
                .rule("a call brings one value for each parameter, in the same order")
                .fix("add what is missing, or take away what is spare"),
            );
            return None;
        }

        let mut args = Vec::new();
        for (given, ty) in call.args.iter().zip(&wanted) {
            args.push(self.value(given, ty, at)?);
        }
        Some((which, args))
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
                    if !INTENDED.contains(&word) {
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
        let Some(from) = self.value(from, &Ty::I64, name) else { return };
        let Some(to) = self.value(to, &Ty::I64, name) else { return };

        let counter = LocalId(self.locals.len() as u32);
        debug_assert_eq!(counter.0, live);
        let text = self.named(name);
        self.locals.push(Local {
            counter: true,
            name: text.clone(),
            ty: Ty::I64,
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

    fn set(&mut self, set: &ast::Set) {
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
                    .primary(span, "and wanted somewhere it lives, here")
                    .rule("a constant is written in wherever it is named, so there is no storage to index or change")
                    .tip("that is what makes it a constant rather than a variable that nobody assigns to.")
                    .fix("declare a `var` inside the function, from this"),
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
