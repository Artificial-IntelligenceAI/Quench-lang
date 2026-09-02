//! Checking that a QIR module says what it claims to.
//!
//! Three execution methods lower this IR. If it is malformed, they will be malformed in
//! three different ways, and the oracle will report a disagreement that is really one bug
//! wearing three costumes. So the IR is checked once, here, before any backend sees it,
//! and a backend is then entitled to assume what it reads is well formed.
//!
//! This is a check on the *frontend*, not on a user's program. Anything it catches is a
//! compiler bug, which is why it produces a plain description rather than a diagnostic
//! with a rule and a suggested fix — there is no fix a user could apply.
//!
//! What is not checked yet: **dominance**. A value used before the block that defines it
//! can reach is not caught here, and will be caught by Cranelift's own verifier with a
//! worse message. Doing it properly needs a dominator tree, which is worth writing once
//! there are enough passes to justify one.

use crate::{Inst, Module, Term, Ty, Value};
use std::collections::HashSet;
use std::fmt;

/// Something wrong with a module, in the terms the compiler's own authors would use.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Invalid {
    /// The function it is in, by name.
    pub function: String,
    pub what: String,
}

impl fmt::Display for Invalid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "in `{}`: {}", self.function, self.what)
    }
}

/// Check every function in a module. Returns everything wrong, not just the first thing.
pub fn verify(module: &Module) -> Result<(), Vec<Invalid>> {
    let mut wrong = Vec::new();

    if let Some(entry) = module.entry
        && entry.0 as usize >= module.functions.len()
    {
        wrong.push(Invalid {
            function: "<module>".into(),
            what: format!("the entry is function {}, and there are {} of them", entry.0, module.functions.len()),
        });
    }

    for func in &module.functions {
        let mut say = |what: String| wrong.push(Invalid { function: func.name.clone(), what });

        if func.blocks.is_empty() {
            say("a function has no blocks, so there is nowhere for a call to start".into());
            continue;
        }

        // The entry block is called with the function's arguments, so it must want them.
        let entry_params: Vec<Ty> =
            func.blocks[0].params.iter().map(|v| func.ty_of(*v)).collect();
        if entry_params != func.params {
            say(format!(
                "the entry block takes ({}), but the function is declared ({})",
                list(&entry_params),
                list(&func.params),
            ));
        }

        // Every value is defined exactly once: as a block parameter, or by an instruction.
        let mut defined: HashSet<Value> = HashSet::new();
        for (i, block) in func.blocks.iter().enumerate() {
            for v in &block.params {
                if !defined.insert(*v) {
                    say(format!("{v:?} is bound twice, the second time by block {i}"));
                }
            }
            for (v, _) in &block.insts {
                if !defined.insert(*v) {
                    say(format!("{v:?} is assigned twice, the second time in block {i}"));
                }
            }
        }

        let known = |v: Value, say: &mut dyn FnMut(String)| {
            if v.0 as usize >= func.value_tys.len() {
                say(format!("{v:?} is used and there are only {} values", func.value_tys.len()));
                false
            } else if !defined.contains(&v) {
                say(format!("{v:?} is used and never defined"));
                false
            } else {
                true
            }
        };

        for (i, block) in func.blocks.iter().enumerate() {
            for (result, inst) in &block.insts {
                match inst {
                    Inst::ConstI64(_) | Inst::ConstBool(_) => {}
                    Inst::Bin { op, lhs, rhs } => {
                        for v in [lhs, rhs] {
                            if known(*v, &mut say) && func.ty_of(*v) != Ty::I64 {
                                say(format!(
                                    "block {i}: {op:?} wants i64 and {v:?} is {}",
                                    func.ty_of(*v).name()
                                ));
                            }
                        }
                    }
                    Inst::Cmp { op, lhs, rhs } => {
                        for v in [lhs, rhs] {
                            if known(*v, &mut say) && func.ty_of(*v) != Ty::I64 {
                                say(format!(
                                    "block {i}: {op:?} wants i64 and {v:?} is {}",
                                    func.ty_of(*v).name()
                                ));
                            }
                        }
                    }
                    Inst::Not(v) => {
                        if known(*v, &mut say) && func.ty_of(*v) != Ty::Bool {
                            say(format!(
                                "block {i}: `not` wants bool and {v:?} is {}",
                                func.ty_of(*v).name()
                            ));
                        }
                    }
                    Inst::Call { func: callee, args } => {
                        let Some(target) = module.functions.get(callee.0 as usize) else {
                            say(format!("block {i}: calls function {}, which does not exist", callee.0));
                            continue;
                        };
                        if args.len() != target.params.len() {
                            say(format!(
                                "block {i}: `{}` takes {} argument(s) and is given {}",
                                target.name,
                                target.params.len(),
                                args.len()
                            ));
                        }
                        for (n, (arg, want)) in args.iter().zip(&target.params).enumerate() {
                            if known(*arg, &mut say) && func.ty_of(*arg) != *want {
                                say(format!(
                                    "block {i}: `{}` wants {} for argument {n} and is given {}",
                                    target.name,
                                    want.name(),
                                    func.ty_of(*arg).name()
                                ));
                            }
                        }
                        if func.ty_of(*result) != target.ret {
                            say(format!(
                                "block {i}: the call to `{}` is recorded as {} and it returns {}",
                                target.name,
                                func.ty_of(*result).name(),
                                target.ret.name()
                            ));
                        }
                    }
                }
            }

            match &block.term {
                Term::Ret(v) => {
                    if known(*v, &mut say) && func.ty_of(*v) != func.ret {
                        say(format!(
                            "block {i}: returns {} from a function declared to return {}",
                            func.ty_of(*v).name(),
                            func.ret.name()
                        ));
                    }
                }
                Term::Jump { to, args } => check_target(func, i, *to, args, &mut say),
                Term::BrIf { cond, then, otherwise } => {
                    if known(*cond, &mut say) && func.ty_of(*cond) != Ty::Bool {
                        say(format!(
                            "block {i}: branches on {:?}, which is {}, not bool",
                            cond,
                            func.ty_of(*cond).name()
                        ));
                    }
                    check_target(func, i, then.block, &then.args, &mut say);
                    check_target(func, i, otherwise.block, &otherwise.args, &mut say);
                }
            }
        }
    }

    if wrong.is_empty() { Ok(()) } else { Err(wrong) }
}

/// A jump has to hand the block it lands on exactly the values that block asks for.
fn check_target(
    func: &crate::Function,
    from: usize,
    to: crate::BlockId,
    args: &[Value],
    say: &mut dyn FnMut(String),
) {
    let Some(block) = func.blocks.get(to.0 as usize) else {
        say(format!("block {from}: jumps to block {}, which does not exist", to.0));
        return;
    };
    if args.len() != block.params.len() {
        say(format!(
            "block {from}: jumps to block {} with {} argument(s), and it takes {}",
            to.0,
            args.len(),
            block.params.len()
        ));
        return;
    }
    for (n, (arg, param)) in args.iter().zip(&block.params).enumerate() {
        let (given, wanted) = (func.ty_of(*arg), func.ty_of(*param));
        if given != wanted {
            say(format!(
                "block {from}: jumps to block {} passing {} for parameter {n}, which is {}",
                to.0,
                given.name(),
                wanted.name()
            ));
        }
    }
}

fn list(tys: &[Ty]) -> String {
    tys.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ")
}
