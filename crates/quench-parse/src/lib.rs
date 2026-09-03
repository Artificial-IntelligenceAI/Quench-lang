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

pub use ast::{
    Arm, Call, Func, Give, If, Item, Loop, LoopKind, OpKind, Operator, Param, Piece,
    Place, Print, Program, Set, Start, Stmt, Term, Value, Var,
};

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
        Parser {
            source,
            tokens: lexed.tokens,
            at: 0,
            errors: lexed.errors,
            typed_in_a_value: Vec::new(),
        };
    let program = parser.program();
    Parsed { program, errors: parser.errors }
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    at: usize,
    errors: Vec<Diagnostic>,
    /// Types written on a value where a chain was going to supply one. Whether that is
    /// a mistake depends on the whole value, so it is decided once the value ends.
    typed_in_a_value: Vec<(Span, Span)>,
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

            if token.kind == Kind::Word {
                match self.text(token.span) {
                    "fn" => {
                        match self.function() {
                            Some(func) => program.items.push(ast::Item::Func(func)),
                            None => self.recover(),
                        }
                        continue;
                    }
                    // A constant is a declaration written somewhere else, so it is
                    // parsed by the same code and gets the same errors.
                    "const" => {
                        match self.var() {
                            Some(item) => program.items.push(ast::Item::Const(item)),
                            None => self.recover(),
                        }
                        continue;
                    }
                    _ => {}
                }
            }

            // `var` at the top of a file is the one worth naming, because it is the
            // mistake somebody makes on purpose: the rule is that constants live
            // outside and variables live inside, and this is where they meet.
            let diag = if token.kind == Kind::Word && self.text(token.span) == "var" {
                Diagnostic::new("E0102", "a variable cannot be at the top of a file.")
                    .primary(token.span, "here")
                    .rule("constants live outside a function and variables live inside one")
                    .tip("a constant is a value the compiler can work out. Anything needing code to run to produce it would need that code to run before `START`.")
                    .fix("`const.<visibility>.<type>` here, or move it inside `START`")
            } else {
                Diagnostic::new("E0102", "this cannot be at the top of a file.")
                    .primary(token.span, "here")
                    .rule("a file holds constants, functions and one `START`")
                    .tip("statements run, and the only place anything runs is inside a function.")
                    .fix("did you mean `const`, `fn` or `START`?")
            };
            self.errors.push(diag);
            self.recover();
        }

        program
    }

    /// `fn.export.i64 ['add'] [immut.i64 'a', immut.i64 'b'] { … }`
    fn function(&mut self) -> Option<ast::Func> {
        let word = self.bump().span;
        let mut chain = vec![word];
        while self.eat(Kind::Dot).is_some() {
            chain.push(self.expect(Kind::Word, "a chain")?);
        }

        let (shape, shape_span) = self.shape()?;

        self.expect(Kind::OpenList, "a function")?;
        let name = self.expect(Kind::Name, "a function")?;
        self.expect(Kind::CloseList, "a function")?;

        // Written even when empty, because `[]` says *takes nothing* out loud and an
        // omission would only say it by not being there.
        let open = self.expect(Kind::OpenList, "a function")?;
        let mut params = Vec::new();
        while !matches!(self.peek().kind, Kind::CloseList | Kind::End) {
            params.push(self.parameter()?);
            if self.eat(Kind::Comma).is_none() {
                break;
            }
        }
        let close = self.expect(Kind::CloseList, "a parameter list")?;

        let body = self.block()?;
        let end = body.last().map(ast::Stmt::span).unwrap_or(close);
        Some(ast::Func {
            chain,
            shape,
            shape_span,
            name,
            takes: open.to(close),
            params,
            body,
            span: word.to(end),
        })
    }

    /// `immut.i64 'a'` — a declaration's chain with `var` taken off.
    fn parameter(&mut self) -> Option<ast::Param> {
        let first = self.expect(Kind::Word, "a parameter")?;
        let mut chain = vec![first];
        while self.eat(Kind::Dot).is_some() {
            chain.push(self.expect(Kind::Word, "a chain")?);
        }
        let (shape, shape_span) = self.shape()?;
        let name = self.expect(Kind::Name, "a parameter")?;
        Some(ast::Param { chain, shape, shape_span, name, span: first.to(name) })
    }

    /// `(5 2)` — a shape, where one is written. Part of the type, so it sits between
    /// the chain and whatever the chain was describing, wherever that happens.
    fn shape(&mut self) -> Option<(Vec<Span>, Option<Span>)> {
        if self.peek().kind != Kind::OpenGroup {
            return Some((Vec::new(), None));
        }
        let open = self.bump().span;
        let mut shape = Vec::new();
        // Numbers, and the one word that stands where a number would: `grow` says
        // there is no number yet, which is a thing a size is allowed to say.
        while matches!(self.peek().kind, Kind::Number | Kind::Word) {
            shape.push(self.bump().span);
        }
        let close = self.expect(Kind::CloseGroup, "a shape")?;
        Some((shape, Some(open.to(close))))
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
            "loop" => self.repeat().map(Stmt::Loop),
            // `add ['xs'] = [*7*];` — the same shape as `set`, because it is the same
            // sentence with a different verb: this one makes the array longer.
            "add" => {
                let word = self.bump().span;
                self.expect(Kind::OpenList, "`add`")?;
                let mut targets = Vec::new();
                loop {
                    targets.push(self.place("`add`")?);
                    if self.eat(Kind::Comma).is_none() {
                        break;
                    }
                }
                self.expect(Kind::CloseList, "`add`")?;
                self.expect(Kind::Equals, "`add`")?;
                self.expect(Kind::OpenList, "`add`")?;
                let mut values = Vec::new();
                loop {
                    values.push(self.value()?);
                    if self.eat(Kind::Comma).is_none() {
                        break;
                    }
                }
                self.expect(Kind::CloseList, "`add`")?;
                let end = self.expect(Kind::Semicolon, "a statement")?;
                Some(Stmt::Add(Set { word, targets, values, span: word.to(end) }))
            }
            "give" => {
                let word = self.bump().span;
                // `give;` from a function that gives nothing back. The word is still
                // written, because leaving early is a thing you do on purpose.
                if let Some(end) = self.eat(Kind::Semicolon) {
                    return Some(Stmt::Give(ast::Give { word, value: None, span: word.to(end) }));
                }
                self.expect(Kind::OpenList, "`give`")?;
                let value = self.value()?;
                self.expect(Kind::CloseList, "`give`")?;
                let end = self.expect(Kind::Semicolon, "a statement")?;
                Some(Stmt::Give(ast::Give { word, value: Some(value), span: word.to(end) }))
            }
            word if self.tokens.get(self.at + 1).map(|t| t.kind) == Some(Kind::OpenList)
                && !matches!(word, "var" | "set" | "print" | "if" | "loop") =>
            {
                let call = self.invocation()?;
                self.expect(Kind::Semicolon, "a statement")?;
                Some(Stmt::Do(call))
            }
            "break" => {
                let word = self.bump().span;
                let end = self.expect(Kind::Semicolon, "a statement")?;
                Some(Stmt::Break(word.to(end)))
            }
            other => {
                self.errors.push(
                    Diagnostic::new("E0104", format!("`{other}` is not something Quench does."))
                        .primary(token.span, "here")
                        .rule("a statement begins with `var`, `set`, `add`, `print`, `if`, `loop` or `break`")
                        .tip("that is the whole list, for now.")
                        .fix("did you mean `var`, `set`, `add`, `print`, `if`, `loop` or `break`?"),
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
            // And a bare word before one is a call, for the same reason it is elsewhere.
            if self.peek().kind == Kind::Word
                && self.tokens.get(self.at + 1).map(|t| t.kind) == Some(Kind::OpenList)
            {
                let ast::Term::Call(call) = self.term()? else {
                    unreachable!("just matched a call")
                };
                pieces.push(ast::Piece::Call(call));
                continue;
            }
            pieces.push(self.piece(true)?);
        }

        self.expect(Kind::CloseList, "`print`")?;
        let end = self.expect(Kind::Semicolon, "a statement")?;
        Some(Print { word, to, pieces, span: word.to(end) })
    }

    /// `add[*1*, *2*]` — arguments are values, so commas separate them. Juxtaposition
    /// builds one value out of pieces, which is why it cannot also separate two.
    fn invocation(&mut self) -> Option<ast::Call> {
        let name = self.bump().span;
        self.bump();
        let mut args = Vec::new();
        while !matches!(self.peek().kind, Kind::CloseList | Kind::End) {
            args.push(self.value()?);
            if self.eat(Kind::Comma).is_none() {
                break;
            }
        }
        let close = self.expect(Kind::CloseList, "a call")?;
        Some(ast::Call { name, args, close })
    }

    /// `loop.temp.range.i64 ['i'] = [*1*, *5*] { … }` or `loop.while … { … }`.
    fn repeat(&mut self) -> Option<ast::Loop> {
        let word = self.bump().span;
        let mut chain = Vec::new();
        while self.eat(Kind::Dot).is_some() {
            chain.push(self.expect(Kind::Word, "a chain")?);
        }

        // Which kind it is decides what follows, and the chain has already said.
        let counted = chain.iter().any(|link| self.text(*link) == "range");
        let kind = if counted {
            self.expect(Kind::OpenList, "a loop")?;
            let name = self.expect(Kind::Name, "a loop")?;
            self.expect(Kind::CloseList, "a loop")?;
            self.expect(Kind::Equals, "a loop")?;
            self.expect(Kind::OpenList, "a loop")?;
            let from = self.value()?;
            self.expect(Kind::Comma, "a range")?;
            let to = self.value()?;
            self.expect(Kind::CloseList, "a loop")?;
            ast::LoopKind::Range { name, from, to }
        } else {
            let condition = self.value()?;
            if condition.terms.is_empty() {
                self.errors.push(
                    Diagnostic::new("E0112", "this loop asks nothing and counts nothing.")
                        .primary(word, "here")
                        .rule("a loop is `range` with bounds, or `while` with a condition")
                        .fix("`loop.temp.range.<type> ['i'] = [*1*, *5*]`, or `loop.while <condition>`"),
                );
                return None;
            }
            ast::LoopKind::While(condition)
        };

        let body = self.block()?;
        let end = body.last().map(ast::Stmt::span).unwrap_or(word);
        Some(ast::Loop { word, chain, kind, body, span: word.to(end) })
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
    /// One thing a value can be put into: a name, or one element of an array.
    fn place(&mut self, what: &'static str) -> Option<ast::Place> {
        let name = self.expect(Kind::Name, what)?;
        if self.peek().kind != Kind::OpenList {
            return Some(ast::Place::Name(name));
        }
        self.bump();
        let mut indices = Vec::new();
        while !matches!(self.peek().kind, Kind::CloseList | Kind::End) {
            indices.push(self.term()?);
        }
        let close = self.expect(Kind::CloseList, "an index")?;
        Some(ast::Place::At { name, indices, close })
    }

    fn set(&mut self) -> Option<ast::Set> {
        let word = self.bump().span;
        self.expect(Kind::OpenList, "`set`")?;

        let mut targets = Vec::new();
        loop {
            targets.push(self.place("`set`")?);
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
        let (shape, shape_span) = self.shape()?;

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
            values.push(self.value_of_a_declaration()?);
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
    /// One value, and afterwards the question the pieces could not answer on their own:
    /// whether a type written on one of them was already said by the chain.
    ///
    /// It was, unless operators were written — `[e:*0.1* == e:*0.3*]` under a `bool`
    /// chain is a comparison of two numbers the chain said nothing about, and the only
    /// place that can say is the value.
    fn value_of_a_declaration(&mut self) -> Option<ast::Value> {
        let before = self.typed_in_a_value.len();
        let value = self.value();
        let said: Vec<(Span, Span)> = self.typed_in_a_value.drain(before..).collect();
        if let Some(value) = &value {
            if !value.has_operators() {
                for (ty, mark) in said {
                    self.errors.push(
                        Diagnostic::new("E0107", "this value says its type twice.")
                            .primary(ty, "said here")
                            .rule("a declaration's chain already says the type, so its values do not repeat it")
                            .fix(format!("`{}`", self.text(mark))),
                    );
                }
            }
        }
        value
    }

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
        // A bare word followed by a bracket is a call. A quoted name followed by one is
        // an index. Names being quoted is what settles this, and always was.
        if token.kind == Kind::Word
            && self.tokens.get(self.at + 1).map(|t| t.kind) == Some(Kind::OpenList)
        {
            return self.invocation().map(ast::Term::Call);
        }
        // A quoted name followed by a bracket is an index.
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
        if token.kind == Kind::Word && matches!(self.text(token.span), "share" | "copy") {
            let copies = self.text(token.span) == "copy";
            let word = self.bump().span;
            let of = self.term()?;
            return Some(ast::Term::Handed { word, copies, of: Box::new(of) });
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
                    self.typed_in_a_value.push((ty, mark));
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
pub fn counted(n: usize, what: &str) -> String {
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
