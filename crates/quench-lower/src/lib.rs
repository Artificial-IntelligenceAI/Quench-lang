//! The meaning of a program, turned into the IR every engine runs.
//!
//! There are no decisions here. [`quench_check`] resolved every name, settled every
//! type, joined every piece of text and refused everything that did not make sense — so
//! what is left is a transliteration, and that is the point of doing the checking first.
//! Anything in this file that started to look like a judgement would belong further up.

use quench_check::{Checked, Printed, Stmt, Ty, Value};
use quench_diag::{Diagnostic, Span};
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
    let mut checked = quench_check::check(source);

    if !checked.has_start {
        // Not something the parser could report: a file of declarations is a fine thing
        // to parse and a useless thing to run, and only something trying to run it knows
        // which was wanted.
        checked.errors.push(
            Diagnostic::new("E0301", "this file has no `START`, so there is nothing to run.")
                .primary(Span::at(source.len()), "the file ends here")
                .rule("a program begins at `START`, and a file without one is not a program")
                .tip("a file may hold declarations and no `START`. It just cannot be run.")
                .fix("add `START { … }`"),
        );
        return Lowered { module: None, errors: checked.errors };
    }
    if !checked.errors.is_empty() {
        // Lowering a program that did not check out would build nonsense out of it.
        return Lowered { module: None, errors: checked.errors };
    }

    Lowered { module: Some(build(&checked)), errors: checked.errors }
}

fn build(checked: &Checked) -> qir::Module {
    let mut module = qir::Module::new();
    let mut b = qir::Builder::new(qir::ENTRY, &[], qir::Ty::I64);

    // Where each variable's value ended up. A declaration fills one in; a use reads it.
    let mut held: Vec<Option<qir::Value>> = vec![None; checked.locals.len()];

    for stmt in &checked.body {
        match stmt {
            Stmt::Declare { local, value } => {
                let value = match value {
                    Value::Text(text) => {
                        let at = module.intern(text);
                        b.const_text(at)
                    }
                    Value::Number(n) => b.const_i64(*n),
                    // Values do not change, so copying one is naming the same value
                    // again rather than doing anything.
                    Value::Copy(from) => held[from.0 as usize].expect("declared before used"),
                };
                held[local.0 as usize] = Some(value);
            }
            Stmt::Print(pieces) => {
                for piece in pieces {
                    match piece {
                        Printed::Text(text) => {
                            let at = module.intern(text);
                            let value = b.const_text(at);
                            b.call_host(qir::Host::PrintText, &[value]);
                        }
                        Printed::Local { local, ty } => {
                            let value = held[local.0 as usize].expect("declared before used");
                            let host = match ty {
                                Ty::Str => qir::Host::PrintText,
                                Ty::I64 => qir::Host::PrintI64,
                            };
                            b.call_host(host, &[value]);
                        }
                    }
                }
            }
        }
    }

    // A program that says nothing about how it ended, ended fine.
    let nothing = b.const_i64(0);
    b.ret(nothing);

    let id = module.add(b.finish());
    module.set_entry(id);
    module
}
