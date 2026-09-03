//! Checking that a QIR module says what it claims to.
//!
//! Three execution methods lower this IR. If it is malformed, they will be malformed in
//! three different ways, and the oracle will report a disagreement that is really one bug
//! wearing three costumes. So the IR is checked once, here, before any backend sees it,
//! and a backend is then entitled to assume what it reads is well formed.
//!
//! **The same check has two audiences.** While a module has only ever been in this
//! process, nothing but a bug in Quench could have malformed it, and saying so plainly is
//! right — there is no fix a user could apply. But QIR travels: it is the artefact
//! Quench compiles to, so a module can also arrive from a file, from another machine,
//! from a download that stopped early. Then a failure is not a compiler bug, it is a
//! damaged or foreign file, and telling its reader that Quench has an internal error
//! would be a lie.
//!
//! So the findings are data, and [`Audience`] decides who is being told. Both come out
//! as ordinary [`Diagnostic`]s, in the format every other Quench error uses — a reader
//! should not have to learn a second one because the trouble is in a file rather than in
//! a line.
//!
//! What is not checked yet: **dominance**. A value used before the block that defines it
//! can reach is not caught here, and will be caught by Cranelift's own verifier with a
//! worse message. Doing it properly needs a dominator tree, which is worth writing once
//! there are enough passes to justify one.

use crate::{BinOp, Inst, Module, Term, Ty, Value};
use quench_diag::Diagnostic;
use std::collections::HashSet;
use std::fmt;

/// Who is being told that a module does not check out.
///
/// The failure is identical; the truthful thing to say about it is not.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Audience {
    /// This compiler built the module and is checking its own work. Nothing else could
    /// have produced it, so this is a bug in Quench and the reader can only report it.
    Ourselves,
    /// The module arrived — a file, another machine, a transfer that stopped early. Not
    /// Quench's bug, and not necessarily anyone's, and there are things the reader can
    /// actually do about it.
    AFileWeWereGiven,
}

/// Turn findings into an error in Quench's own format.
///
/// `origin` names the thing at fault: the path a module was read from, or what the
/// compiler was about to do with one it built.
pub fn diagnose(wrong: &[Invalid], audience: Audience, origin: &str) -> Diagnostic {
    // E9xxx is the range for "this is not your program's fault".
    let mut diag = match audience {
        Audience::Ourselves => Diagnostic::new(
            "E9001",
            format!("Quench built a module for {origin} that is not well formed. This is a bug in Quench, not in your program."),
        )
        .rule("the compiler checks its own output before any backend sees it, so that one bug cannot become three different wrong answers")
        .tip("your program may well be fine. Nothing you wrote can cause this.")
        .fix("please report it, with the program that caused it if you can share it"),

        Audience::AFileWeWereGiven => Diagnostic::new(
            "E0801",
            format!("`{origin}` is not a Quench module this version can run."),
        )
        .rule("a module is checked before it is believed, rather than being read field by field into some plausible program nobody wrote")
        .tip("a copy that stopped early, or a module built by a different version of Quench, both look like this.")
        .fix("build it again from source")
        .fix("or check it was transferred whole"),
    };

    for one in wrong {
        diag = diag.tip(format!("what was wrong: {one}"));
    }
    diag
}

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
                    Inst::ConstText(at) => {
                        if *at as usize >= module.text.len() {
                            say(format!(
                                "block {i}: names text {at}, and the module holds {}",
                                module.text.len()
                            ));
                        }
                    }
                    Inst::ConstHandle(at) => {
                        if *at as usize >= module.tables.len() {
                            say(format!(
                                "block {i}: names table {at}, and the module holds {}",
                                module.tables.len()
                            ));
                        }
                    }
                    Inst::CallHost { host, args } => {
                        let wants = host.params();
                        if args.len() != wants.len() {
                            say(format!(
                                "block {i}: `{}` takes {} argument(s) and is given {}",
                                host.name(),
                                wants.len(),
                                args.len()
                            ));
                        }
                        for (n, (arg, want)) in args.iter().zip(wants).enumerate() {
                            // One argument is whatever the array holds, and a slot is
                            // the same width whatever that is.
                            if host.takes_an_element() == Some(n) {
                                continue;
                            }
                            if known(*arg, &mut say) && func.ty_of(*arg) != *want {
                                say(format!(
                                    "block {i}: `{}` wants {} for argument {n} and is given {}",
                                    host.name(),
                                    want.name(),
                                    func.ty_of(*arg).name()
                                ));
                            }
                        }
                    }
                    Inst::Bin { op, lhs, rhs } => {
                        let wants = match op {
                            BinOp::And | BinOp::Or => Ty::Bool,
                            _ => Ty::I64,
                        };
                        for v in [lhs, rhs] {
                            if known(*v, &mut say) && func.ty_of(*v) != wants {
                                say(format!(
                                    "block {i}: {op:?} wants {} and {v:?} is {}",
                                    wants.name(),
                                    func.ty_of(*v).name()
                                ));
                            }
                        }
                    }
                    Inst::Cmp { op, lhs, rhs } => {
                        // Two of the same thing, and a thing that fits in a register.
                        // Text and exact numbers are compared by a host call instead,
                        // because what they hold is not what their value is.
                        for v in [lhs, rhs] {
                            if known(*v, &mut say)
                                && !matches!(func.ty_of(*v), Ty::I64 | Ty::Bool)
                            {
                                say(format!(
                                    "block {i}: {op:?} wants i64 or bool and {v:?} is {}",
                                    func.ty_of(*v).name()
                                ));
                            }
                        }
                        if known(*lhs, &mut say)
                            && known(*rhs, &mut say)
                            && func.ty_of(*lhs) != func.ty_of(*rhs)
                        {
                            say(format!(
                                "block {i}: {op:?} compares {} against {}",
                                func.ty_of(*lhs).name(),
                                func.ty_of(*rhs).name()
                            ));
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
