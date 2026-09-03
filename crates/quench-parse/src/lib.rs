//! Tokens into a tree, and a great deal to say when they will not go.
//!
//! A parser has more to say about a mistake than a lexer does, because it knows what it
//! was expecting. That is most of the value here: the tree is a straightforward walk
//! over a token list, and the interesting code is the part that runs when the walk fails.
//!
//! Two rules it follows:
//!
//! - **Never stop at the first mistake.** A statement that goes wrong is abandoned at the
//!   next `;`, and parsing resumes. A file with four mistakes should report four.
//! - **Never report the same mistake twice.** Recovery that produces a cascade of
//!   invented errors is worse than stopping, because a reader cannot tell which of them
//!   was real.

pub mod ast;

pub use ast::{Arm, If, OpKind, Operator, Piece, Place, Print, Program, Set, Start, Stmt, Term, Value, Var};

use quench_diag::{Diagnostic, Span};
use quench_lex::{Kind, Token};

/// What a file turned into, and everything wrong with it.
#[derive(Clone, Debug)]
pub struct Parsed {
    pub program: Program,
    pub errors: Vec<Diagnostic>,
}

impl Parsed {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Read a whole file, lexing it on the way.
pub fn parse(source: &str) -> Parsed {
    let lexed = quench_lex::lex(source);
    let mut parser =
        Parser { source, tokens: lexed.tokens, at: 0, errors: lexed.errors };
    let program = parser.program();
    Parsed { program, errors: parser.errors }
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    at: usize,
    errors: Vec<Diagnostic>,
}

impl<'a> Parser<'a> {
    // --- looking around -------------------------------------------------------------

    fn peek(&self) -> Token {
        self.tokens[self.at.min(self.tokens.len() - 1)]
    }

    fn text(&self, span: Span) -> &'a str {
        &self.source[span.start..span.end]
    }

    fn at_end(&self) -> bool {
        self.peek().kind == Kind::End
    }

    fn bump(&mut self) -> Token {
        let token = self.peek();
        if token.kind != Kind::End {
            self.at += 1;
        }
        token
    }

    fn eat(&mut self, kind: Kind) -> Option<Span> {
        if self.peek().kind == kind { Some(self.bump().span) } else { None }
    }

    /// Take a token of the given kind, or say what was there instead.
    fn expect(&mut self, kind: Kind, doing: &str) -> Option<Span> {
        if let Some(span) = self.eat(kind) {
            return Some(span);
        }
        let found = self.peek();
        self.errors.push(
            Diagnostic::new("E0101", format!("{doing} wants {} here.", kind.describe()))
                .primary(found.span, format!("found {}", found.kind.describe()))
                .rule(format!("{doing} is written with {}", kind.describe()))
                .fix(format!("put {} here", kind.describe())),
        );
        None
    }

    /// Give up on this statement and start again after the next `;`.
    ///
    /// The semicolon is what makes recovery honest: a parser that guessed where a
    /// statement ended would invent errors about the guess.
    fn recover(&mut self) {
        while !self.at_end() {
            if self.bump().kind == Kind::Semicolon {
                return;
            }
        }
    }

    // --- the grammar ----------------------------------------------------------------

    fn program(&mut self) -> Program {
        let mut program = Program::default();

        while !self.at_end() {
            let token = self.peek();
            if token.kind == Kind::Word && self.text(token.span) == quench_qir_entry() {
                let word = self.bump().span;
                match self.block() {
                    Some(body) => program.start = Some(Start { word, body }),
                    None => self.recover(),
                }
                continue;
            }

            self.errors.push(
                Diagnostic::new("E0102", "only `START` can be at the top of a file so far.")
                    .primary(token.span, "this is not `START`")
                    .rule("a file holds declarations and one `START`, and nothing else runs")
                    .tip("declaring things outside `START` is not built yet.")
                    .fix("move this after `START`"),
            );
            self.recover();
        }

        program
    }

    /// `{ … }` — the statements a block holds.
    ///
    /// The closing brace is what says where it ends, which is also what lets a file hold
    /// something after it.
    fn block(&mut self) -> Option<Vec<Stmt>> {
        let open = self.expect(Kind::OpenBlock, "a block")?;
        let mut body = Vec::new();

        loop {
            match self.peek().kind {
                Kind::CloseBlock => {
                    self.bump();
                    return Some(body);
                }
                Kind::End => {
                    // Point at the brace that was opened, not at the end of the file:
                    // the end of the file is where it was noticed, not where it went
                    // wrong, and a long file makes that difference matter.
                    self.errors.push(
                        Diagnostic::new("E0109", "a block was opened here and never closed.")
                            .primary(open, "this `{` has no partner")
                            .rule("a block begins with `{` and ends with `}`")
                            .tip("the end of the file closes nothing — it is the brace that does.")
                            .fix("add a `}` where the block should end"),
                    );
                    return Some(body);
                }
                _ => match self.statement() {
                    Some(stmt) => body.push(stmt),
                    None => self.recover_in_block(),
                },
            }
        }
    }

    /// Give up on a statement, but stop at the closing brace rather than running past it.
    fn recover_in_block(&mut self) {
        while !self.at_end() {
            if self.peek().kind == Kind::CloseBlock {
                return;
            }
            if self.bump().kind == Kind::Semicolon {
                return;
            }
        }
    }

    fn statement(&mut self) -> Option<Stmt> {
        let token = self.peek();
        if token.kind != Kind::Word {
            self.errors.push(
                Diagnostic::new("E0103", "a statement begins with a word.")
                    .primary(token.span, format!("found {}", token.kind.describe()))
                    .rule("every statement starts by saying what it is — `var`, `print`")
                    .fix("start the line with what it does"),
            );
            return None;
        }

        match self.text(token.span) {
            "print" => self.print().map(Stmt::Print),
            "var" => self.var().map(Stmt::Var),
            "set" => self.set().map(Stmt::Set),
            "if" => self.conditional().map(Stmt::If),
            other => {
                self.errors.push(
                    Diagnostic::new("E0104", format!("`{other}` is not something Quench does."))
                        .primary(token.span, "here")
                        .rule("a statement begins with `var`, `set`, `print` or `if`")
                        .tip("that is the whole list, for now.")
                        .fix("did you mean `var`, `set`, `print` or `if`?"),
                );
                None
            }
        }
    }

    fn print(&mut self) -> Option<Print> {
        let word = self.bump().span;
        // Where it goes is part of the statement, not a default somebody has to know.
        self.expect(Kind::Dot, "`print`")?;
        let to = self.expect(Kind::Word, "`print`")?;
        self.expect(Kind::OpenList, "`print`")?;

        let mut pieces = Vec::new();
        while !matches!(self.peek().kind, Kind::CloseList | Kind::End) {
            // `'xs'[*1*]` in a print list is an index, not a name and then a list.
            if self.peek().kind == Kind::Name
                && self.tokens.get(self.at + 1).map(|t| t.kind) == Some(Kind::OpenList)
            {
                let ast::Term::At { name, indices, close } = self.term()? else {
                    unreachable!("just matched an index")
                };
                pieces.push(ast::Piece::At { name, indices, close });
                continue;
            }
            pieces.push(self.piece(true)?);
        }

        self.expect(Kind::CloseList, "`print`")?;
        let end = self.expect(Kind::Semicolon, "a statement")?;
        Some(Print { word, to, pieces, span: word.to(end) })
    }

    /// `if … { } else-if … { } else { }`
    fn conditional(&mut self) -> Option<ast::If> {
        let start = self.peek().span;
        let mut arms = Vec::new();
        let mut otherwise = None;
        let mut end;

        loop {
            let word = self.bump().span; // `if` or `else-if`
            let condition = self.value()?;
            if condition.terms.is_empty() {
                self.errors.push(
                    Diagnostic::new("E0111", "this asks nothing.")
                        .primary(word, "here")
                        .rule("`if` is followed by something that is true or false, and then a block")
                        .fix("put a condition between it and the `{`"),
                );
                return None;
            }
            let body = self.block()?;
            end = body.last().map(ast::Stmt::span).unwrap_or(word);
            arms.push(ast::Arm { word, condition, body });

            // `else-if` is one word, so chaining and nesting are different syntax rather
            // than the same syntax read two ways. That is the whole of the dangling-else
            // problem, absent.
            if self.peek().kind != Kind::Word {
                break;
            }
            match self.text(self.peek().span) {
                "else-if" => continue,
                "else" => {
                    let word = self.bump().span;
                    let body = self.block()?;
                    end = body.last().map(ast::Stmt::span).unwrap_or(word);
                    otherwise = Some(body);
                    break;
                }
                _ => break,
            }
        }

        Some(ast::If { arms, otherwise, span: start.to(end) })
    }

    /// `set ['x', 'xs'[*1*]] = [*5*, *9*];`
    fn set(&mut self) -> Option<ast::Set> {
        let word = self.bump().span;
        self.expect(Kind::OpenList, "`set`")?;

        let mut targets = Vec::new();
        loop {
            let name = self.expect(Kind::Name, "`set`")?;
            if self.peek().kind == Kind::OpenList {
                self.bump();
                let mut indices = Vec::new();
                while !matches!(self.peek().kind, Kind::CloseList | Kind::End) {
                    indices.push(self.term()?);
                }
                let close = self.expect(Kind::CloseList, "an index")?;
                targets.push(ast::Place::At { name, indices, close });
            } else {
                targets.push(ast::Place::Name(name));
            }
            if self.eat(Kind::Comma).is_none() {
                break;
            }
        }
        let targets_end = self.expect(Kind::CloseList, "`set`")?;
        self.expect(Kind::Equals, "`set`")?;

        let values_start = self.expect(Kind::OpenList, "`set`")?;
        let mut values = Vec::new();
        loop {
            values.push(self.value()?);
            if self.eat(Kind::Comma).is_none() {
                break;
            }
        }
        let values_end = self.expect(Kind::CloseList, "`set`")?;
        let end = self.expect(Kind::Semicolon, "a statement")?;

        if targets.len() != values.len() {
            let named = targets[0].span().to(targets_end);
            let given = values_start.to(values_end);
            self.errors.push(
                Diagnostic::new(
                    "E0110",
                    format!(
                        "{} changed, and {} given.",
                        counted(targets.len(), "thing"),
                        counted(values.len(), "value")
                    ),
                )
                .secondary(named, counted(targets.len(), "thing"))
                .primary(given, counted(values.len(), "value"))
                .rule("`set` gives one value for each thing it changes, in the same order")
                .tip("a value runs until a comma, so a missing comma joins two of them into one.")
                .fix("give one value for each"),
            );
        }

        Some(ast::Set { word, targets, values, span: word.to(end) })
    }

    fn var(&mut self) -> Option<Var> {
        let start = self.peek().span;
        let mut chain = vec![self.bump().span];
        while self.eat(Kind::Dot).is_some() {
            chain.push(self.expect(Kind::Word, "a chain")?);
        }

        // `(5 2)` — the shape, if there is one. A shape is part of the type, so it
        // sits between the chain and the names rather than inside either.
        let mut shape = Vec::new();
        let mut shape_span = None;
        if self.peek().kind == Kind::OpenGroup {
            let open = self.bump().span;
            while self.peek().kind == Kind::Number {
                shape.push(self.bump().span);
            }
            let close = self.expect(Kind::CloseGroup, "a shape")?;
            shape_span = Some(open.to(close));
        }

        // The names.
        self.expect(Kind::OpenList, "a declaration")?;
        let mut names = Vec::new();
        loop {
            names.push(self.expect(Kind::Name, "a declaration")?);
            if self.eat(Kind::Comma).is_none() {
                break;
            }
        }
        let names_end = self.expect(Kind::CloseList, "a declaration")?;
        // The names themselves, not the brackets around them: the count is what is
        // wrong, and the brackets are innocent.
        let names_span = match (names.first(), names.last()) {
            (Some(first), Some(last)) => first.to(*last),
            _ => names_end,
        };

        self.expect(Kind::Equals, "a declaration")?;

        // The values. Each one is a flat run of terms with whatever was written
        // between them; what binds to what is decided later, by something that can
        // explain itself when the answer is not settled.
        let values_start = self.expect(Kind::OpenList, "a declaration")?;
        let mut values = Vec::new();
        loop {
            values.push(self.value()?);
            if self.eat(Kind::Comma).is_none() {
                break;
            }
        }
        let values_end = self.expect(Kind::CloseList, "a declaration")?;
        let end = self.expect(Kind::Semicolon, "a statement")?;
        let values_span = match (values.first(), values.last()) {
            (Some(first), Some(last)) => first.span.to(last.span),
            _ => values_start.to(values_end),
        };

        if names.len() != values.len() {
            self.errors.push(
                Diagnostic::new(
                    "E0105",
                    format!(
                        "{} declared, and {} given.",
                        counted(names.len(), "name"),
                        counted(values.len(), "value")
                    ),
                )
                .secondary(names_span, counted(names.len(), "name"))
                .primary(values_span, counted(values.len(), "value"))
                .rule("a declaration gives one value for each name, in the same order")
                .tip("a value runs until a comma, so a missing comma joins two of them into one.")
                .fix(if names.len() > values.len() {
                    "add the missing value, or remove the name"
                } else {
                    "add the missing name, or remove the value"
                }),
            );
        }

        Some(Var { chain, shape, shape_span, names, values, span: start.to(end) })
    }

    /// One value: terms, and whatever sits between them.
    fn value(&mut self) -> Option<ast::Value> {
        let from = self.peek().span;
        let mut terms = Vec::new();
        let mut between = Vec::new();

        while !matches!(
            self.peek().kind,
            Kind::Comma | Kind::CloseList | Kind::CloseGroup | Kind::OpenBlock | Kind::End
        ) {
            if !terms.is_empty() {
                // Either an operator, or nothing at all — and nothing at all is
                // juxtaposition, which is how a list of pieces builds text.
                between.push(self.operator());
            }
            terms.push(self.term()?);
        }

        let to = terms.last().map(ast::Term::span).unwrap_or(from);
        Some(ast::Value { terms, between, span: from.to(to) })
    }

    /// An operator, if one is written here. Words are operators too, and cost nothing,
    /// because Quench reserves none of them.
    fn operator(&mut self) -> Option<ast::Operator> {
        use ast::OpKind::*;
        let token = self.peek();
        let kind = match token.kind {
            Kind::Plus => Add,
            Kind::Minus => Sub,
            Kind::Times => Mul,
            Kind::Slash => Div,
            Kind::Power => Pow,
            Kind::Less => Lt,
            Kind::Greater => Gt,
            Kind::LessEqual => Le,
            Kind::GreaterEqual => Ge,
            Kind::EqualTo => Eq,
            Kind::NotEqual => Ne,
            // A word is an operator only when it is not a type in front of a value:
            // `str:*x*` starts a term, `x` between two of them multiplies.
            Kind::Word if self.tokens.get(self.at + 1).map(|t| t.kind) != Some(Kind::Colon) => {
                match self.text(token.span) {
                    "x" => Mul,
                    "xx" => Pow,
                    "mod" => Mod,
                    "and" => And,
                    "or" => Or,
                    "eq-to" => Eq,
                    _ => return None,
                }
            }
            _ => return None,
        };
        self.bump();
        Some(ast::Operator { kind, span: token.span })
    }

    /// One operand: a piece, a bracketed value, an array, an index, or `not`.
    fn term(&mut self) -> Option<ast::Term> {
        let token = self.peek();
        if token.kind == Kind::Number {
            return Some(ast::Term::Number(self.bump().span));
        }
        // `[…]` here is a list of elements. `[` always opens a list in Quench -- of
        // names, of values, of things to print -- and this is one more of them.
        if token.kind == Kind::OpenList {
            let open = self.bump().span;
            let mut of = Vec::new();
            while !matches!(self.peek().kind, Kind::CloseList | Kind::End) {
                of.push(self.term()?);
            }
            let close = self.expect(Kind::CloseList, "an array")?;
            return Some(ast::Term::Elements { open, of, close });
        }
        // A quoted name followed by a bracket is an index. A bare word followed by one
        // would be a call, which is why names being quoted settles this for free.
        if token.kind == Kind::Name && self.tokens.get(self.at + 1).map(|t| t.kind) == Some(Kind::OpenList)
        {
            let name = self.bump().span;
            self.bump();
            let mut indices = Vec::new();
            while !matches!(self.peek().kind, Kind::CloseList | Kind::End) {
                indices.push(self.term()?);
            }
            let close = self.expect(Kind::CloseList, "an index")?;
            return Some(ast::Term::At { name, indices, close });
        }
        if token.kind == Kind::OpenGroup {
            let open = self.bump().span;
            let value = self.value()?;
            let close = self.expect(Kind::CloseGroup, "a bracketed value")?;
            return Some(ast::Term::Group { open, value: Box::new(value), close });
        }
        if token.kind == Kind::Word && self.text(token.span) == "not" {
            let word = self.bump().span;
            let of = self.term()?;
            return Some(ast::Term::Not { word, of: Box::new(of) });
        }
        self.piece(false).map(ast::Term::Piece)
    }

    /// One piece of a list. `typed` says whether a written value may carry a type: it
    /// must in a `print`, where nothing else supplies one, and must not in a declaration,
    /// where the chain already did.
    fn piece(&mut self, typed: bool) -> Option<Piece> {
        let token = self.peek();
        match token.kind {
            Kind::Name => Some(Piece::Name(self.bump().span)),
            Kind::Escape => Some(Piece::Escape(self.bump().span)),
            Kind::Written => {
                let mark = self.bump().span;
                if typed {
                    self.errors.push(
                        Diagnostic::new("E0106", "this written value does not say what it is.")
                            .primary(mark, "no type in front of it")
                            .rule("a written value means nothing until a type reads it: `*1000*` is a number under `b16` and four characters under `str`")
                            .tip("a declaration says the type in its chain, so only here does the value have to.")
                            .fix(format!("`str:{}` if it is text", self.text(mark))),
                    );
                }
                Some(Piece::Written { ty: None, mark })
            }
            Kind::Word => {
                let ty = self.bump().span;
                self.expect(Kind::Colon, "a typed value")?;
                let mark = self.expect(Kind::Written, "a typed value")?;
                if !typed {
                    self.errors.push(
                        Diagnostic::new("E0107", "this value says its type twice.")
                            .primary(ty, "said here")
                            .rule("a declaration's chain already says the type, so its values do not repeat it")
                            .fix(format!("`{}`", self.text(mark))),
                    );
                }
                Some(Piece::Written { ty: Some(ty), mark })
            }
            _ => {
                self.errors.push(
                    Diagnostic::new("E0108", "this cannot be part of a list.")
                        .primary(token.span, format!("found {}", token.kind.describe()))
                        .rule("a list holds written values, names and escapes")
                        .fix("remove it"),
                );
                None
            }
        }
    }
}

/// `one name`, `two names`, `13 names`.
///
/// Small counts are spelled out because the message is a sentence and a reader is
/// reading it, not scanning it. Past twelve, digits are what a person would write.
fn counted(n: usize, what: &str) -> String {
    const WORDS: [&str; 13] = [
        "no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
        "ten", "eleven", "twelve",
    ];
    let count = WORDS.get(n).map_or_else(|| n.to_string(), |word| word.to_string());
    if n == 1 { format!("{count} {what}") } else { format!("{count} {what}s") }
}

/// The name a program starts at. Kept in one place; see `quench_qir::ENTRY`.
fn quench_qir_entry() -> &'static str {
    "START"
}
