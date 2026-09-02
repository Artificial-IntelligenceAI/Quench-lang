//! The tree a person wrote, turned into the IR every engine runs.
//!
//! This is the join. Above it, everything is about what somebody typed — spans, marks,
//! the chain, recovery at the semicolon. Below it, everything is about what a machine
//! does — blocks, values, instructions. Nothing on one side needs to know the other
//! exists, which is why the join is a crate of its own rather than a method somewhere.
//!
//! It is also the last place a diagnostic can point at source. Everything after this
//! sees QIR, which carries spans but not the text they came from, so a mistake noticed
//! here is a mistake reported properly and a mistake noticed later is an apology.

use quench_diag::{Diagnostic, Span};
use quench_parse::{ast, Parsed};
use quench_qir as qir;

/// What a file became, and everything wrong with it.
pub struct Lowered {
    /// Absent when the file could not be turned into a program at all.
    pub module: Option<qir::Module>,
    pub errors: Vec<Diagnostic>,
}

impl Lowered {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Read a whole file and turn it into something that can run.
pub fn lower(source: &str) -> Lowered {
    let Parsed { program, errors } = quench_parse::parse(source);
    let mut lowerer = Lowerer { source, module: qir::Module::new(), errors };

    let Some(start) = program.start else {
        // Not an error the parser could report: a file of declarations is a fine thing
        // to parse and a useless thing to run, and only something trying to run it
        // knows which was wanted.
        lowerer.errors.push(
            Diagnostic::new("E0301", "this file has no `START`, so there is nothing to run.")
                .primary(Span::at(source.len()), "the file ends here")
                .rule("a program begins at `START`, and a file without one is not a program")
                .tip("a file may hold declarations and no `START`. It just cannot be run.")
                .fix("add `START { … }`"),
        );
        return Lowered { module: None, errors: lowerer.errors };
    };

    lowerer.start(&start);
    let module = lowerer.module;
    Lowered { module: Some(module), errors: lowerer.errors }
}

struct Lowerer<'a> {
    source: &'a str,
    module: qir::Module,
    errors: Vec<Diagnostic>,
}

impl<'a> Lowerer<'a> {
    fn text(&self, span: Span) -> &'a str {
        &self.source[span.start..span.end]
    }

    fn start(&mut self, start: &ast::Start) {
        let mut b = qir::Builder::new(qir::ENTRY, &[], qir::Ty::I64);

        for stmt in &start.body {
            match stmt {
                ast::Stmt::Print(print) => self.print(&mut b, print),
                ast::Stmt::Var(var) => self.errors.push(
                    Diagnostic::new("E0302", "declaring things is not built yet.")
                        .primary(var.span, "here")
                        .rule("the parts of Quench arrive one at a time, and this one has not")
                        .tip("`print` works. A declaration is read and checked, and then nothing happens with it.")
                        .fix("print something instead, for now"),
                ),
            }
        }

        // A program that says nothing about how it ended ended fine.
        let nothing = b.const_i64(0);
        b.ret(nothing);

        let id = self.module.add(b.finish());
        self.module.set_entry(id);
    }

    /// `print[…]` — each piece written in turn.
    ///
    /// Nothing is joined together first. The pieces are printed in the order they were
    /// written, which is what the list already meant, and means a printed line never
    /// has to be built in memory before any of it is seen.
    fn print(&mut self, b: &mut qir::Builder, print: &ast::Print) {
        for piece in &print.pieces {
            let Some(text) = self.piece(piece) else { continue };
            let at = self.module.intern(&text);
            let value = b.const_text(at);
            b.call_host(qir::Host::PrintText, &[value]);
        }
    }

    /// What one piece of a list says, as text.
    fn piece(&mut self, piece: &ast::Piece) -> Option<String> {
        match piece {
            ast::Piece::Written { ty, mark } => {
                let ty = (*ty)?;
                let named = self.text(ty);
                if named != "str" {
                    self.errors.push(
                        Diagnostic::new("E0303", format!("`{named}` is not built yet."))
                            .primary(ty, "here")
                            .rule("the types arrive one at a time, and `str` is the one that has")
                            .fix("`str` for now"),
                    );
                    return None;
                }
                Some(unmarked(self.text(*mark)))
            }
            ast::Piece::Escape(span) => match self.text(*span) {
                "\\n" => Some("\n".to_string()),
                "\\t" => Some("\t".to_string()),
                "\\r" => Some("\r".to_string()),
                "\\\\" => Some("\\".to_string()),
                other => {
                    self.errors.push(
                        Diagnostic::new("E0304", format!("`{other}` is not an escape."))
                            .primary(*span, "here")
                            .rule("the escapes are `\\n`, `\\t`, `\\r` and `\\\\`"),
                    );
                    None
                }
            },
            ast::Piece::Name(span) => {
                self.errors.push(
                    Diagnostic::new("E0305", "printing a name is not built yet.")
                        .primary(*span, "here")
                        .rule("a name means a variable, and declaring one is not built yet either")
                        .tip("written values print. Names do not, yet.")
                        .fix("write the value out instead, for now"),
                );
                None
            }
        }
    }
}

/// What is between the marks, with `\*` and `\\` given their meanings.
///
/// Only those two. Everything else between the marks is the character it looks like —
/// that is what makes a written value literal, and why `\n` has to stand outside one.
fn unmarked(written: &str) -> String {
    let inside = &written[1..written.len().saturating_sub(1).max(1)];
    let mut out = String::with_capacity(inside.len());
    let mut chars = inside.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some(next @ ('*' | '\\')) => out.push(next),
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
